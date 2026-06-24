use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub(crate) struct TopLevelRevokeKeyPath {
    pub(crate) key_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct ApiKeyGrantPath {
    pub(crate) key_id: uuid::Uuid,
    pub(crate) grant_id: uuid::Uuid,
}

use atlas_api::{
    dtos::{
        ApiKeyCreated, ApiKeyDto, ApiKeyGrantDto, CreateUserApiKeyRequest, InitialGrantRequest,
    },
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor,
    entities::{
        identity::ApiKeyType,
        permissions::{NewPermissionGrant, PermissionGrant, PermissionGrantId},
        security_audit::{NewSecurityAuditEvent, SecurityAction},
    },
    ids::{ApiKeyId, ProjectId, UserId, WorkspaceId},
    permissions::{Principal, ResourceRole, ShareDenied, authorize_grant_target},
};
use sea_orm::TransactionTrait;

use crate::{
    auth::{
        middleware::Principal as AuthPrincipal,
        tokens::{generate_api_key, hash_token},
    },
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, NewApiKey, PermissionGrantRepo, PgApiKeyRepo, PgPermissionGrantRepo,
        PgProjectRepo, PgSecurityAuditRepo, PgWorkspaceRepo, ProjectRepo, WorkspaceRepo,
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

    let new_key = NewApiKey {
        name: body.name,
        token_hash,
        type_: key_type,
        expires_at: body.expires_at,
    };

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let key = PgApiKeyRepo::create_for_user_in(&txn, user_id, new_key)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: Actor::User(user_id),
            action: SecurityAction::ApiKeyCreated,
            target_type: "api_key".to_string(),
            target_id: Some(key.id.0),
            metadata: serde_json::json!({
                "key_type": key.type_.as_str(),
                "key_name": key.name,
            }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
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

/// Resolves workspace name/slug and optional project name/slug for a set of grants.
///
/// Returns a map of `workspace_id → (slug, name)` and a map of
/// `project_id → (slug, name)` for every workspace/project referenced in the
/// given grants. Unknown ids are omitted; callers fall back to the raw id.
async fn resolve_resource_labels(
    state: &AppState,
    grants: &[PermissionGrant],
    caller: UserId,
) -> Result<
    (
        HashMap<WorkspaceId, (String, String)>,
        HashMap<ProjectId, (String, String)>,
    ),
    ApiError,
> {
    let ws_ids: Vec<WorkspaceId> = {
        let mut seen = std::collections::HashSet::new();
        grants
            .iter()
            .filter(|g| seen.insert(g.workspace_id.0))
            .map(|g| g.workspace_id)
            .collect()
    };

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    let mut ws_map: HashMap<WorkspaceId, (String, String)> = HashMap::new();
    for ws_id in ws_ids {
        if let Ok(Some(ws)) = ws_repo.find_by_id(ws_id).await {
            ws_map.insert(ws_id, (ws.slug, ws.name));
        }
    }

    let project_grants: Vec<(ProjectId, WorkspaceId)> = grants
        .iter()
        .filter_map(|g| g.project_id.map(|pid| (pid, g.workspace_id)))
        .collect();

    let mut project_map: HashMap<ProjectId, (String, String)> = HashMap::new();
    for (project_id, workspace_id) in project_grants {
        if project_map.contains_key(&project_id) {
            continue;
        }
        let proj_repo = PgProjectRepo {
            conn: (*state.db).clone(),
        };
        let ctx = atlas_domain::WorkspaceCtx::new(workspace_id, atlas_domain::Actor::User(caller));
        if let Ok(Some(proj)) = proj_repo.find(&ctx, project_id).await {
            project_map.insert(project_id, (proj.slug, proj.name));
        }
    }

    Ok((ws_map, project_map))
}

fn grant_to_api_key_grant_dto(
    grant: &PermissionGrant,
    ws_map: &HashMap<WorkspaceId, (String, String)>,
    project_map: &HashMap<ProjectId, (String, String)>,
) -> ApiKeyGrantDto {
    let role = match grant.role {
        ResourceRole::Viewer => "viewer".to_string(),
        ResourceRole::Editor => "editor".to_string(),
        ResourceRole::Admin => "admin".to_string(),
    };

    let workspace_slug = ws_map
        .get(&grant.workspace_id)
        .map(|(slug, _)| slug.clone())
        .unwrap_or_else(|| grant.workspace_id.0.to_string());

    if let Some(pid) = grant.project_id {
        let (project_slug, project_name) = project_map
            .get(&pid)
            .map(|(slug, name)| (slug.clone(), name.clone()))
            .unwrap_or_else(|| (pid.0.to_string(), pid.0.to_string()));

        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "project".to_string(),
            resource_label: project_name,
            workspace_slug,
            project_slug: Some(project_slug),
        };
    }

    if let Some(fid) = grant.folder_id {
        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "folder".to_string(),
            resource_label: format!("folder:{}", fid.0),
            workspace_slug,
            project_slug: None,
        };
    }

    if let Some(did) = grant.document_id {
        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "document".to_string(),
            resource_label: format!("document:{}", did.0),
            workspace_slug,
            project_slug: None,
        };
    }

    if let Some(bid) = grant.board_id {
        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "board".to_string(),
            resource_label: format!("board:{}", bid.0),
            workspace_slug,
            project_slug: None,
        };
    }

    let ws_label = ws_map
        .get(&grant.workspace_id)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| grant.workspace_id.0.to_string());

    ApiKeyGrantDto {
        id: grant.id.0,
        role,
        resource_kind: "workspace".to_string(),
        resource_label: ws_label,
        workspace_slug,
        project_slug: None,
    }
}

