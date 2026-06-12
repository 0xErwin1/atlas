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

use atlas_api::dtos::{LoginRequest, LoginResponse, MeResponse, UserDto};
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

    let user = user_repo
        .find_by_username(&body.username)
        .await
        .map_err(|_| ApiError::Unauthorized)?
        .ok_or(ApiError::Unauthorized)?;

    if user.disabled_at.is_some() {
        return Err(ApiError::Unauthorized);
    }

    let is_valid = password::verify(body.password, user.password_hash.clone())
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    if !is_valid {
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
            {
                session_repo.revoke(session.id).await.ok();
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

    let (principal_type, username) = match principal {
        Principal::User(user_id) => {
            let user = user_repo
                .find_by_id(user_id)
                .await
                .map_err(|_| ApiError::Internal {
                    message: "user lookup failed".into(),
                })?
                .ok_or(ApiError::Unauthorized)?;
            ("user".to_string(), user.username)
        }
        Principal::ApiKey(_key_id) => ("api_key".to_string(), "api_key".to_string()),
    };

    Ok(Json(MeResponse {
        principal_type,
        username,
    }))
}

fn user_to_dto(user: &atlas_domain::entities::identity::User) -> UserDto {
    UserDto {
        id: user.id.0,
        username: user.username.clone(),
        display_name: user.display_name.clone(),
        is_root: user.is_root,
        disabled_at: user.disabled_at,
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
