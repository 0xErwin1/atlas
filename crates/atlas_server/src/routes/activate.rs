use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use chrono::Utc;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement, TransactionTrait};
use uuid::Uuid;

use atlas_api::dtos::{ActivatePasswordRequest, ActivationInfoDto, LoginResponse};
use atlas_domain::ids::{ActivationTokenId, SessionId, UserId};

use atlas_domain::{
    Actor,
    entities::security_audit::{NewSecurityAuditEvent, SecurityAction},
};

use crate::{
    auth::{
        password,
        tokens::{generate_session_token, hash_token},
    },
    error::ApiError,
    persistence::repos::{
        ActivationTokenRepo, PgActivationTokenRepo, PgSecurityAuditRepo, PgUserRepo, UserRepo,
    },
    routes::auth::user_to_dto,
    state::AppState,
};

/// Validates the password meets minimum length before any DB work is done.
///
/// A min-length of 8 is the only rule applied here. It is checked before
/// argon2 hashing so the token is not consumed when the password is too weak.
pub(crate) fn validate_password_strength(pw: &str) -> Result<(), ApiError> {
    if pw.chars().count() < 8 {
        return Err(ApiError::InvalidInput {
            message: "Password must be at least 8 characters long.".into(),
        });
    }

    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/activate/{token}",
    tag = "auth",
    params(("token" = String, Path, description = "Activation token")),
    responses(
        (status = 200, description = "Token valid — returns display info", body = ActivationInfoDto),
        (status = 404, description = "Invalid or expired activation link"),
        (status = 429, description = "Rate limit exceeded"),
    )
)]
pub(crate) async fn get_activation_info(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<ActivationInfoDto>, ApiError> {
    let token_repo = PgActivationTokenRepo {
        conn: (*state.db).clone(),
    };
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let token_hash = hash_token(&token);

    let activation_token = token_repo
        .find_active_by_token_hash(&token_hash)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let user = user_repo
        .find_by_id(activation_token.user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(ActivationInfoDto {
        username: user.username,
        display_name: user.display_name,
    }))
}

/// Completes account activation in a single DB transaction.
///
/// Atomicity invariant: the row lock (`FOR UPDATE` on the token SELECT) combined
/// with the guarded consume (`AND consumed_at IS NULL` on the UPDATE plus a
/// rows-affected check) guarantee that exactly one activation succeeds per token
/// even under concurrent POST requests. Under READ COMMITTED, two racers that
/// reach the SELECT simultaneously will serialize at the lock: the loser blocks
/// until the winner commits, at which point the loser's `FOR UPDATE` re-read
/// sees the now-consumed row, the guarded UPDATE matches 0 rows, and the
/// transaction is rolled back with a 404 — no double-activation, no
/// double-session.
///
/// Steps inside the transaction:
/// 1. Re-validate and lock the token row (`FOR UPDATE`).
/// 2. Consume the token (`AND consumed_at IS NULL`); abort if already consumed.
/// 3. Set the user's `password_hash` and `activated_at`.
/// 4. Create a new session for the newly activated user.
///
/// Any failure rolls back the entire transaction: the user stays pending,
/// the token stays unconsumed, and no session is issued.
#[utoipa::path(
    post,
    path = "/api/activate/{token}",
    tag = "auth",
    params(("token" = String, Path, description = "Activation token")),
    request_body = ActivatePasswordRequest,
    responses(
        (status = 200, description = "Activated and logged in", body = LoginResponse),
        (status = 404, description = "Invalid or expired activation link"),
        (status = 422, description = "Password too short"),
        (status = 429, description = "Rate limit exceeded"),
    )
)]
pub(crate) async fn post_activate(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(token): Path<String>,
    Json(body): Json<ActivatePasswordRequest>,
) -> Result<(CookieJar, Response), ApiError> {
    validate_password_strength(&body.password)?;

    let token_hash = hash_token(&token);

    let password_hash = password::hash(body.password)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let raw_session_token = generate_session_token();
    let session_token_hash = hash_token(&raw_session_token);
    let session_expires_at = Utc::now() + chrono::Duration::hours(state.session_ttl_hours);
    let now = Utc::now();

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    #[derive(Debug, FromQueryResult)]
    struct TokenRow {
        id: Uuid,
        user_id: Uuid,
    }

    let token_rows = TokenRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT id, user_id FROM user_activation_tokens \
         WHERE token_hash = $1 AND consumed_at IS NULL AND expires_at > now() \
         LIMIT 1 FOR UPDATE",
        [token_hash.into()],
    ))
    .all(&txn)
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let Some(token_row) = token_rows.into_iter().next() else {
        txn.rollback().await.ok();
        return Err(ApiError::NotFound);
    };

    let token_id = ActivationTokenId(token_row.id);
    let user_id = UserId(token_row.user_id);

    // A disabled user must not be able to self-activate. Re-read the user inside
    // the transaction and reject with the SAME generic 404 used for an invalid
    // token, so the response is not a "this account is disabled" oracle.
    #[derive(Debug, FromQueryResult)]
    struct UserStateRow {
        disabled_at: Option<chrono::DateTime<Utc>>,
    }

    let user_state_rows = UserStateRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT disabled_at FROM users WHERE id = $1 LIMIT 1",
        [user_id.0.into()],
    ))
    .all(&txn)
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let user_is_disabled = user_state_rows
        .first()
        .map(|r| r.disabled_at.is_some())
        .unwrap_or(true);

    if user_is_disabled {
        txn.rollback().await.ok();
        return Err(ApiError::NotFound);
    }

    let consume_result = txn
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE user_activation_tokens \
             SET consumed_at = $1 \
             WHERE id = $2 AND consumed_at IS NULL",
            [now.into(), token_id.0.into()],
        ))
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if consume_result.rows_affected() == 0 {
        txn.rollback().await.ok();
        return Err(ApiError::NotFound);
    }

    // Guard on `activated_at IS NULL`: a stray still-live token must never
    // silently reset an already-activated user's password. If the user is no
    // longer pending, no row matches and we roll back with the generic 404.
    let activate_result = txn
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE users SET password_hash = $1, activated_at = $2, updated_at = $3 \
             WHERE id = $4 AND activated_at IS NULL",
            [
                password_hash.into(),
                now.into(),
                now.into(),
                user_id.0.into(),
            ],
        ))
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if activate_result.rows_affected() == 0 {
        txn.rollback().await.ok();
        return Err(ApiError::NotFound);
    }

    let session_id = SessionId::new();
    txn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO sessions (id, user_id, token_hash, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
        [
            session_id.0.into(),
            user_id.0.into(),
            session_token_hash.into(),
            session_expires_at.into(),
            now.into(),
        ],
    ))
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    // Actor == target: the activating user is the same person acting on themselves.
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: Actor::User(user_id),
            action: SecurityAction::AccountActivated,
            target_type: "user".to_string(),
            target_id: Some(user_id.0),
            metadata: serde_json::json!({}),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let user = user_repo
        .find_by_id(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::Internal {
            message: "user not found after activation commit".into(),
        })?;

    let user_dto = user_to_dto(&user);
    let response_body = LoginResponse {
        token: raw_session_token.clone(),
        expires_at: session_expires_at,
        user: user_dto,
    };

    let cookie = build_session_cookie(raw_session_token, session_expires_at, state.cookie_secure);
    let updated_jar = jar.add(cookie);

    Ok((
        updated_jar,
        (StatusCode::OK, Json(response_body)).into_response(),
    ))
}

fn build_session_cookie(
    token: String,
    expires_at: chrono::DateTime<Utc>,
    secure: bool,
) -> Cookie<'static> {
    let max_age_secs = (expires_at - Utc::now()).num_seconds().max(0);
    Cookie::build(("atlas_session", token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(time::Duration::seconds(max_age_secs))
        .build()
}
