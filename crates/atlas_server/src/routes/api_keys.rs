use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct RevokeKeyPath {
    #[allow(dead_code)]
    pub(crate) ws: String,
    pub(crate) key_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct TopLevelRevokeKeyPath {
    pub(crate) key_id: uuid::Uuid,
}

use atlas_api::{
    dtos::{
        ApiKeyCreated, ApiKeyDto, CreateApiKeyRequest, CreateUserApiKeyRequest, InitialGrantRequest,
    },
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{identity::ApiKeyType, permissions::NewPermissionGrant},
    ids::{ApiKeyId, UserId},
    permissions::{Principal, ResourceRole, ShareDenied, authorize_grant_target},
};

use crate::{
    auth::{
        middleware::Principal as AuthPrincipal,
        tokens::{generate_api_key, hash_token},
    },
    authz::{AdminMin, Authorized, authorized::WorkspaceRes},
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, NewApiKey, PermissionGrantRepo, PgApiKeyRepo, PgPermissionGrantRepo,
        PgWorkspaceRepo, WorkspaceRepo,
    },
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

fn parse_key_type(s: Option<&str>) -> Result<ApiKeyType, ApiError> {
    match s.unwrap_or("agent") {
        "agent" => Ok(ApiKeyType::Agent),
        "cli" => Ok(ApiKeyType::Cli),
        "bot" => Ok(ApiKeyType::Bot),
        "integration" => Ok(ApiKeyType::Integration),
        other => Err(ApiError::InvalidInput {
            message: format!(
                "invalid key type: {other}; expected 'agent', 'cli', 'bot', or 'integration'"
            ),
        }),
    }
}

fn parse_role(role: &str) -> Result<ResourceRole, ApiError> {
    match role {
        "viewer" => Ok(ResourceRole::Viewer),
        "editor" => Ok(ResourceRole::Editor),
        _ => Err(ApiError::InvalidInput {
            message: format!(
                "invalid role: {role}; expected 'viewer' or 'editor' (agent cap prevents admin)"
            ),
        }),
    }
}

fn key_to_dto(k: &atlas_domain::entities::identity::ApiKey) -> ApiKeyDto {
    ApiKeyDto {
        id: k.id.0,
        name: k.name.clone(),
        r#type: k.type_.as_str().to_string(),
        expires_at: k.expires_at,
        last_used_at: k.last_used_at,
        revoked_at: k.revoked_at,
        created_at: k.created_at,
    }
}

// ---------------------------------------------------------------------------
// Top-level user-owned key routes (`/v1/api-keys`)
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/api-keys",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    request_body = CreateUserApiKeyRequest,
    responses(
        (status = 201, description = "API key created (secret shown once)", body = ApiKeyCreated),
        (status = 400, description = "Invalid key type or role"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot create keys"),
    )
)]
pub(crate) async fn create_user_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreateUserApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = match principal {
        AuthPrincipal::User(uid) => uid,
        AuthPrincipal::ApiKey(_) => {
            return Err(ApiError::Forbidden {
                message: "API keys cannot create other API keys".into(),
            });
        }
    };

    let key_type = parse_key_type(body.r#type.as_deref())?;
    let secret = generate_api_key();
    let token_hash = hash_token(&secret);

    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let key = repo
        .create_for_user(
            user_id,
            NewApiKey {
                name: body.name,
                token_hash,
                type_: key_type,
                expires_at: body.expires_at,
            },
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if let Some(grant_req) = body.initial_grant {
        create_initial_grant(&state, user_id, key.id, &grant_req).await?;
    }

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreated {
            id: key.id.0,
            name: key.name,
            secret,
            r#type: key.type_.as_str().to_string(),
            expires_at: key.expires_at,
            created_at: key.created_at,
        }),
    ))
}

