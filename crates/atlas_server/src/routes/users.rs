use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{Duration, Utc};

use atlas_api::dtos::{
    ActivationLinkResponse, CreateUserRequest, CreateUserResponse, ResetPasswordRequest,
    SetSystemAdminRequest, UserDto,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{MemberRole, NewUser},
    ids::UserId,
};

use crate::{
    auth::tokens::{generate_session_token, hash_token},
    authz::{RequireRoot, RequireUserAdmin},
    error::ApiError,
    persistence::repos::{
        ActivationTokenRepo, MembershipRepo, NewActivationToken, PgActivationTokenRepo,
        PgMembershipRepo, PgSessionRepo, PgUserRepo, PgWorkspaceRepo, SessionRepo, UserRepo,
        WorkspaceRepo,
    },
    state::AppState,
};

/// How long an activation token remains valid.
const ACTIVATION_TOKEN_TTL_DAYS: i64 = 7;

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
        (status = 201, description = "Pending user created with activation link", body = CreateUserResponse),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
        (status = 422, description = "Workspace not found, or role=owner"),
    )
)]
pub(crate) async fn create_user(
    admin: RequireUserAdmin,
    State(state): State<AppState>,
    Json(body): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let role = parse_member_role(&body.role)?;

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: (*state.db).clone(),
    };
    let token_repo = PgActivationTokenRepo {
        conn: (*state.db).clone(),
    };

    let workspace = ws_repo
        .find_by_slug(&body.workspace)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or_else(|| ApiError::InvalidInput {
            message: format!("workspace '{}' not found", body.workspace),
        })?;

    let user = user_repo
        .create(NewUser {
            username: body.username,
            display_name: body.display_name,
            email: body.email,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let ctx = WorkspaceCtx::new(workspace.id, Actor::User(admin.user.id));
    membership_repo
        .add(&ctx, user.id, role)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let plaintext = generate_session_token();
    let token_hash = hash_token(&plaintext);
    let expires_at = Utc::now() + Duration::days(ACTIVATION_TOKEN_TTL_DAYS);

    token_repo
        .create(NewActivationToken {
            user_id: user.id,
            token_hash,
            expires_at,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let activation_link = build_activation_link(&plaintext);

    let response = CreateUserResponse {
        user: user_to_dto(&user),
        activation_link,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

#[utoipa::path(
    post,
    path = "/v1/users/{user_id}/activation-link",
    tag = "users",
    security(("bearer_auth" = [])),
    params(("user_id" = uuid::Uuid, Path, description = "User ID")),
    responses(
        (status = 200, description = "Fresh activation link issued", body = ActivationLinkResponse),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
        (status = 409, description = "User is already activated"),
    )
)]
pub(crate) async fn regenerate_activation_link(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Path(user_id): Path<uuid::Uuid>,
) -> Result<Json<ActivationLinkResponse>, ApiError> {
    let user_id = UserId(user_id);

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let token_repo = PgActivationTokenRepo {
        conn: (*state.db).clone(),
    };

    let user = user_repo
        .find_by_id(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    if user.activated_at.is_some() {
        return Err(ApiError::Conflict);
    }

    token_repo
        .invalidate_unconsumed_for_user(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let plaintext = generate_session_token();
    let token_hash = hash_token(&plaintext);
    let expires_at = Utc::now() + Duration::days(ACTIVATION_TOKEN_TTL_DAYS);

    token_repo
        .create(NewActivationToken {
            user_id,
            token_hash,
            expires_at,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let activation_link = build_activation_link(&plaintext);

    Ok(Json(ActivationLinkResponse { activation_link }))
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

    use crate::auth::password;

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

/// Parses a membership role string. Only "admin" and "member" are accepted;
/// "owner" is explicitly rejected with 422 to prevent privilege escalation.
fn parse_member_role(role: &str) -> Result<MemberRole, ApiError> {
    match role {
        "admin" => Ok(MemberRole::Admin),
        "member" => Ok(MemberRole::Member),
        "owner" => Err(ApiError::InvalidInput {
            message: "role 'owner' is not permitted when inviting a user; use 'admin' or 'member'"
                .into(),
        }),
        other => Err(ApiError::InvalidInput {
            message: format!("unknown role '{other}'; use 'admin' or 'member'"),
        }),
    }
}

/// Builds the activation link path from a plaintext token.
///
/// The base URL prefix comes from the `ATLAS_SERVER_URL` environment variable
/// when set; otherwise the link is a bare path so the web layer can prefix it.
fn build_activation_link(plaintext: &str) -> String {
    let base = std::env::var("ATLAS_SERVER_URL").unwrap_or_default();
    format!("{base}/activate/{plaintext}")
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
