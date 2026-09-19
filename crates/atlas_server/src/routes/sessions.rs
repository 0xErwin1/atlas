//! Self-service session management for the authenticated human user
//! (`v2-e4-s3a-sessions`): list your own sessions, revoke one, revoke all
//! except the current one. No migration: `custos.sessions` already carries
//! every column these routes need (`id`, `user_id`, `token_hash`,
//! `expires_at`, `last_used_at`, `revoked_at`, `created_at`) — there is no
//! IP or user-agent column, so no "device" is projected.
//!
//! ## Policy decisions baked into these handlers
//!
//! - **`User` principals only.** A session belongs to a human login; an
//!   API-key principal acting on its owner's sessions would need its own
//!   policy justification, so all three routes answer 403 with an actionable
//!   hint instead of guessing.
//! - **Non-disclosure.** Revoking a session id that is not the caller's
//!   answers 404, not 403 — an id the caller cannot see does not exist for
//!   them. Revoking an already-revoked session is an idempotent no-op.
//! - **`DELETE /sessions` keeps the caller's current session alive.**
//!   Revoking the current session itself is `POST /auth/logout`; it is not
//!   duplicated here.
//! - The `token_hash` never appears in any projection, log line, or
//!   response body (`SessionDto` has no such field).

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use axum_extra::extract::CookieJar;
use chrono::{DateTime, Utc};

use atlas_api::dtos::SessionDto;
use atlas_custos::entities::identity::Session;
use atlas_custos::ids::SessionId;
use atlas_custos_postgres::repos::identity::{PgSessionRepo, SessionRepo};

use crate::{
    auth::{middleware::Principal, tokens::hash_token},
    error::ApiError,
    state::AppState,
};

/// Extracts the authenticated human user's id, or refuses an API-key
/// principal with an actionable hint. Sessions belong to human logins; a key
/// acting on its owner's sessions is a policy decision that has not been
/// made, so refusing is the safe default.
fn user_id_or_forbidden(principal: &Principal) -> Result<atlas_core::principal::UserId, ApiError> {
    match principal {
        Principal::User(user_id) => Ok(*user_id),
        Principal::ApiKey(_) => Err(ApiError::Forbidden {
            message: "Sessions belong to a human login; API keys cannot list or revoke \
                      sessions. Authenticate as the user instead."
                .into(),
        }),
    }
}

fn session_repo(state: &AppState) -> PgSessionRepo {
    PgSessionRepo {
        conn: (*state.db).clone(),
    }
}

/// Projects a session row for the wire: never the `token_hash`, and the
/// active flag computed from `revoked_at IS NULL AND expires_at > now()`.
fn session_dto(session: &Session, now: DateTime<Utc>) -> SessionDto {
    SessionDto {
        id: session.id.0,
        created_at: session.created_at,
        last_used_at: session.last_used_at,
        expires_at: session.expires_at,
        revoked_at: session.revoked_at,
        active: session.revoked_at.is_none() && session.expires_at > now,
    }
}

#[utoipa::path(
    get,
    path = "/sessions",
    tag = "auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The caller's own sessions, newest first", body = Vec<SessionDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot list user sessions"),
    )
)]
pub(crate) async fn list_sessions(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<SessionDto>>, ApiError> {
    let user_id = user_id_or_forbidden(&principal)?;

    let sessions = session_repo(&state)
        .list_for_user(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let now = Utc::now();
    Ok(Json(sessions.iter().map(|s| session_dto(s, now)).collect()))
}

/// Resolves the caller's current session row from the presented cookie or
/// bearer token — the same cookie-first resolution order `logout` and
/// `change_password` use, so the middleware that authenticated the request
/// and this resolver can never disagree about which row is "current".
async fn current_session(
    state: &AppState,
    user_id: atlas_core::principal::UserId,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Result<Session, ApiError> {
    let bearer_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_owned());

    let raw_token = jar
        .get("atlas_session")
        .map(|c| c.value().to_owned())
        .or(bearer_token);

    let Some(raw) = raw_token else {
        return Err(ApiError::Unauthorized);
    };

    let hash = hash_token(&raw);
    let session = session_repo(state)
        .find_active_by_token_hash(&hash)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::Unauthorized)?;

    if session.user_id != user_id {
        return Err(ApiError::Unauthorized);
    }
    Ok(session)
}

#[utoipa::path(
    delete,
    path = "/sessions/{session_id}",
    tag = "auth",
    security(("bearer_auth" = [])),
    params(("session_id" = uuid::Uuid, Path, description = "Session ID")),
    responses(
        (status = 204, description = "Session revoked (a repeat revoke is the same 204 no-op)"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot revoke user sessions"),
        (status = 404, description = "No session with this id belongs to the caller — non-disclosure, not a permission error"),
    )
)]
pub(crate) async fn revoke_session(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(session_id): Path<uuid::Uuid>,
) -> Result<StatusCode, ApiError> {
    let user_id = user_id_or_forbidden(&principal)?;
    let repo = session_repo(&state);
    let target = SessionId(session_id);

    // Ownership is resolved through the caller's own scoped list: a session
    // id absent from it does not exist for this caller (404, never 403).
    let sessions = repo
        .list_for_user(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let Some(session) = sessions.iter().find(|s| s.id == target) else {
        return Err(ApiError::NotFound);
    };

    // Already revoked: idempotent no-op, so a replay keeps the original
    // revocation timestamp instead of restamping it.
    if session.revoked_at.is_some() {
        return Ok(StatusCode::NO_CONTENT);
    }

    repo.revoke(target).await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/sessions",
    tag = "auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Every other active session revoked; the current session stays alive"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot revoke user sessions"),
    )
)]
pub(crate) async fn revoke_other_sessions(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<StatusCode, ApiError> {
    let user_id = user_id_or_forbidden(&principal)?;

    // Revoking the current session itself is POST /auth/logout; this route
    // deliberately keeps it alive.
    let current = current_session(&state, user_id, &headers, &jar).await?;

    session_repo(&state)
        .revoke_all_for_user_except(user_id, current.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
}
