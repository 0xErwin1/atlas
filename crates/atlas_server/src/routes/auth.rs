use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use chrono::Utc;
use std::sync::LazyLock;

use atlas_api::dtos::{
    ChangePasswordRequest, LoginRequest, LoginResponse, MeResponse, UpdateMeRequest, UserDto,
};
use atlas_domain::ids::{SessionId, UserId};

use crate::{
    auth::{
        middleware::Principal,
        password,
        tokens::{generate_session_token, hash_token},
    },
    error::ApiError,
    persistence::repos::{NewSession, PgSessionRepo, PgUserRepo, SessionRepo, UserRepo},
    state::AppState,
};

/// A pre-computed argon2 hash of a throwaway password used to equalise timing
/// when the submitted username does not exist. Without this, an attacker could
/// distinguish "unknown user" from "wrong password" by measuring response latency.
///
/// The value is a valid PHC-format hash; producing it cannot fail under normal
/// circumstances. If it somehow does, the server panics at startup rather than
/// silently shipping a timing oracle — a deliberate fail-fast choice.
#[allow(clippy::expect_used)]
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    use argon2::{
        Argon2, PasswordHasher, password_hash::SaltString, password_hash::rand_core::OsRng,
    };
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(b"atlas_dummy_password_for_timing", &salt)
        .expect("argon2 dummy hash initialisation failed")
        .to_string()
});

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 401, description = "Invalid credentials"),
        (status = 429, description = "Rate limit exceeded"),
    )
)]
pub(crate) async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<LoginRequest>,
) -> Result<(CookieJar, Response), ApiError> {
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let session_repo = PgSessionRepo {
        conn: (*state.db).clone(),
    };

    let maybe_user = user_repo
        .find_by_username(&body.username)
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    let Some(user) = maybe_user else {
        // Run a dummy verify so both branches pay the same argon2 cost.
        let _ = password::verify(body.password, DUMMY_HASH.clone()).await;
        return Err(ApiError::Unauthorized);
    };

    // A pending user has no password hash. Run a dummy verify so the timing
    // cost matches the normal path, then reject uniformly with 401. A pending
    // account, an unknown user, and a wrong password are all indistinguishable:
    // returning a distinct account-state error here would be an enumeration
    // oracle. A valid password can only exist on an activated account (the hash
    // is set at activation), so no legitimate login is lost by this uniformity.
    let hash_to_check = user
        .password_hash
        .clone()
        .unwrap_or_else(|| DUMMY_HASH.clone());

    let is_valid = password::verify(body.password, hash_to_check)
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    if !is_valid {
        return Err(ApiError::Unauthorized);
    }

    if user.disabled_at.is_some() {
        return Err(ApiError::Unauthorized);
    }

    let raw_token = generate_session_token();
    let token_hash = hash_token(&raw_token);
    let expires_at = Utc::now() + chrono::Duration::hours(state.session_ttl_hours);

    let session = session_repo
        .create(NewSession {
            user_id: user.id,
            token_hash,
            expires_at,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let user_dto = user_to_dto(&user);
    let response_body = LoginResponse {
        token: raw_token.clone(),
        expires_at: session.expires_at,
        user: user_dto,
    };

    let cookie = build_session_cookie(raw_token, session.expires_at, state.cookie_secure);
    let updated_jar = jar.add(cookie);

    Ok((
        updated_jar,
        (StatusCode::OK, Json(response_body)).into_response(),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    tag = "auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Logged out"),
        (status = 401, description = "Unauthenticated"),
    )
)]
pub(crate) async fn logout(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), ApiError> {
    if let Principal::User(user_id) = principal {
        let session_repo = PgSessionRepo {
            conn: (*state.db).clone(),
        };

        let bearer_token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.to_owned());

        let raw_token = jar
            .get("atlas_session")
            .map(|c| c.value().to_owned())
            .or(bearer_token);

        if let Some(raw) = raw_token {
            let hash = hash_token(&raw);
            if let Ok(Some(session)) = session_repo.find_active_by_token_hash(&hash).await
                && session.user_id == user_id
                && let Err(e) = session_repo.revoke(session.id).await
            {
                tracing::warn!(error = %e, "logout: failed to revoke session");
            }
        }
    }

    let removal_cookie = Cookie::build(("atlas_session", ""))
        .path("/")
        .max_age(time::Duration::ZERO)
        .build();

    Ok((jar.remove(removal_cookie), StatusCode::NO_CONTENT))
}