#[utoipa::path(
    get,
    path = "/v1/api-keys/{key_id}/grants",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    responses(
        (status = 200, description = "Grants belonging to this API key", body = Vec<ApiKeyGrantDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not the key owner"),
        (status = 404, description = "Key not found"),
    )
)]
pub(crate) async fn list_api_key_grants(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
) -> Result<Json<Vec<ApiKeyGrantDto>>, ApiError> {
    let user_id = match principal {
        AuthPrincipal::User(uid) => uid,
        AuthPrincipal::ApiKey(_) => {
            return Err(ApiError::Forbidden {
                message: "API keys cannot list grants".into(),
            });
        }
    };

    let key_id = ApiKeyId(params.key_id);
    let api_key_repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let key = api_key_repo
        .get_by_id(key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    if key.created_by_user_id != user_id {
        return Err(ApiError::Forbidden {
            message: "you can only view grants for API keys you own".into(),
        });
    }

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let grants = grant_repo
        .list_for_api_key(key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let (ws_map, project_map) = resolve_resource_labels(&state, &grants, user_id).await?;

    let dtos: Vec<ApiKeyGrantDto> = grants
        .iter()
        .map(|g| grant_to_api_key_grant_dto(g, &ws_map, &project_map))
        .collect();

    Ok(Json(dtos))
}

#[utoipa::path(
    delete,
    path = "/v1/api-keys/{key_id}/grants/{grant_id}",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(
        ("key_id" = uuid::Uuid, Path, description = "API key id"),
        ("grant_id" = uuid::Uuid, Path, description = "Grant id to revoke"),
    ),
    responses(
        (status = 204, description = "Grant revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not the key owner"),
        (status = 404, description = "Key or grant not found"),
    )
)]
pub(crate) async fn delete_api_key_grant(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<ApiKeyGrantPath>,
) -> Result<StatusCode, ApiError> {
    let user_id = match principal {
        AuthPrincipal::User(uid) => uid,
        AuthPrincipal::ApiKey(_) => {
            return Err(ApiError::Forbidden {
                message: "API keys cannot revoke grants".into(),
            });
        }
    };

    let key_id = ApiKeyId(params.key_id);
    let api_key_repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let key = api_key_repo
        .get_by_id(key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    if key.created_by_user_id != user_id {
        return Err(ApiError::Forbidden {
            message: "you can only revoke grants for API keys you own".into(),
        });
    }

    let grant_id = PermissionGrantId(params.grant_id);
    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let deleted = grant_repo
        .delete_for_api_key(grant_id, key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if !deleted {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
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
            group_id: None,
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

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let key = PgApiKeyRepo::revoke_for_user_in(&txn, user_id, key_id)
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::NotFound { .. } => ApiError::NotFound,
            atlas_domain::DomainError::Forbidden { message } => ApiError::Forbidden { message },
            other => ApiError::Internal {
                message: other.to_string(),
            },
        })?;

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: Actor::User(user_id),
            action: SecurityAction::ApiKeyRevoked,
            target_type: "api_key".to_string(),
            target_id: Some(key_id.0),
            metadata: serde_json::json!({ "key_type": key.type_.as_str() }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(StatusCode::NO_CONTENT)
}
