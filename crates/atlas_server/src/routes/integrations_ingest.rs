#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::collections::HashMap;

use axum::{
    extract::{FromRequest, FromRequestParts, Path, Request, State},
    http::StatusCode,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::{
    error::ApiError,
    persistence::repos::{PgIntegrationConfigRepo, PgWorkspaceRepo, WorkspaceRepo},
    services::AutomationService,
    state::AppState,
};

type HmacSha256 = Hmac<Sha256>;

/// Maximum allowed incoming body size in bytes (1 MiB).
const BODY_LIMIT: usize = 1024 * 1024;

/// Proof that the request passed HMAC-SHA256 verification against the per-integration
/// secret stored in `integration_configs`.
///
/// Produced by reading and verifying the raw request body, so the handler receives both
/// the verified delivery metadata and the already-parsed JSON payload. Intended for the
/// public router; must not be placed behind `require_authn`.
pub(crate) struct VerifiedIntegrationEvent {
    pub workspace_id: Uuid,
    /// Integration slug (e.g. `"github"`). Available for future multi-integration routing.
    #[allow(dead_code)]
    pub integration: String,
    pub integration_api_key_id: Uuid,
    pub delivery_id: Uuid,
    pub event_name: String,
    pub data: serde_json::Value,
}

impl FromRequest<AppState> for VerifiedIntegrationEvent {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, ApiError> {
        let (mut parts, body) = req.into_parts();

        let Path(params): Path<HashMap<String, String>> =
            Path::from_request_parts(&mut parts, state)
                .await
                .map_err(|_| ApiError::NotFound)?;

        let ws_slug = params.get("ws").ok_or(ApiError::NotFound)?.clone();
        let integration = params.get("integration").ok_or(ApiError::NotFound)?.clone();

        let sig_header = parts
            .headers
            .get("x-hub-signature-256")
            .ok_or(ApiError::Unauthorized)?
            .to_str()
            .map_err(|_| ApiError::Unauthorized)?
            .to_string();

        let delivery_id = {
            let raw = parts
                .headers
                .get("x-github-delivery")
                .ok_or(ApiError::BadRequest {
                    message: "X-GitHub-Delivery header is required".into(),
                })?
                .to_str()
                .map_err(|_| ApiError::BadRequest {
                    message: "X-GitHub-Delivery header contains invalid UTF-8".into(),
                })?;

            Uuid::parse_str(raw).map_err(|_| ApiError::BadRequest {
                message: "X-GitHub-Delivery must be a valid UUID".into(),
            })?
        };

        let event_name = parts
            .headers
            .get("x-github-event")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let bytes = axum::body::to_bytes(body, BODY_LIMIT).await.map_err(|_| {
            ApiError::PayloadTooLarge {
                message: "request body exceeds 1 MiB limit".into(),
            }
        })?;

        let ws_repo = PgWorkspaceRepo {
            conn: (*state.db).clone(),
        };
        let workspace = ws_repo
            .find_by_slug(&ws_slug)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let config = PgIntegrationConfigRepo::find_active(&*state.db, workspace.id.0, &integration)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let secret_bytes = state
            .webhook_crypto
            .decrypt(&config.encrypted_secret, &config.secret_nonce)
            .map_err(|e| ApiError::Internal { message: e })?;

        verify_github_signature(&sig_header, &secret_bytes, &bytes)?;

        let data: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::BadRequest {
                message: "request body is not valid JSON".into(),
            })?;

        Ok(VerifiedIntegrationEvent {
            workspace_id: workspace.id.0,
            integration,
            integration_api_key_id: config.integration_api_key_id,
            delivery_id,
            event_name,
            data,
        })
    }
}

/// Verifies the `X-Hub-Signature-256: sha256=<hex>` header against the raw body bytes
/// using constant-time HMAC-SHA256 comparison.
///
/// Returns `ApiError::Unauthorized` on any mismatch, malformed prefix, or invalid hex.
/// Returns `ApiError::Internal` only if the HMAC key itself is unusable (should not happen
/// in practice after successful decryption).
pub(crate) fn verify_github_signature(
    sig_header: &str,
    secret: &[u8],
    body: &[u8],
) -> Result<(), ApiError> {
    let hex_str = sig_header
        .strip_prefix("sha256=")
        .ok_or(ApiError::Unauthorized)?;

    let sig_bytes = decode_hex(hex_str).map_err(|_| ApiError::Unauthorized)?;

    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| ApiError::Internal {
        message: format!("HMAC key error: {e}"),
    })?;
    mac.update(body);

    mac.verify_slice(&sig_bytes)
        .map_err(|_| ApiError::Unauthorized)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/integrations/{integration}/events
// ---------------------------------------------------------------------------

