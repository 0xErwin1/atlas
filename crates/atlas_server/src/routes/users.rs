use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::{CreateUserRequest, ResetPasswordRequest, SetSystemAdminRequest, UserDto};
use atlas_domain::{entities::identity::NewUser, ids::UserId};

use crate::{
    auth::password,
    authz::{RequireRoot, RequireUserAdmin},
    error::ApiError,
    persistence::repos::{PgSessionRepo, PgUserRepo, SessionRepo, UserRepo},
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/v1/users",
    tag = "users",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "All users (active and disabled)", body = [UserDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
    )
)]
pub(crate) async fn list_users(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let users = user_repo.list().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let dtos = users.iter().map(user_to_dto).collect();
    Ok(Json(dtos))
}

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
            email: body.email,
            password_hash: Some(password_hash),
            is_root: false,
            is_system_admin: false,
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
        (status = 403, description = "Not a root/admin user, or target is protected"),
    )
)]
pub(crate) async fn disable_user(
    admin: RequireUserAdmin,
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

    // A non-root system-admin may not disable a root or another system-admin.
    // Root bypasses this check and can disable any non-self target.
    if !admin.user.is_root {
        let target = user_repo
            .find_by_id(user_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        if target.is_root || target.is_system_admin {
            return Err(ApiError::Forbidden {
                message: "Cannot disable a root or system-admin user".into(),
            });
        }
    }

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

#[utoipa::path(
    post,
    path = "/v1/users/{user_id}/reset-password",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("user_id" = uuid::Uuid, Path, description = "User ID")),
    request_body = ResetPasswordRequest,
    responses(
        (status = 204, description = "Password reset and all sessions revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user, or target is protected"),
    )
)]
pub(crate) async fn reset_password(
    admin: RequireUserAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<uuid::Uuid>,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let user_id = UserId(user_id);

    let new_hash = password::hash(body.new_password)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let session_repo = PgSessionRepo {
        conn: (*state.db).clone(),
    };

    // A non-root system-admin may not reset the password of a root or another system-admin.
    if !admin.user.is_root {
        let target = user_repo
            .find_by_id(user_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        if target.is_root || target.is_system_admin {
            return Err(ApiError::Forbidden {
                message: "Cannot reset the password of a root or system-admin user".into(),
            });
        }
    }

    user_repo
        .set_password_hash(user_id, new_hash)
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
    path = "/v1/users/{user_id}/system-admin",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("user_id" = uuid::Uuid, Path, description = "User ID")),
    request_body = SetSystemAdminRequest,
    responses(
        (status = 200, description = "User system-admin status updated", body = UserDto),
        (status = 400, description = "Cannot target self or a root user"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Root access required"),
    )
)]
pub(crate) async fn set_system_admin(
    root: RequireRoot,
    State(state): State<AppState>,
    Path(user_id): Path<uuid::Uuid>,
    Json(body): Json<SetSystemAdminRequest>,
) -> Result<Json<UserDto>, ApiError> {
    let target_id = UserId(user_id);

    if target_id == root.user.id {
        return Err(ApiError::BadRequest {
            message: "Cannot change system-admin status of yourself".into(),
        });
    }

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let target = user_repo
        .find_by_id(target_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    if target.is_root {
        return Err(ApiError::BadRequest {
            message: "Cannot change system-admin status of a root user".into(),
        });
    }

    let updated = user_repo
        .set_system_admin(target_id, body.is_system_admin)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(user_to_dto(&updated)))
}

fn user_to_dto(user: &atlas_domain::entities::identity::User) -> UserDto {
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
