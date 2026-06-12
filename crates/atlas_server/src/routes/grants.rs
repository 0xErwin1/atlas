use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::{
    dtos::{CreateGrantRequest, GrantDto, GrantPrincipal},
    pagination::{Cursor, Page},
};
use atlas_domain::{
    entities::permissions::NewPermissionGrant,
    ids::{ApiKeyId, UserId},
    permissions::{Principal, ResourceRef, ResourceRole, authorize_share},
};

use crate::{
    authz::{
        Authorized, EditorMin,
        authorized::{ProjectRes, WorkspaceRes},
    },
    error::ApiError,
    persistence::repos::{ApiKeyRepo, MembershipRepo, PermissionGrantRepo, PgPermissionGrantRepo},
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

pub(crate) async fn create_project_grant(
    auth: Authorized<ProjectRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateGrantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let role_in_play = parse_role(&body.role)?;

    authorize_share(&auth.principal, auth.effective, role_in_play).map_err(|d| {
        ApiError::Forbidden {
            message: format!("{d:?}"),
        }
    })?;

    let (user_id, api_key_id) =
        parse_principal(&body.principal, &auth.workspace.id, &state).await?;

    let created_by_user_id = match &auth.principal {
        Principal::User(uid) => Some(*uid),
        Principal::ApiKey(_) => None,
    };
    let created_by_api_key_id = match &auth.principal {
        Principal::ApiKey(kid) => Some(*kid),
        Principal::User(_) => None,
    };

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };
    let grant = grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: auth.workspace.id,
            user_id,
            api_key_id,
            project_id: Some(auth.resource.0.id),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: role_in_play,
            created_by_user_id,
            created_by_api_key_id,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(grant_to_dto(&grant, &body.principal)),
    ))
}

pub(crate) async fn list_project_grants(
    auth: Authorized<ProjectRes, EditorMin>,
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

pub(crate) async fn delete_project_grant(
    auth: Authorized<ProjectRes, EditorMin>,
    State(state): State<AppState>,
    Path(params): Path<std::collections::HashMap<String, String>>,
) -> Result<StatusCode, ApiError> {
    let grant_id_str = params.get("grant_id").ok_or(ApiError::NotFound)?;
    let grant_uuid = grant_id_str
        .parse::<uuid::Uuid>()
        .map_err(|_| ApiError::NotFound)?;
    let grant_id = atlas_domain::entities::permissions::PermissionGrantId(grant_uuid);

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let existing = grant_repo
        .list_for_resource(
            auth.workspace.id,
            &ResourceRef::Project(auth.resource.0.id),
            None,
            1000,
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let target_grant = existing
        .into_iter()
        .find(|g| g.id.0 == grant_uuid)
        .ok_or(ApiError::NotFound)?;

    authorize_share(&auth.principal, auth.effective, target_grant.role).map_err(|d| {
        ApiError::Forbidden {
            message: format!("{d:?}"),
        }
    })?;

    grant_repo
        .delete(grant_id, auth.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn create_workspace_grant(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateGrantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let role_in_play = parse_role(&body.role)?;

    authorize_share(&auth.principal, auth.effective, role_in_play).map_err(|d| {
        ApiError::Forbidden {
            message: format!("{d:?}"),
        }
    })?;

    let (user_id, api_key_id) =
        parse_principal(&body.principal, &auth.workspace.id, &state).await?;

    let created_by_user_id = match &auth.principal {
        Principal::User(uid) => Some(*uid),
        Principal::ApiKey(_) => None,
    };
    let created_by_api_key_id = match &auth.principal {
        Principal::ApiKey(kid) => Some(*kid),
        Principal::User(_) => None,
    };

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };
    let grant = grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: auth.workspace.id,
            user_id,
            api_key_id,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: role_in_play,
            created_by_user_id,
            created_by_api_key_id,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(grant_to_dto(&grant, &body.principal)),
    ))
}

pub(crate) async fn list_workspace_grants(
    auth: Authorized<WorkspaceRes, EditorMin>,
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

pub(crate) async fn delete_workspace_grant(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Path(params): Path<std::collections::HashMap<String, String>>,
) -> Result<StatusCode, ApiError> {
    let grant_id_str = params.get("grant_id").ok_or(ApiError::NotFound)?;
    let grant_uuid = grant_id_str
        .parse::<uuid::Uuid>()
        .map_err(|_| ApiError::NotFound)?;
    let grant_id = atlas_domain::entities::permissions::PermissionGrantId(grant_uuid);

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let existing = grant_repo
        .list_for_resource(auth.workspace.id, &ResourceRef::Workspace, None, 1000)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let target_grant = existing
        .into_iter()
        .find(|g| g.id.0 == grant_uuid)
        .ok_or(ApiError::NotFound)?;

    authorize_share(&auth.principal, auth.effective, target_grant.role).map_err(|d| {
        ApiError::Forbidden {
            message: format!("{d:?}"),
        }
    })?;

    grant_repo
        .delete(grant_id, auth.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
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

async fn parse_principal(
    principal: &GrantPrincipal,
    workspace_id: &atlas_domain::ids::WorkspaceId,
    state: &AppState,
) -> Result<(Option<UserId>, Option<ApiKeyId>), ApiError> {
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
            Ok((Some(uid), None))
        }
        "api_key" => {
            let kid = ApiKeyId(principal.id);
            let api_key_repo = crate::persistence::repos::PgApiKeyRepo {
                conn: (*state.db).clone(),
            };
            let ctx =
                atlas_domain::WorkspaceCtx::new(*workspace_id, atlas_domain::Actor::ApiKey(kid));
            let keys = api_key_repo
                .list(&ctx)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;
            if !keys.iter().any(|k| k.id == kid) {
                return Err(ApiError::InvalidInput {
                    message: "api key does not belong to this workspace".into(),
                });
            }
            Ok((None, Some(kid)))
        }
        other => Err(ApiError::InvalidInput {
            message: format!("invalid principal type: {other}; expected 'user' or 'api_key'"),
        }),
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

fn grant_domain_to_dto(grant: &atlas_domain::entities::permissions::PermissionGrant) -> GrantDto {
    let (principal_type, principal_id) = if let Some(uid) = grant.user_id {
        ("user".to_string(), uid.0)
    } else if let Some(kid) = grant.api_key_id {
        ("api_key".to_string(), kid.0)
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