#[utoipa::path(
    get,
    path = "/v1/auth/me",
    tag = "auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current principal", body = MeResponse),
        (status = 401, description = "Unauthenticated"),
    )
)]
pub(crate) async fn me(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<MeResponse>, ApiError> {
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let me = match principal {
        Principal::User(user_id) => {
            let user = user_repo
                .find_by_id(user_id)
                .await
                .map_err(|_| ApiError::Internal {
                    message: "user lookup failed".into(),
                })?
                .ok_or(ApiError::Unauthorized)?;
            MeResponse {
                principal_type: "user".to_string(),
                username: user.username,
                email: user.email,
                id: Some(user.id.0),
                display_name: Some(user.display_name),
                is_root: user.is_root,
                is_system_admin: user.is_system_admin,
            }
        }
        Principal::ApiKey(_key_id) => MeResponse {
            principal_type: "api_key".to_string(),
            username: "api_key".to_string(),
            email: None,
            id: None,
            display_name: None,
            is_root: false,
            is_system_admin: false,
        },
    };

    Ok(Json(me))
}

#[utoipa::path(
    post,
    path = "/v1/auth/change-password",
    tag = "auth",
    security(("bearer_auth" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed"),
        (status = 401, description = "Unauthenticated or wrong current password"),
        (status = 403, description = "API keys cannot change a user password"),
    )
)]
pub(crate) async fn change_password(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let Principal::User(user_id) = principal else {
        return Err(ApiError::Forbidden {
            message: "API keys cannot change a user password".into(),
        });
    };

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let session_repo = PgSessionRepo {
        conn: (*state.db).clone(),
    };

    let user = user_repo
        .find_by_id(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::Unauthorized)?;

    let Some(current_hash) = user.password_hash.clone() else {
        return Err(ApiError::Unauthorized);
    };

    let is_valid = password::verify(body.current_password, current_hash)
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    if !is_valid {
        return Err(ApiError::Unauthorized);
    }

    let new_hash = password::hash(body.new_password)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    user_repo
        .set_password_hash(user_id, new_hash)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    // Resolve the calling session so we can keep it alive while revoking all
    // others. A stolen session token that survives a self password-change is a
    // security gap — we close it here.
    let bearer_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_owned());

    let raw_token = jar
        .get("atlas_session")
        .map(|c| c.value().to_owned())
        .or(bearer_token);

    if let Some(raw) = raw_token {
        let hash = hash_token(&raw);
        if let Ok(Some(session)) = session_repo.find_active_by_token_hash(&hash).await
            && let Err(e) = session_repo
                .revoke_all_for_user_except(user_id, session.id)
                .await
        {
            tracing::warn!(error = %e, "change_password: failed to revoke other sessions");
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/v1/users/me",
    tag = "users",
    security(("bearer_auth" = [])),
    request_body = UpdateMeRequest,
    responses(
        (status = 200, description = "Updated profile", body = UserDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot update a user profile"),
    )
)]
pub(crate) async fn update_me(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<UpdateMeRequest>,
) -> Result<Json<UserDto>, ApiError> {
    let Principal::User(user_id) = principal else {
        return Err(ApiError::Forbidden {
            message: "API keys cannot update a user profile".into(),
        });
    };

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let user = user_repo
        .update_profile(user_id, body.email, body.display_name)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(user_to_dto(&user)))
}

pub(crate) fn user_to_dto(user: &atlas_domain::entities::identity::User) -> UserDto {
    UserDto {
        id: user.id.0,
        username: user.username.clone(),
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        is_root: user.is_root,
        is_system_admin: user.is_system_admin,
        disabled_at: user.disabled_at,
        activated_at: user.activated_at,
        created_at: user.created_at,
        updated_at: user.updated_at,
    }
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

#[allow(dead_code)]
fn session_id_unused(_: SessionId) {}

#[allow(dead_code)]
fn user_id_unused(_: UserId) {}
