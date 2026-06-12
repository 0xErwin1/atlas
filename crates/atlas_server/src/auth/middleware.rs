use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;

use crate::{
    auth::tokens::hash_token,
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, PgApiKeyRepo, PgSessionRepo, PgUserRepo, SessionRepo, UserRepo,
    },
    state::AppState,
};

/// The resolved authentication principal injected into request extensions.
#[derive(Debug, Clone)]
pub enum Principal {
    User(atlas_domain::ids::UserId),
    ApiKey(atlas_domain::ids::ApiKeyId),
}

/// Middleware that authenticates every request to the protected router.
///
/// Token resolution order:
/// 1. `Authorization: Bearer <token>` header (wins over cookie)
/// 2. `atlas_session` cookie
///
/// Token dispatch:
/// - `atlas_` prefix → API key path (SHA-256 hex lookup in api_keys.token_hash)
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
    let raw_token = extract_token(&request, &jar).ok_or(ApiError::Unauthorized)?;
    let token_hash = hash_token(&raw_token);

    let principal = if raw_token.starts_with("atlas_") {
        resolve_api_key(&state, &token_hash).await?
    } else {
        resolve_session(&state, &token_hash).await?
    };

    request.extensions_mut().insert(principal);
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

async fn resolve_api_key(state: &AppState, token_hash: &str) -> Result<Principal, ApiError> {
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };
    let key = repo
        .find_active_by_token_hash(token_hash)
        .await
        .map_err(|_| ApiError::Unauthorized)?
        .ok_or(ApiError::Unauthorized)?;

    Ok(Principal::ApiKey(key.id))
}

async fn resolve_session(state: &AppState, token_hash: &str) -> Result<Principal, ApiError> {
    let session_repo = PgSessionRepo {
        conn: (*state.db).clone(),
    };
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let session = session_repo
        .find_active_by_token_hash(token_hash)
        .await
        .map_err(|_| ApiError::Unauthorized)?
        .ok_or(ApiError::Unauthorized)?;

    let user = user_repo
        .find_by_id(session.user_id)
        .await
        .map_err(|_| ApiError::Unauthorized)?
        .ok_or(ApiError::Unauthorized)?;

    if user.disabled_at.is_some() {
        return Err(ApiError::Unauthorized);
    }

    session_repo
        .touch(
            session.id,
            state.session_ttl_hours,
            state.session_max_ttl_hours,
        )
        .await
        .ok();

    Ok(Principal::User(session.user_id))
}