/// Receives a signed GitHub delivery, verifies it via the `VerifiedIntegrationEvent`
/// extractor, and hands it off to `AutomationService` for dedup, outbox storage,
/// and rule evaluation.
///
/// Returns 200 regardless of whether the delivery was new or a duplicate so
/// that GitHub does not retry on idempotent re-deliveries.
#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/integrations/{integration}/events",
    tag = "integrations",
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("integration" = String, Path, description = "Integration slug (e.g. 'github')"),
    ),
    responses(
        (status = 200, description = "Event accepted (new or duplicate)"),
        (status = 401, description = "Signature missing or invalid"),
        (status = 404, description = "No active integration config found for this workspace"),
        (status = 413, description = "Request body exceeds 1 MiB"),
    )
)]
pub(crate) async fn ingest_github_event(
    State(state): State<AppState>,
    event: VerifiedIntegrationEvent,
) -> Result<StatusCode, ApiError> {
    let svc = AutomationService::new((*state.db).clone());

    svc.process_github_delivery(
        event.workspace_id,
        event.integration_api_key_id,
        event.delivery_id,
        &event.event_name,
        &event.data,
    )
    .await
    .map_err(ApiError::Domain)?;

    Ok(StatusCode::OK)
}

/// Decodes a lowercase or uppercase hexadecimal string into raw bytes.
///
/// Returns `Err(())` for odd-length input or characters outside `[0-9a-fA-F]`.
fn decode_hex(s: &str) -> Result<Vec<u8>, ()> {
    if !s.len().is_multiple_of(2) {
        return Err(());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2).ok_or(())?, 16).map_err(|_| ()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    fn compute_sig(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let bytes = mac.finalize().into_bytes();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256={hex}")
    }

    // B3.2 [U] Valid HMAC signature is accepted
    #[test]
    fn valid_signature_accepted() {
        let secret = b"test-secret";
        let body = b"hello world payload";
        let sig = compute_sig(secret, body);

        let result = verify_github_signature(&sig, secret, body);
        assert!(
            result.is_ok(),
            "valid signature must be accepted: {result:?}"
        );
    }

    // B3.2 [U] Wrong secret produces a different signature, which is rejected with 401
    #[test]
    fn wrong_secret_rejected_with_401() {
        let secret = b"correct-secret";
        let wrong_secret = b"wrong-secret";
        let body = b"hello world";
        let sig = compute_sig(secret, body);

        let err = verify_github_signature(&sig, wrong_secret, body).unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized),
            "signature mismatch must return Unauthorized, got {err:?}"
        );
    }

    // B3.2 [U] All-zero signature (wrong value) is rejected with 401
    #[test]
    fn all_zero_signature_rejected_with_401() {
        let secret = b"my-secret";
        let body = b"some payload";
        let bad_sig = "sha256=0000000000000000000000000000000000000000000000000000000000000000";

        let err = verify_github_signature(bad_sig, secret, body).unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized),
            "all-zero sig must return Unauthorized, got {err:?}"
        );
    }

    // B3.2 [U] Missing sha256= prefix returns 401 (not 500)
    #[test]
    fn missing_prefix_rejected_with_401() {
        let secret = b"secret";
        let body = b"body";
        let no_prefix = "deadbeef";

        let err = verify_github_signature(no_prefix, secret, body).unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized),
            "missing sha256= prefix must return Unauthorized, got {err:?}"
        );
    }

    // B3.2 [U] Invalid hex in the signature returns 401
    #[test]
    fn invalid_hex_signature_rejected_with_401() {
        let secret = b"secret";
        let body = b"body";
        let invalid_hex = "sha256=zzzz";

        let err = verify_github_signature(invalid_hex, secret, body).unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized),
            "invalid hex must return Unauthorized, got {err:?}"
        );
    }

    // B3.2 [U] Odd-length hex string is rejected
    #[test]
    fn odd_length_hex_rejected() {
        let err = verify_github_signature("sha256=abc", b"secret", b"body").unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized),
            "odd-length hex must return Unauthorized, got {err:?}"
        );
    }

    // B3.2 [U] Single-bit tamper of signature → Unauthorized (exercises constant-time path)
    #[test]
    fn single_bit_tamper_rejected() {
        let secret = b"s3cr3t";
        let body = b"important content";
        let mut sig = compute_sig(secret, body);
        // Flip the last hex nibble: '0'→'1', anything else→'0'
        let last = sig.pop().unwrap();
        sig.push(if last == '0' { '1' } else { '0' });

        let err = verify_github_signature(&sig, secret, body).unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized),
            "single-bit tamper must return Unauthorized, got {err:?}"
        );
    }

    // B3.2 [U] Body size limit constant must be 1 MiB
    #[test]
    fn body_limit_is_one_mib() {
        assert_eq!(BODY_LIMIT, 1024 * 1024);
    }

    // decode_hex helpers
    #[test]
    fn decode_hex_known_value() {
        let bytes = decode_hex("deadbeef").unwrap();
        assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn decode_hex_rejects_odd_length() {
        assert!(decode_hex("abc").is_err());
    }

    #[test]
    fn decode_hex_rejects_non_hex_chars() {
        assert!(decode_hex("zz").is_err());
    }
}
