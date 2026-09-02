use atlas_api::{dtos::documents::ConflictProblemDto, problem::ProblemDetails};
use atlas_core::error::DomainError;
use atlas_core::error::RevisionConflict;
use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

/// Component-conflict codes for Acta resources, relocated from `atlas_domain`
/// (S2e). Kept beside `ApiError` since it is the sole consumer of these codes.
pub mod acta_conflict {
    /// Fractional position space in a column is exhausted: no midpoint can be
    /// computed between the two anchors. The adapter must rebalance the
    /// column's keys and retry, or surface a 409 to the caller.
    pub const POSITION_EXHAUSTED: &str = "position-exhausted";
}

/// Server-side error taxonomy.
///
/// Every variant maps to a specific RFC 9457 problem type and HTTP status.
/// Handlers build `ApiError` values; the `IntoResponse` impl serializes them.
/// The problem-stamp middleware fills `request_id` and `instance` after the fact.
#[derive(Debug)]
pub enum ApiError {
    Domain(DomainError),
    Unauthorized,
    CsrfRequired,
    InvalidInput {
        message: String,
    },
    NotFound,
    Forbidden {
        message: String,
    },
    /// Malformed query parameter — the value is syntactically invalid and
    /// cannot be coerced.  Returns 400 Bad Request, not 422 (which is reserved
    /// for semantically invalid but parseable inputs).
    BadRequest {
        message: String,
    },
    /// Generic conflict (no payload). Prefer `RevisionConflict` for CAS failures.
    Conflict,
    /// CAS revision conflict with full patch payload for the 409 response body.
    RevisionConflict(RevisionConflict),
    /// The target is archived, so it accepts reads but refuses this write.
    ///
    /// Distinct from `Forbidden`: the caller's permissions are fine, the
    /// resource's own state is what refuses.
    Archived {
        message: String,
    },
    /// The requested operation would remove the last owner from a workspace.
    ///
    /// A workspace must keep at least one owner at all times. This check applies
    /// to all callers including break-glass (root/system-admin) — it is a
    /// data-integrity invariant, not a permission.
    LastOwner {
        message: String,
    },
    PayloadTooLarge {
        message: String,
    },
    ServiceUnavailable {
        message: String,
    },
    /// The authenticated principal exceeded its request quota. Carries the number
    /// of whole seconds the caller should wait, surfaced via `Retry-After`.
    TooManyRequests {
        retry_after_secs: u64,
    },
    /// An `Idempotency-Key` was reused by the same principal against a
    /// different request (a different method, path, or body) than the one
    /// that first claimed it (`v2-e3-s3` D1). Distinct from
    /// `IdempotencyKeyInFlight`: this is a *completed* row with a mismatched
    /// fingerprint, never reusing `revision-conflict`'s problem `type` (a
    /// client-side CAS-merge UI pattern-matches on that exact string).
    IdempotencyKeyConflict {
        existing_fingerprint_hint: String,
    },
    /// A second request presented the same `Idempotency-Key` while the first
    /// request for it is still executing (`v2-e3-s3` D2). Distinct from
    /// `IdempotencyKeyConflict`: same 409 status, different problem `type`,
    /// so a client can tell "retry later" from "you sent a different
    /// request under this key."
    IdempotencyKeyInFlight,
    Internal {
        message: String,
    },
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, problem) = match self {
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                ProblemDetails::new(
                    "urn:atlas:error:unauthorized",
                    "Unauthorized",
                    401,
                )
                .with_hint("Provide a valid Bearer token or session cookie. Login at POST /api/auth/login."),
            ),
            ApiError::CsrfRequired => (
                StatusCode::FORBIDDEN,
                ProblemDetails::new(
                    "urn:atlas:error:csrf-required",
                    "CSRF Protection Required",
                    403,
                )
                .with_hint("Include the 'X-Atlas-CSRF: 1' header on cookie-authenticated state-changing requests."),
            ),
            ApiError::InvalidInput { message } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemDetails::new("urn:atlas:error:invalid-input", "Invalid Input", 422)
                    .with_detail(message),
            ),
            ApiError::BadRequest { message } => (
                StatusCode::BAD_REQUEST,
                ProblemDetails::new("urn:atlas:error:bad-request", "Bad Request", 400)
                    .with_detail(message),
            ),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                ProblemDetails::new("urn:atlas:error:not-found", "Not Found", 404).with_hint(
                    "Check the identifier — it may not exist or you may not have access.",
                ),
            ),
            ApiError::Forbidden { message } => (
                StatusCode::FORBIDDEN,
                ProblemDetails::new("urn:atlas:error:forbidden", "Forbidden", 403)
                    .with_detail(message),
            ),
            ApiError::Conflict => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "urn:atlas:error:revision-conflict",
                    "Revision Conflict",
                    409,
                ),
            ),
            ApiError::PayloadTooLarge { message } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                ProblemDetails::new(
                    "urn:atlas:error:payload-too-large",
                    "Payload Too Large",
                    413,
                )
                .with_detail(message),
            ),
            ApiError::ServiceUnavailable { message } => (
                StatusCode::SERVICE_UNAVAILABLE,
                ProblemDetails::new(
                    "urn:atlas:error:service-unavailable",
                    "Service Unavailable",
                    503,
                )
                .with_detail(message),
            ),
            ApiError::RevisionConflict(c) => {
                let body = ConflictProblemDto::new(
                    c.current_revision_id,
                    c.current_seq,
                    c.base_to_current_patch,
                );
                return render_problem(StatusCode::CONFLICT, body);
            }
            ApiError::TooManyRequests { retry_after_secs } => {
                let problem = ProblemDetails::new(
                    "urn:atlas:error:rate-limited",
                    "Too Many Requests",
                    429,
                )
                .with_hint(
                    "You are sending requests too quickly. Wait for the Retry-After interval before retrying.",
                );

                let mut response = render_problem(StatusCode::TOO_MANY_REQUESTS, problem);
                if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }
                return response;
            }
            ApiError::IdempotencyKeyConflict {
                existing_fingerprint_hint,
            } => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "urn:atlas:error:idempotency-key-conflict",
                    "Idempotency Key Conflict",
                    409,
                )
                .with_detail(format!(
                    "This Idempotency-Key was already used for a different request \
                     (existing fingerprint: {existing_fingerprint_hint})."
                ))
                .with_hint(
                    "Use a new Idempotency-Key for a different request, or resend the exact \
                     same request to replay its stored response.",
                ),
            ),
            ApiError::IdempotencyKeyInFlight => {
                let problem = ProblemDetails::new(
                    "urn:atlas:error:idempotency-key-in-flight",
                    "Idempotency Key In Flight",
                    409,
                )
                .with_hint(
                    "A request with this Idempotency-Key is still executing. Wait for it to \
                     complete before retrying.",
                );

                let mut response = render_problem(StatusCode::CONFLICT, problem);
                if let Ok(value) = HeaderValue::from_str("1") {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }
                return response;
            }
            ApiError::Archived { message } => (
                StatusCode::CONFLICT,
                ProblemDetails::new("urn:atlas:error:archived", "Archived", 409)
                    .with_detail(message)
                    .with_hint("Unarchive the board to make changes to it again."),
            ),
            ApiError::LastOwner { message } => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "urn:atlas:error:last-owner",
                    "Last Owner",
                    409,
                )
                .with_detail(message)
                .with_hint(
                    "Promote another member to owner before demoting or removing the last owner.",
                ),
            ),
            ApiError::Internal { message } => {
                tracing::error!(error = %message, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ProblemDetails::new("urn:atlas:error:internal", "Internal Server Error", 500)
                        .with_detail("An internal error occurred."),
                )
            }
            ApiError::Domain(domain_err) => return domain_error_response(domain_err),
        };

        render_problem(status, problem)
    }
}

