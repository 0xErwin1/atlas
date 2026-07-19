use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use sea_orm::TransactionTrait;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct ProjectGrantPath {
    #[allow(dead_code)]
    pub(crate) ws: String,
    #[allow(dead_code)]
    pub(crate) project_slug: String,
    pub(crate) grant_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct WorkspaceGrantPath {
    #[allow(dead_code)]
    pub(crate) ws: String,
    pub(crate) grant_id: uuid::Uuid,
}

use atlas_api::{
    dtos::{CreateGrantRequest, GrantDto, GrantPrincipal},
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor,
    entities::{
        permissions::NewPermissionGrant,
        security_audit::{NewSecurityAuditEvent, SecurityAction},
    },
    ids::{ApiKeyId, GroupId, UserId},
    permissions::{
        Principal, ResourceRef, ResourceRole, ShareDenied, authorize_grant_target, authorize_share,
    },
};

use crate::{
    authz::{
        Authorized, EditorMin, GrantsRead,
        authorized::{ProjectRes, WorkspaceRes},
    },
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, MembershipRepo, PermissionGrantRepo, PgGroupRepo, PgPermissionGrantRepo,
        PgSecurityAuditRepo,
    },
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/projects/{project_slug}/grants",
    tag = "grants",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    request_body = CreateGrantRequest,
    responses(
        (status = 201, description = "Grant created", body = GrantDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions or guardrail violation"),
    )
)]
pub(crate) async fn create_project_grant(
    auth: Authorized<ProjectRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateGrantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let role_in_play = parse_role(&body.role)?;

    authorize_share(&auth.principal, auth.effective, role_in_play)
        .map_err(share_denied_to_api_error)?;

    let (user_id, api_key_id, group_id) =
        parse_principal(&body.principal, &auth.workspace.id, &auth.principal, &state).await?;

    authorize_grant_target(
        &target_principal(user_id, api_key_id, group_id),
        role_in_play,
    )
    .map_err(share_denied_to_api_error)?;

    let created_by_user_id = match &auth.principal {
        Principal::User(uid) => Some(*uid),
        Principal::ApiKey(_) | Principal::Group(_) => None,
    };
    let created_by_api_key_id = match &auth.principal {
        Principal::ApiKey(kid) => Some(*kid),
        Principal::User(_) | Principal::Group(_) => None,
    };

    let actor = match &auth.principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => {
            return Err(ApiError::Forbidden {
                message: "groups cannot be grant actors".into(),
            });
        }
    };

    let new_grant = NewPermissionGrant {
        workspace_id: auth.workspace.id,
        user_id,
        api_key_id,
        group_id,
        project_id: Some(auth.resource.0.id),
        folder_id: None,
        document_id: None,
        board_id: None,
        role: role_in_play,
        created_by_user_id,
        created_by_api_key_id,
    };

    let (grantee_type, grantee_id) =
        grantee_fields(new_grant.user_id, new_grant.api_key_id, new_grant.group_id);

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let grant = PgPermissionGrantRepo::upsert_in(&txn, new_grant)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    // The audit row and the upsert commit or roll back together.
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(auth.workspace.id),
            actor,
            action: SecurityAction::GrantCreated,
            target_type: "grant".to_string(),
            target_id: Some(grant.id.0),
            metadata: serde_json::json!({
                "resource_type": "project",
                "resource_id": auth.resource.0.id.0,
                "role": role_str(role_in_play),
                "grantee_type": grantee_type,
                "grantee_id": grantee_id,
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

    Ok((
        StatusCode::CREATED,
        Json(grant_to_dto(&grant, &body.principal)),
    ))
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/projects/{project_slug}/grants",
    tag = "grants",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated grant list", body = Page<GrantDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_project_grants(
    auth: Authorized<ProjectRes, EditorMin, GrantsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<GrantDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };
    let resource = ResourceRef::Project(auth.resource.0.id);
    let mut grants = grant_repo
        .list_for_resource(auth.workspace.id, &resource, after_id, limit + 1)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let has_more = grants.len() > limit as usize;
    if has_more {
        grants.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        grants.last().map(|g| Cursor(g.id.0))
    } else {
        None
    };

    let dtos: Vec<GrantDto> = grants.iter().map(grant_domain_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}",
    tag = "grants",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
        ("grant_id" = uuid::Uuid, Path, description = "Grant id"),
    ),
    responses(
        (status = 204, description = "Grant deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Grant not found"),
    )
)]
pub(crate) async fn delete_project_grant(
    auth: Authorized<ProjectRes, EditorMin>,
    State(state): State<AppState>,
    Path(params): Path<ProjectGrantPath>,
) -> Result<StatusCode, ApiError> {
    let grant_uuid = params.grant_id;
    let grant_id = atlas_domain::entities::permissions::PermissionGrantId(grant_uuid);

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let target_grant = grant_repo
        .find_by_id(
            auth.workspace.id,
            &ResourceRef::Project(auth.resource.0.id),
            atlas_domain::entities::permissions::PermissionGrantId(grant_uuid),
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    authorize_share(&auth.principal, auth.effective, target_grant.role)
        .map_err(share_denied_to_api_error)?;

    let actor = match &auth.principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => {
            return Err(ApiError::Forbidden {
                message: "groups cannot be grant actors".into(),
            });
        }
    };

    let (grantee_type, grantee_id) = grantee_fields(
        target_grant.user_id,
        target_grant.api_key_id,
        target_grant.group_id,
    );

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    PgPermissionGrantRepo::delete_in(&txn, grant_id, auth.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    // The audit row and the delete commit or roll back together.
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(auth.workspace.id),
            actor,
            action: SecurityAction::GrantRevoked,
            target_type: "grant".to_string(),
            target_id: Some(grant_uuid),
            metadata: serde_json::json!({
                "resource_type": "project",
                "resource_id": auth.resource.0.id.0,
                "grantee_type": grantee_type,
                "grantee_id": grantee_id,
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

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/grants",
    tag = "grants",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateGrantRequest,
    responses(
        (status = 201, description = "Workspace grant created", body = GrantDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions or guardrail violation"),
    )
)]
pub(crate) async fn create_workspace_grant(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateGrantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let role_in_play = parse_role(&body.role)?;

    authorize_share(&auth.principal, auth.effective, role_in_play)
        .map_err(share_denied_to_api_error)?;

    let (user_id, api_key_id, group_id) =
        parse_principal(&body.principal, &auth.workspace.id, &auth.principal, &state).await?;

    authorize_grant_target(
        &target_principal(user_id, api_key_id, group_id),
        role_in_play,
    )
    .map_err(share_denied_to_api_error)?;

    let created_by_user_id = match &auth.principal {
        Principal::User(uid) => Some(*uid),
        Principal::ApiKey(_) | Principal::Group(_) => None,
    };
    let created_by_api_key_id = match &auth.principal {
        Principal::ApiKey(kid) => Some(*kid),
        Principal::User(_) | Principal::Group(_) => None,
    };

    let actor = match &auth.principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => {
            return Err(ApiError::Forbidden {
                message: "groups cannot be grant actors".into(),
            });
        }
    };

    let new_grant = NewPermissionGrant {
        workspace_id: auth.workspace.id,
        user_id,
        api_key_id,
        group_id,
        project_id: None,
        folder_id: None,
        document_id: None,
        board_id: None,
        role: role_in_play,
        created_by_user_id,
        created_by_api_key_id,
    };

    let (grantee_type, grantee_id) =
        grantee_fields(new_grant.user_id, new_grant.api_key_id, new_grant.group_id);

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let grant = PgPermissionGrantRepo::upsert_in(&txn, new_grant)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    // The audit row and the upsert commit or roll back together.
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(auth.workspace.id),
            actor,
            action: SecurityAction::GrantCreated,
            target_type: "grant".to_string(),
            target_id: Some(grant.id.0),
            metadata: serde_json::json!({
                "resource_type": "workspace",
                "resource_id": auth.workspace.id.0,
                "role": role_str(role_in_play),
                "grantee_type": grantee_type,
                "grantee_id": grantee_id,
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

    Ok((
        StatusCode::CREATED,
        Json(grant_to_dto(&grant, &body.principal)),
    ))
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/grants",
    tag = "grants",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated workspace grant list", body = Page<GrantDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_workspace_grants(
    auth: Authorized<WorkspaceRes, EditorMin, GrantsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<GrantDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };
    let mut grants = grant_repo
        .list_for_resource(
            auth.workspace.id,
            &ResourceRef::Workspace,
            after_id,
            limit + 1,
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let has_more = grants.len() > limit as usize;
    if has_more {
        grants.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        grants.last().map(|g| Cursor(g.id.0))
    } else {
        None
    };

    let dtos: Vec<GrantDto> = grants.iter().map(grant_domain_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/grants/{grant_id}",
    tag = "grants",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("grant_id" = uuid::Uuid, Path, description = "Grant id"),
    ),
    responses(
        (status = 204, description = "Workspace grant deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Grant not found"),
    )
)]
pub(crate) async fn delete_workspace_grant(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Path(params): Path<WorkspaceGrantPath>,
) -> Result<StatusCode, ApiError> {
    let grant_uuid = params.grant_id;
    let grant_id = atlas_domain::entities::permissions::PermissionGrantId(grant_uuid);

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let target_grant = grant_repo
        .find_by_id(
            auth.workspace.id,
            &ResourceRef::Workspace,
            atlas_domain::entities::permissions::PermissionGrantId(grant_uuid),
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    authorize_share(&auth.principal, auth.effective, target_grant.role)
        .map_err(share_denied_to_api_error)?;

    let actor = match &auth.principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => {
            return Err(ApiError::Forbidden {
                message: "groups cannot be grant actors".into(),
            });
        }
    };

    let (grantee_type, grantee_id) = grantee_fields(
        target_grant.user_id,
        target_grant.api_key_id,
        target_grant.group_id,
    );

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    PgPermissionGrantRepo::delete_in(&txn, grant_id, auth.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    // The audit row and the delete commit or roll back together.
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(auth.workspace.id),
            actor,
            action: SecurityAction::GrantRevoked,
            target_type: "grant".to_string(),
            target_id: Some(grant_uuid),
            metadata: serde_json::json!({
                "resource_type": "workspace",
                "resource_id": auth.workspace.id.0,
                "grantee_type": grantee_type,
                "grantee_id": grantee_id,
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

    Ok(StatusCode::NO_CONTENT)
}

fn grantee_fields(
    user_id: Option<UserId>,
    api_key_id: Option<ApiKeyId>,
    group_id: Option<GroupId>,
) -> (&'static str, uuid::Uuid) {
    match (user_id, api_key_id, group_id) {
        (Some(uid), _, _) => ("user", uid.0),
        (_, Some(kid), _) => ("api_key", kid.0),
        (_, _, Some(gid)) => ("group", gid.0),
        (None, None, None) => ("unknown", uuid::Uuid::nil()),
    }
}

fn parse_role(role: &str) -> Result<ResourceRole, ApiError> {
    match role {
        "viewer" => Ok(ResourceRole::Viewer),
        "editor" => Ok(ResourceRole::Editor),
        "admin" => Ok(ResourceRole::Admin),
        other => Err(ApiError::InvalidInput {
            message: format!("invalid role: {other}; expected 'viewer', 'editor', or 'admin'"),
        }),
    }
}

/// Resolves and validates a grant principal.
///
/// For `user` principals, membership in the target workspace is required.
/// For `api_key` principals, the key is looked up by id (workspace-independent)
/// and the caller must own it — `created_by_user_id` must match the acting user.
/// This ownership requirement means a user can only grant access for their own keys;
/// cross-user key grants are out of scope and would require an explicit delegation model.
/// For `group` principals, the group must belong to the target workspace and must
/// not be soft-deleted.
async fn parse_principal(
    principal: &GrantPrincipal,
    workspace_id: &atlas_domain::ids::WorkspaceId,
    caller: &Principal,
    state: &AppState,
) -> Result<(Option<UserId>, Option<ApiKeyId>, Option<GroupId>), ApiError> {
    match principal.r#type.as_str() {
        "user" => {
            let uid = UserId(principal.id);
            let membership_repo = crate::persistence::repos::PgMembershipRepo {
                conn: (*state.db).clone(),
            };
            let ctx =
                atlas_domain::WorkspaceCtx::new(*workspace_id, atlas_domain::Actor::User(uid));
            let m = membership_repo
                .find(&ctx, uid)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;
            if m.is_none() {
                return Err(ApiError::InvalidInput {
                    message: "user is not a member of this workspace".into(),
                });
            }
            Ok((Some(uid), None, None))
        }
        "api_key" => {
            let kid = ApiKeyId(principal.id);
            let api_key_repo = crate::persistence::repos::PgApiKeyRepo {
                conn: (*state.db).clone(),
            };

            let key = api_key_repo
                .get_by_id(kid)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?
                .ok_or_else(|| ApiError::InvalidInput {
                    message: "api key not found or has been revoked".into(),
                })?;

            if key.revoked_at.is_some() {
                return Err(ApiError::InvalidInput {
                    message: "api key not found or has been revoked".into(),
                });
            }

            let caller_user_id = match caller {
                Principal::User(uid) => *uid,
                Principal::ApiKey(_) | Principal::Group(_) => {
                    return Err(ApiError::Forbidden {
                        message: "agents cannot manage grants".into(),
                    });
                }
            };

            if key.created_by_user_id != caller_user_id {
                return Err(ApiError::InvalidInput {
                    message: "you can only grant API keys you own".into(),
                });
            }

            Ok((None, Some(kid), None))
        }
        "group" => {
            let gid = GroupId(principal.id);
            let group_repo = PgGroupRepo {
                conn: (*state.db).clone(),
            };

            use atlas_domain::ports::group_repo::GroupRepo as GroupRepoTrait;
            let group = group_repo
                .get(gid, *workspace_id)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?
                .ok_or_else(|| ApiError::InvalidInput {
                    message: "group not found in this workspace or has been deleted".into(),
                })?;

            if group.deleted_at.is_some() {
                return Err(ApiError::InvalidInput {
                    message: "group not found in this workspace or has been deleted".into(),
                });
            }

            Ok((None, None, Some(gid)))
        }
        other => Err(ApiError::InvalidInput {
            message: format!(
                "invalid principal type: {other}; expected 'user', 'api_key', or 'group'"
            ),
        }),
    }
}

fn target_principal(
    user_id: Option<UserId>,
    api_key_id: Option<ApiKeyId>,
    group_id: Option<GroupId>,
) -> Principal {
    match (user_id, api_key_id, group_id) {
        (_, Some(kid), _) => Principal::ApiKey(kid),
        (Some(uid), None, _) => Principal::User(uid),
        (None, None, Some(gid)) => Principal::Group(gid),
        (None, None, None) => Principal::User(UserId(uuid::Uuid::nil())),
    }
}

fn role_str(r: ResourceRole) -> String {
    match r {
        ResourceRole::Viewer => "viewer".to_string(),
        ResourceRole::Editor => "editor".to_string(),
        ResourceRole::Admin => "admin".to_string(),
    }
}

fn grant_to_dto(
    grant: &atlas_domain::entities::permissions::PermissionGrant,
    principal: &GrantPrincipal,
) -> GrantDto {
    GrantDto {
        id: grant.id.0,
        principal: GrantPrincipal {
            r#type: principal.r#type.clone(),
            id: principal.id,
        },
        role: role_str(grant.role),
        created_at: grant.created_at,
    }
}

fn share_denied_to_api_error(denied: ShareDenied) -> ApiError {
    let message = match denied {
        ShareDenied::AgentCannotBeAdmin => {
            "Agents and scripts cannot be granted the Admin role.".to_string()
        }
        ShareDenied::AgentsNeverManageGrants
        | ShareDenied::RoleExceedsGrantors
        | ShareDenied::InsufficientRoleToShare => {
            "insufficient permissions to manage grants".to_string()
        }
    };

    ApiError::Forbidden { message }
}

fn grant_domain_to_dto(grant: &atlas_domain::entities::permissions::PermissionGrant) -> GrantDto {
    let (principal_type, principal_id) = if let Some(uid) = grant.user_id {
        ("user".to_string(), uid.0)
    } else if let Some(kid) = grant.api_key_id {
        ("api_key".to_string(), kid.0)
    } else if let Some(gid) = grant.group_id {
        ("group".to_string(), gid.0)
    } else {
        ("unknown".to_string(), uuid::Uuid::nil())
    };

    GrantDto {
        id: grant.id.0,
        principal: GrantPrincipal {
            r#type: principal_type,
            id: principal_id,
        },
        role: role_str(grant.role),
        created_at: grant.created_at,
    }
}
