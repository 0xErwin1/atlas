use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::{
    dtos::{CreateProjectRequest, ProjectDto, UpdateProjectRequest},
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::permissions::NewPermissionGrant,
    entities::workspace_core::{NewProject, UpdateProject},
    permissions::{Principal, ResourceRole, ShareDenied, Visibility, VisibilityRole, authorize_share},
};

use crate::{
    authz::{Authorized, EditorMin, ViewerMin, WorkspaceMember, authorized::ProjectRes},
    error::ApiError,
    persistence::repos::{PermissionGrantRepo, PgPermissionGrantRepo, PgProjectRepo, ProjectRepo},
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/projects",
    tag = "projects",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateProjectRequest,
    responses(
        (status = 201, description = "Project created", body = ProjectDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn create_project(
    auth: Authorized<crate::authz::authorized::WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let visibility = parse_visibility(body.visibility.as_deref(), body.visibility_role.as_deref())?;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgProjectRepo {
        conn: (*state.db).clone(),
    };

    let project = repo
        .create(
            &ctx,
            NewProject {
                name: body.name,
                slug: body.slug,
                task_prefix: body.task_prefix,
                visibility: visibility.clone(),
            },
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let creator_role = match auth.effective {
        r if r >= ResourceRole::Admin => ResourceRole::Admin,
        r => r,
    };
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
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: auth.workspace.id,
            user_id: created_by_user_id,
            api_key_id: created_by_api_key_id,
            project_id: Some(project.id),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: creator_role,
            created_by_user_id,
            created_by_api_key_id,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok((StatusCode::CREATED, Json(project_to_dto(&project))))
}

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/projects",
    tag = "projects",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated project list"),
        (status = 401, description = "Unauthenticated"),
    )
)]
pub(crate) async fn list_projects(
    member: WorkspaceMember,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ProjectDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let principal = workspace_member_to_principal(&member);
    let actor = member_to_actor(&member);
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgProjectRepo {
        conn: (*state.db).clone(),
    };

    let mut items = repo
        .list_visible(&ctx, &principal, after_id, limit + 1)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let has_more = items.len() > limit as usize;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        items.last().map(|p| Cursor(p.id.0))
    } else {
        None
    };

    let dtos: Vec<ProjectDto> = items.iter().map(project_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/projects/{project_slug}",
    tag = "projects",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    responses(
        (status = 200, description = "Project details", body = ProjectDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found"),
    )
)]
pub(crate) async fn get_project(
    auth: Authorized<ProjectRes, ViewerMin>,
    State(_state): State<AppState>,
) -> Result<Json<ProjectDto>, ApiError> {
    Ok(Json(project_to_dto(&auth.resource.0)))
}

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/projects/{project_slug}",
    tag = "projects",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    request_body = UpdateProjectRequest,
    responses(
        (status = 200, description = "Project updated", body = ProjectDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found"),
    )
)]
pub(crate) async fn update_project(
    auth: Authorized<ProjectRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectDto>, ApiError> {
    let new_visibility = if body.visibility.is_some() || body.visibility_role.is_some() {
        let vis = parse_visibility(body.visibility.as_deref(), body.visibility_role.as_deref())?;

        let role_in_play = match &vis {
            Visibility::Workspace(r) | Visibility::Public(r) => match r {
                VisibilityRole::Viewer => ResourceRole::Viewer,
                VisibilityRole::Editor => ResourceRole::Editor,
            },
            Visibility::Private => ResourceRole::Viewer,
        };

        authorize_share(&auth.principal, auth.effective, role_in_play)
            .map_err(share_denied_to_api_error)?;

        Some(vis)
    } else {
        None
    };

    let update = UpdateProject {
        name: body.name,
        visibility: new_visibility,
    };

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgProjectRepo {
        conn: (*state.db).clone(),
    };

    let updated = repo
        .update(&ctx, auth.resource.0.id, update)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(project_to_dto(&updated)))
}

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/projects/{project_slug}",
    tag = "projects",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    responses(
        (status = 204, description = "Project deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found"),
    )
)]
pub(crate) async fn delete_project(
    auth: Authorized<ProjectRes, EditorMin>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgProjectRepo {
        conn: (*state.db).clone(),
    };

    repo.soft_delete(&ctx, auth.resource.0.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

fn project_to_dto(p: &atlas_domain::entities::workspace_core::Project) -> ProjectDto {
    let (vis_str, vis_role_str) = match &p.visibility {
        Visibility::Private => ("private".to_string(), None),
        Visibility::Workspace(r) => ("workspace".to_string(), Some(vis_role_str(r))),
        Visibility::Public(r) => ("public".to_string(), Some(vis_role_str(r))),
    };
    ProjectDto {
        id: p.id.0,
        workspace_id: p.workspace_id.0,
        name: p.name.clone(),
        slug: p.slug.clone(),
        task_prefix: p.task_prefix.clone(),
        visibility: vis_str,
        visibility_role: vis_role_str,
        created_at: p.created_at,
        updated_at: p.updated_at,
    }
}

fn vis_role_str(r: &VisibilityRole) -> String {
    match r {
        VisibilityRole::Viewer => "viewer".to_string(),
        VisibilityRole::Editor => "editor".to_string(),
    }
}

fn parse_visibility(
    visibility: Option<&str>,
    visibility_role: Option<&str>,
) -> Result<Visibility, ApiError> {
    let role = match visibility_role.unwrap_or("editor") {
        "viewer" => VisibilityRole::Viewer,
        "editor" => VisibilityRole::Editor,
        other => {
            return Err(ApiError::InvalidInput {
                message: format!("invalid visibility_role: {other}; expected 'viewer' or 'editor'"),
            });
        }
    };
    match visibility.unwrap_or("workspace") {
        "private" => Ok(Visibility::Private),
        "workspace" => Ok(Visibility::Workspace(role)),
        "public" => Err(ApiError::InvalidInput {
            message: "public visibility is not yet supported".into(),
        }),
        other => Err(ApiError::InvalidInput {
            message: format!("invalid visibility: {other}; expected 'private' or 'workspace'"),
        }),
    }
}

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    }
}

fn workspace_member_to_principal(member: &WorkspaceMember) -> Principal {
    if let Some(user) = &member.user {
        Principal::User(user.id)
    } else if let Some(kid) = member.api_key_id {
        Principal::ApiKey(kid)
    } else {
        Principal::ApiKey(atlas_domain::ids::ApiKeyId::new())
    }
}

fn member_to_actor(member: &WorkspaceMember) -> atlas_domain::Actor {
    if let Some(user) = &member.user {
        atlas_domain::Actor::User(user.id)
    } else if let Some(kid) = member.api_key_id {
        atlas_domain::Actor::ApiKey(kid)
    } else {
        atlas_domain::Actor::User(atlas_domain::ids::UserId::new())
    }
}

fn share_denied_to_api_error(_: ShareDenied) -> ApiError {
    ApiError::Forbidden {
        message: "insufficient permissions to manage grants".into(),
    }
}
