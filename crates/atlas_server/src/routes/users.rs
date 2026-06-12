use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::{CreateUserRequest, UserDto};
use atlas_domain::{entities::identity::NewUser, ids::UserId};

use crate::{
    auth::password,
    authz::RequireUserAdmin,
    error::ApiError,
    persistence::repos::{PgSessionRepo, PgUserRepo, SessionRepo, UserRepo},
    state::AppState,
};

#[utoipa::path(
    post,
    path = "/v1/users",
    tag = "users",
    security(("bearer_auth" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = UserDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
    )
)]
pub(crate) async fn create_user(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Json(body): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let password_hash = password::hash(body.password)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let user = user_repo
        .create(NewUser {
            username: body.username,
            display_name: body.display_name,
            password_hash,
            is_root: false,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let dto = user_to_dto(&user);
    Ok((StatusCode::CREATED, Json(dto)))
}

#[utoipa::path(
    post,
    path = "/v1/users/{user_id}/disable",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("user_id" = uuid::Uuid, Path, description = "User ID")),
    responses(
        (status = 204, description = "User disabled"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
    )
)]
pub(crate) async fn disable_user(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<uuid::Uuid>,
) -> Result<StatusCode, ApiError> {
    let user_id = UserId(user_id);
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let session_repo = PgSessionRepo {
        conn: (*state.db).clone(),
    };

    user_repo
        .disable(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    session_repo
        .revoke_all_for_user(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/v1/users/{user_id}/enable",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("user_id" = uuid::Uuid, Path, description = "User ID")),
    responses(
        (status = 204, description = "User enabled"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
    )
)]
pub(crate) async fn enable_user(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<uuid::Uuid>,
) -> Result<StatusCode, ApiError> {
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    user_repo
        .enable(UserId(user_id))
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
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