fn domain_error_response(err: DomainError) -> Response {
    let (status, problem) = match err {
        DomainError::NotFound { entity, id } => (
            StatusCode::NOT_FOUND,
            ProblemDetails::new("urn:atlas:error:not-found", "Not Found", 404)
                .with_hint("Check the identifier — it may not exist or you may not have access.")
                .with_detail(format!("{entity} {id} not found")),
        ),
        DomainError::Conflict(c) => {
            let body = ConflictProblemDto::new(
                c.current_revision_id,
                c.current_seq,
                c.base_to_current_patch,
            );
            return render_problem(StatusCode::CONFLICT, body);
        }
        DomainError::InvalidInput { message } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemDetails::new("urn:atlas:error:invalid-input", "Invalid Input", 422)
                .with_detail(message),
        ),
        DomainError::Forbidden { message } => (
            StatusCode::FORBIDDEN,
            ProblemDetails::new("urn:atlas:error:forbidden", "Forbidden", 403).with_detail(message),
        ),
        DomainError::AlreadyExists { message } => (
            StatusCode::CONFLICT,
            ProblemDetails::new("urn:atlas:error:already-exists", "Already Exists", 409)
                .with_hint("An item with this name already exists here — choose a different name.")
                .with_detail(message),
        ),
        DomainError::RestoreParentDeleted { kind } => (
            StatusCode::CONFLICT,
            ProblemDetails::new(
                "urn:atlas:error:restore-parent-deleted",
                "Restore Blocked",
                409,
            )
            .with_hint("Restore the deleted parent before restoring this item.")
            .with_detail(format!(
                "restore is blocked because the {kind}'s parent is deleted"
            )),
        ),
        DomainError::RestoreIdentityConflict { kind } => (
            StatusCode::CONFLICT,
            ProblemDetails::new(
                "urn:atlas:error:restore-identity-conflict",
                "Restore Blocked",
                409,
            )
            .with_hint("Resolve the live conflicting identity before restoring this item.")
            .with_detail(format!(
                "restore is blocked because a live {kind} has the same identity"
            )),
        ),
        DomainError::CommentDraftConflict { reason } => (
            StatusCode::CONFLICT,
            ProblemDetails::new(
                "urn:atlas:error:comment-draft-conflict",
                "Comment Draft Conflict",
                409,
            )
            .with_detail(reason),
        ),
        DomainError::CommentDraftGone { reason } => (
            StatusCode::GONE,
            ProblemDetails::new(
                "urn:atlas:error:comment-draft-gone",
                "Comment Draft Gone",
                410,
            )
            .with_detail(reason),
        ),
        DomainError::Internal { message } => {
            tracing::error!(error = %message, "domain internal error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ProblemDetails::new("urn:atlas:error:internal", "Internal Server Error", 500)
                    .with_detail("An internal error occurred."),
            )
        }
        DomainError::ComponentConflict { code, message } => match code {
            acta_conflict::POSITION_EXHAUSTED => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "urn:atlas:error:position-exhausted",
                    "Position Exhausted",
                    409,
                )
                .with_hint("Retry the move; the server attempted to rebalance column positions."),
            ),
            unknown => {
                tracing::error!(code = %unknown, "unmapped component conflict code");
                (
                    StatusCode::CONFLICT,
                    ProblemDetails::new("urn:atlas:error:conflict", "Conflict", 409)
                        .with_detail(message.unwrap_or_else(|| "conflict".into())),
                )
            }
        },
    };

    render_problem(status, problem)
}