/// Creates a workspace-scope grant for a newly created key. Rejects admin roles
/// (agent cap enforced via `authorize_grant_target`).
async fn create_initial_grant(
    state: &AppState,
    user_id: UserId,
    key_id: ApiKeyId,
    grant_req: &InitialGrantRequest,
) -> Result<(), ApiError> {
    let role = parse_role(&grant_req.role)?;

    authorize_grant_target(&Principal::ApiKey(key_id), role).map_err(|e| {
        let message = match e {
            ShareDenied::AgentCannotBeAdmin => {
                "agents cannot be granted the Admin role".to_string()
            }
            _ => "insufficient permissions to create grant".to_string(),
        };
        ApiError::Forbidden { message }
    })?;

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };
    let workspace = ws_repo
        .find_by_slug(&grant_req.workspace)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: workspace.id,
            user_id: None,
            api_key_id: Some(key_id),
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role,
            created_by_user_id: Some(user_id),
            created_by_api_key_id: None,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/api-keys",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated list of the caller's API keys", body = Page<ApiKeyDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot list keys"),
    )
)]
pub(crate) async fn list_user_api_keys(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ApiKeyDto>>, ApiError> {
    let user_id = match principal {
        AuthPrincipal::User(uid) => uid,
        AuthPrincipal::ApiKey(_) => {
            return Err(ApiError::Forbidden {
                message: "API keys cannot list API keys".into(),
            });
        }
    };

    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let all_keys = repo
        .list_for_user(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let mut filtered: Vec<_> = all_keys
        .into_iter()
        .filter(|k| after_id.is_none_or(|cursor| k.id.0 > cursor))
        .collect();

    let has_more = filtered.len() > limit as usize;
    if has_more {
        filtered.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        filtered.last().map(|k| Cursor(k.id.0))
    } else {
        None
    };

    let dtos: Vec<ApiKeyDto> = filtered.iter().map(key_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

#[utoipa::path(
    delete,
    path = "/v1/api-keys/{key_id}",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    responses(
        (status = 204, description = "API key revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Cannot revoke another user's key"),
        (status = 404, description = "Key not found or already revoked"),
    )
)]
pub(crate) async fn revoke_user_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
) -> Result<StatusCode, ApiError> {
    let user_id = match principal {
        AuthPrincipal::User(uid) => uid,
        AuthPrincipal::ApiKey(_) => {
            return Err(ApiError::Forbidden {
                message: "API keys cannot revoke API keys".into(),
            });
        }
    };

    let key_id = ApiKeyId(params.key_id);
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    repo.revoke_for_user(user_id, key_id)
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::NotFound { .. } => ApiError::NotFound,
            atlas_domain::DomainError::Forbidden { message } => ApiError::Forbidden { message },
            other => ApiError::Internal {
                message: other.to_string(),
            },
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Deprecated workspace-scoped key routes (`/v1/workspaces/{ws}/api-keys`)
// ---------------------------------------------------------------------------
//
// Kept functional while the web client (C2c) migrates to the top-level routes.
// Do not remove until C2c ships.

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/api-keys",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "API key created (secret shown once)", body = ApiKeyCreated),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn create_api_key(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let secret = generate_api_key();
    let token_hash = hash_token(&secret);

    let actor = match &auth.principal {
        atlas_domain::permissions::Principal::User(uid) => Actor::User(*uid),
        atlas_domain::permissions::Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    };
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let creator_user_id = match &auth.principal {
        Principal::User(uid) => Some(*uid),
        Principal::ApiKey(_) => None,
    };

    let key = repo
        .create(
            &ctx,
            NewApiKey {
                name: body.name,
                token_hash,
                type_: ApiKeyType::Agent,
                expires_at: body.expires_at,
            },
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: auth.workspace.id,
            user_id: None,
            api_key_id: Some(key.id),
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: creator_user_id,
            created_by_api_key_id: None,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreated {
            id: key.id.0,
            name: key.name,
            secret,
            r#type: key.type_.as_str().to_string(),
            expires_at: key.expires_at,
            created_at: key.created_at,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/api-keys",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated API key list", body = Page<ApiKeyDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_api_keys(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ApiKeyDto>>, ApiError> {
    let actor = match &auth.principal {
        atlas_domain::permissions::Principal::User(uid) => Actor::User(*uid),
        atlas_domain::permissions::Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    };
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let all_keys = repo.list(&ctx).await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let mut filtered: Vec<_> = all_keys
        .into_iter()
        .filter(|k| after_id.is_none_or(|cursor| k.id.0 > cursor))
        .collect();

    let has_more = filtered.len() > limit as usize;
    if has_more {
        filtered.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        filtered.last().map(|k| Cursor(k.id.0))
    } else {
        None
    };

    let dtos: Vec<ApiKeyDto> = filtered.iter().map(key_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/api-keys/{key_id}/revoke",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("key_id" = uuid::Uuid, Path, description = "API key id"),
    ),
    responses(
        (status = 204, description = "API key revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn revoke_api_key(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Path(params): Path<RevokeKeyPath>,
) -> Result<StatusCode, ApiError> {
    let key_id = atlas_domain::ids::ApiKeyId(params.key_id);

    let actor = match &auth.principal {
        atlas_domain::permissions::Principal::User(uid) => Actor::User(*uid),
        atlas_domain::permissions::Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    };
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    repo.revoke(&ctx, key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
}
