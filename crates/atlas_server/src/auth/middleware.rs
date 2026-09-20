use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;

use crate::{
    auth::tokens::hash_token,
    error::ApiError,
    persistence::repos::{ApiKeyRepo, SessionRepo, UserRepo},
    state::AppState,
};
use atlas_custos_postgres::repos::identity::{PgApiKeyRepo, PgSessionRepo, PgUserRepo};

use atlas_custos::entities::principals::PrincipalKind;

/// The break-glass context of a session-authenticated root user, resolved at
/// authentication time so the root-audit gate never needs a second lookup.
/// Present only for a root user authenticated by session; API-key principals
/// never carry it (a root personal key must not exist at all — W4/W1).
#[derive(Debug, Clone)]
pub(crate) struct RootSession {
    pub(crate) user_id: atlas_core::principal::UserId,
    /// The justification stated at root login. `None` means the session was
    /// tampered with or predates the requirement, and must fail closed.
    pub(crate) reason: Option<String>,
}

/// The resolved authentication principal injected into request extensions.
#[derive(Debug, Clone)]
pub enum Principal {
    User(atlas_core::principal::UserId),
    ApiKey(atlas_core::principal::ApiKeyId),
}

/// Middleware that authenticates every request to the protected router.
///
/// Token resolution order:
/// 1. `Authorization: Bearer <token>` header (wins over cookie)
/// 2. `atlas_session` cookie
///
/// Token dispatch:
/// - `atlas_pk_` prefix → API key path, declaring a personal key: the linked
///   principal must be the owning user's principal (`kind = user`)
/// - `atlas_ak_` prefix → API key path, declaring an agent key: the linked
///   principal must be an agent principal (`kind = agent`)
/// - bare `atlas_` prefix → API key path, untyped (V1 keys carry no kind
///   segment): accepted for every principal kind
/// - No prefix → session path (SHA-256 hex lookup + user.disabled_at IS NULL check)
///
/// On success, inserts `Principal` into request extensions and calls `touch()`.
/// On failure, returns 401 with `WWW-Authenticate: Bearer`.
pub async fn require_authn(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let raw_token = match extract_token(&request, &jar) {
        Some(token) => token,
        None => {
            tracing::debug!("authentication rejected: no bearer token or session cookie");
            return Err(ApiError::Unauthorized);
        }
    };
    let token_hash = hash_token(&raw_token);

    let (principal, root_session) = if raw_token.starts_with("atlas_pk_") {
        let (p, _) = resolve_api_key(&state, &token_hash, Some(PrincipalKind::User)).await?;
        (p, None)
    } else if raw_token.starts_with("atlas_ak_") {
        let (p, _) = resolve_api_key(&state, &token_hash, Some(PrincipalKind::Agent)).await?;
        (p, None)
    } else if raw_token.starts_with("atlas_") {
        // V1 keys have no kind segment: untyped tokens are accepted whatever
        // principal kind they link to. Only a *declared* kind that disagrees
        // with the linked principal is rejected.
        let (p, _) = resolve_api_key(&state, &token_hash, None).await?;
        (p, None)
    } else {
        resolve_session(&state, &token_hash).await?
    };

    match &principal {
        Principal::User(user_id) => tracing::debug!(user_id = ?user_id, "authenticated user"),
        Principal::ApiKey(api_key_id) => {
            tracing::debug!(api_key_id = ?api_key_id, "authenticated api key");
        }
    }

    request.extensions_mut().insert(principal);

    // The one per-action audit site for break-glass sessions: runs before the
    // handler so an unjustified or unrecordable root mutation never executes.
    // The path is taken from `OriginalUri` (inserted by `Router::nest`) so the
    // row records the externally visible path, not the prefix-stripped one.
    if let Some(root) = &root_session {
        let path = request
            .extensions()
            .get::<axum::extract::OriginalUri>()
            .map(|uri| uri.0.path().to_owned())
            .unwrap_or_else(|| request.uri().path().to_owned());
        crate::middleware::root_audit::gate_root_session(
            &state,
            root.user_id,
            root.reason.as_deref(),
            request.method(),
            &path,
        )
        .await?;
    }

    Ok(next.run(request).await)
}

fn extract_token(request: &Request, jar: &CookieJar) -> Option<String> {
    if let Some(auth_header) = request.headers().get(axum::http::header::AUTHORIZATION)
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
    {
        return Some(token.to_owned());
    }

    jar.get("atlas_session").map(|c| c.value().to_owned())
}

async fn resolve_api_key(
    state: &AppState,
    token_hash: &str,
    declared_kind: Option<PrincipalKind>,
) -> Result<(Principal, Option<RootSession>), ApiError> {
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };
    let key = repo
        .find_active_by_token_hash(token_hash)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "api key lookup failed");
            ApiError::Unauthorized
        })?
        .ok_or_else(|| {
            tracing::debug!("authentication rejected: no active api key for token");
            ApiError::Unauthorized
        })?;

    // Only the token's SHA-256 is stored, so the minted prefix is not
    // recoverable at rest — the prefix/kind agreement is enforceable only at
    // authentication time. A declared kind that disagrees with the linked
    // principal can only be a mislinked row, not a forged token: the lookup is
    // by hash of the full token, so altering a prefix merely misses it. Reject
    // the row as the data inconsistency it is.
    if let Some(expected) = declared_kind
        && key.principal_kind != expected
    {
        tracing::debug!(
            api_key_id = ?key.id,
            declared = expected.as_str(),
            linked = key.principal_kind.as_str(),
            "authentication rejected: api key prefix disagrees with its principal kind"
        );
        return Err(ApiError::Unauthorized);
    }

    // A failed touch must not fail authentication; the timestamp is best-effort.
    if let Err(e) = repo.touch(key.id).await {
        tracing::warn!(api_key_id = ?key.id, error = %e, "failed to touch api key last_used");
    }

    Ok((Principal::ApiKey(key.id), None))
}

async fn resolve_session(
    state: &AppState,
    token_hash: &str,
) -> Result<(Principal, Option<RootSession>), ApiError> {
    let session_repo = PgSessionRepo {
        conn: (*state.db).clone(),
    };
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let session = session_repo
        .find_active_by_token_hash(token_hash)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "session lookup failed");
            ApiError::Unauthorized
        })?
        .ok_or_else(|| {
            tracing::debug!("authentication rejected: no active session for token");
            ApiError::Unauthorized
        })?;

    let user = user_repo
        .find_by_id(session.user_id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "user lookup failed");
            ApiError::Unauthorized
        })?
        .ok_or_else(|| {
            tracing::debug!(user_id = ?session.user_id, "authentication rejected: session user not found");
            ApiError::Unauthorized
        })?;

    if user.disabled_at.is_some() {
        tracing::debug!(user_id = ?session.user_id, "authentication rejected: user is disabled");
        return Err(ApiError::Unauthorized);
    }

    // A failed touch must not fail authentication; the sliding expiry is best-effort.
    if let Err(e) = session_repo
        .touch(
            session.id,
            state.session_ttl_hours,
            state.session_max_ttl_hours,
        )
        .await
    {
        tracing::warn!(user_id = ?session.user_id, error = %e, "failed to touch session expiry");
    }

    let root_session = if user.is_root {
        Some(RootSession {
            user_id: session.user_id,
            reason: session.root_reason,
        })
    } else {
        None
    };

    Ok((Principal::User(session.user_id), root_session))
}