/// Serializes `body` and wraps it in a `StatusCode` response carrying
/// `content-type: application/problem+json` — the single site every
/// RFC 9457 error variant renders through, including variants (like
/// `ConflictProblemDto`) that extend `ProblemDetails` with extra fields
/// rather than using it directly (D-9457).
fn render_problem<T: Serialize>(status: StatusCode, body: T) -> Response {
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = (status, bytes).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}

/// T1.1/T1.5/T1.6/T1.7 (`v2-e3-s3` PR1, D-9457): golden-body regression
/// coverage for the RFC 9457 fold.
///
/// `RevisionConflict` and `TooManyRequests` are the only two `ApiError`
/// variants that hand-build their JSON body instead of routing through the
/// shared `ProblemDetails` render path (see the module doc). The fold in
/// this PR must not change one byte of either variant's wire body: the
/// fixtures below were captured from this file's pre-fold rendering (a
/// `serde_json::to_vec` + manual `content-type` header per variant) and are
/// asserted byte-identical after the fold lands.
#[cfg(test)]
mod golden_body_tests {
    use super::*;
    use atlas_core::error::RevisionConflict;

    fn sample_revision_conflict() -> ApiError {
        ApiError::RevisionConflict(RevisionConflict {
            resource_id: uuid::Uuid::nil(),
            current_revision_id: uuid::Uuid::parse_str("0195f7b4-1234-7abc-8def-0123456789ab")
                .expect("valid uuid literal"),
            current_seq: 42,
            base_to_current_patch: "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        })
    }

    fn sample_too_many_requests() -> ApiError {
        ApiError::TooManyRequests {
            retry_after_secs: 17,
        }
    }

    /// Captured verbatim (T1.1) from the pre-fold hand-built render path at
    /// `error.rs:144-157`, before any fold code touched this file.
    const REVISION_CONFLICT_GOLDEN: &str = r#"{"type":"urn:atlas:error:revision-conflict","title":"Revision Conflict","status":409,"detail":"The base_revision_id does not match the current revision. Apply base_to_current_patch and retry.","hint":"Apply the provided patch to your local content, then retry with the new revision id.","current_revision_id":"0195f7b4-1234-7abc-8def-0123456789ab","current_seq":42,"base_to_current_patch":"--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n"}"#;

    /// Captured verbatim (T1.1) from the pre-fold hand-built render path at
    /// `error.rs:158-178`, before any fold code touched this file.
    const TOO_MANY_REQUESTS_GOLDEN: &str = r#"{"type":"urn:atlas:error:rate-limited","title":"Too Many Requests","status":429,"hint":"You are sending requests too quickly. Wait for the Retry-After interval before retrying."}"#;

    async fn render_body(err: ApiError) -> (StatusCode, Option<String>, Option<String>, String) {
        let response = err.into_response();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("response body must be readable");
        let body = String::from_utf8(bytes.to_vec()).expect("response body must be valid utf-8");
        (status, content_type, retry_after, body)
    }

    #[tokio::test]
    async fn revision_conflict_body_is_byte_identical_to_golden() {
        let (status, content_type, _retry_after, body) =
            render_body(sample_revision_conflict()).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(content_type.as_deref(), Some("application/problem+json"));
        assert_eq!(
            body, REVISION_CONFLICT_GOLDEN,
            "RevisionConflict's rendered body drifted from the pre-fold golden fixture"
        );
    }

    #[tokio::test]
    async fn too_many_requests_body_and_retry_after_are_byte_identical_to_golden() {
        let (status, content_type, retry_after, body) =
            render_body(sample_too_many_requests()).await;

        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(content_type.as_deref(), Some("application/problem+json"));
        assert_eq!(retry_after.as_deref(), Some("17"));
        assert_eq!(
            body, TOO_MANY_REQUESTS_GOLDEN,
            "TooManyRequests's rendered body drifted from the pre-fold golden fixture"
        );
    }

    /// T1.1's own regression proof: the golden comparison must be capable of
    /// failing. Moving/renaming a field in the fixture's copy (never in the
    /// handler) must turn the comparison RED — proving the assertion is not
    /// passing vacuously by construction.
    #[tokio::test]
    async fn golden_comparison_fails_when_a_field_is_renamed() {
        let (_status, _content_type, _retry_after, body) =
            render_body(sample_revision_conflict()).await;

        let mutated_golden = REVISION_CONFLICT_GOLDEN.replacen(
            "\"current_revision_id\"",
            "\"current_revision_id_renamed\"",
            1,
        );
        assert_ne!(
            mutated_golden, REVISION_CONFLICT_GOLDEN,
            "the mutation helper must actually change the fixture copy"
        );
        assert_ne!(
            body, mutated_golden,
            "a renamed field in the fixture must make the byte-identity check fail; \
             it did not, so this test does not actually prove anything"
        );
    }

    /// T1.6/T1.7: every `ApiError` variant OTHER than `RevisionConflict` and
    /// `TooManyRequests` must render byte-unchanged after the fold. These
    /// already went through `ProblemDetails`/`render_problem` before
    /// this PR, so the fold (T1.2-T1.4) must not touch their construction at
    /// all — this test pins their bodies too, independent of the two folded
    /// variants above.
    #[tokio::test]
    async fn other_variants_render_unchanged_by_the_fold() {
        let (status, content_type, _retry_after, body) = render_body(ApiError::Unauthorized).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(content_type.as_deref(), Some("application/problem+json"));
        assert_eq!(
            body,
            r#"{"type":"urn:atlas:error:unauthorized","title":"Unauthorized","status":401,"hint":"Provide a valid Bearer token or session cookie. Login at POST /api/auth/login."}"#
        );

        let (status, content_type, _retry_after, body) = render_body(ApiError::Conflict).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(content_type.as_deref(), Some("application/problem+json"));
        assert_eq!(
            body,
            r#"{"type":"urn:atlas:error:revision-conflict","title":"Revision Conflict","status":409}"#
        );

        let (status, content_type, _retry_after, body) = render_body(ApiError::NotFound).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(content_type.as_deref(), Some("application/problem+json"));
        assert_eq!(
            body,
            r#"{"type":"urn:atlas:error:not-found","title":"Not Found","status":404,"hint":"Check the identifier — it may not exist or you may not have access."}"#
        );
    }

    /// T2.X (`v2-e3-s3` PR2): `DomainError::Conflict` (`error.rs:210-223`)
    /// hand-builds `serde_json::to_vec` bytes exactly like the two
    /// `ApiError` variants PR1 folded, and was left out of that PR only
    /// because it is a different enum (PR1's own verify report flagged it
    /// as the same drift class). Captured verbatim, pre-fold, the same way
    /// T1.1 captured `RevisionConflict`'s body — `DomainError::Conflict`
    /// wraps the same `atlas_core::error::RevisionConflict` payload, built
    /// through the same `ConflictProblemDto::new`, so its body is
    /// byte-identical to `REVISION_CONFLICT_GOLDEN` above.
    fn sample_domain_conflict() -> DomainError {
        DomainError::Conflict(RevisionConflict {
            resource_id: uuid::Uuid::nil(),
            current_revision_id: uuid::Uuid::parse_str("0195f7b4-1234-7abc-8def-0123456789ab")
                .expect("valid uuid literal"),
            current_seq: 42,
            base_to_current_patch: "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        })
    }

    async fn render_domain_body(err: DomainError) -> (StatusCode, Option<String>, String) {
        let response = domain_error_response(err);
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("response body must be readable");
        let body = String::from_utf8(bytes.to_vec()).expect("response body must be valid utf-8");
        (status, content_type, body)
    }

    #[tokio::test]
    async fn domain_conflict_body_is_byte_identical_to_golden() {
        let (status, content_type, body) = render_domain_body(sample_domain_conflict()).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(content_type.as_deref(), Some("application/problem+json"));
        assert_eq!(
            body, REVISION_CONFLICT_GOLDEN,
            "DomainError::Conflict's rendered body drifted from the pre-fold golden fixture"
        );
    }

    /// T2.X's own RED proof, mirroring T1.1's: the golden comparison must be
    /// capable of failing, not passing vacuously.
    #[tokio::test]
    async fn domain_conflict_golden_comparison_fails_when_a_field_is_renamed() {
        let (_status, _content_type, body) = render_domain_body(sample_domain_conflict()).await;

        let mutated_golden = REVISION_CONFLICT_GOLDEN.replacen(
            "\"current_revision_id\"",
            "\"current_revision_id_renamed\"",
            1,
        );
        assert_ne!(
            mutated_golden, REVISION_CONFLICT_GOLDEN,
            "the mutation helper must actually change the fixture copy"
        );
        assert_ne!(
            body, mutated_golden,
            "a renamed field in the fixture must make the byte-identity check fail; \
             it did not, so this test does not actually prove anything"
        );
    }

    /// The dual of `domain_conflict_golden_comparison_fails_when_a_field_is_
    /// renamed`: that test mutates a COPY of the golden fixture, proving the
    /// comparison mechanism itself isn't vacuous, but never touches the
    /// production render path — it would still pass unchanged even if
    /// `domain_error_response` silently dropped `current_seq` from its
    /// output. This test instead mutates the DOMAIN INPUT and renders it
    /// through the real, unmodified `domain_error_response`, proving the
    /// production path actually carries `current_seq` through to the wire
    /// rather than rendering a constant body regardless of input.
    #[tokio::test]
    async fn domain_conflict_golden_comparison_fails_when_current_seq_is_off_by_one() {
        let mut mutated_conflict = sample_domain_conflict();
        let DomainError::Conflict(c) = &mut mutated_conflict else {
            unreachable!("sample_domain_conflict always returns DomainError::Conflict");
        };
        c.current_seq += 1;

        let (_status, _content_type, body) = render_domain_body(mutated_conflict).await;

        assert_ne!(
            body, REVISION_CONFLICT_GOLDEN,
            "rendering DomainError::Conflict with current_seq off by one from the \
             golden fixture must produce a different body; it did not, so \
             `domain_error_response` is not actually rendering current_seq"
        );
    }
}
