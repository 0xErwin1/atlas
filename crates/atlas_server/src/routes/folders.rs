use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use atlas_api::{
    dtos::folders::{CreateFolderRequest, FolderDto, MoveFolderRequest, RenameFolderRequest},
    pagination::Page,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::workspace_core::{Folder, NewFolder},
    ids::FolderId,
    permissions::Principal,
};

use crate::{
    authz::{
        Authorized, EditorMin, FolderRes, MinRole, ViewerMin, authorize_folder_destination,
        authorized::ProjectRes, resolve_folder_ancestry,
    },
    error::ApiError,
    persistence::repos::{FolderRepo, PgFolderRepo},
    routes::validation::validate_name,
    state::AppState,
};

fn folder_to_dto(f: Folder) -> FolderDto {
    FolderDto {
        id: f.id.0,
        workspace_id: f.workspace_id.0,
        project_id: f.project_id.map(|id| id.0),
        parent_folder_id: f.parent_folder_id.map(|id| id.0),
        name: f.name,
        created_at: f.created_at,
        updated_at: f.updated_at,
    }
}

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    }
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/projects/{project_slug}/folders
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/projects/{project_slug}/folders",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    request_body = CreateFolderRequest,
    responses(
        (status = 201, description = "Folder created", body = FolderDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn create_folder(
    auth: Authorized<ProjectRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateFolderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let project_id = auth.resource.0.id;

    if let Some(parent_uuid) = body.parent_folder_id {
        let parent_fid = FolderId(parent_uuid);

        let ancestry = resolve_folder_ancestry(&state.db, auth.workspace.id, parent_fid).await?;

        if ancestry.is_empty() {
            return Err(ApiError::InvalidInput {
                message: "parent folder does not exist in this workspace".into(),
            });
        }

        let folder_project = ancestry.last().and_then(|f| f.project_id);
        if folder_project != Some(project_id) {
            return Err(ApiError::InvalidInput {
                message: "parent folder does not belong to this project".into(),
            });
        }
    }

    let repo = PgFolderRepo {
        conn: (*state.db).clone(),
    };

    let folder = repo
        .create(
            &ctx,
            NewFolder {
                project_id: Some(project_id),
                parent_folder_id: body.parent_folder_id.map(FolderId),
                name: body.name.trim().to_string(),
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(folder_to_dto(folder))))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/projects/{project_slug}/folders
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/projects/{project_slug}/folders",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    responses(
        (status = 200, description = "Folder list", body = Page<FolderDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_folders(
    auth: Authorized<ProjectRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Page<FolderDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let project_id = auth.resource.0.id;

    let repo = PgFolderRepo {
        conn: (*state.db).clone(),
    };

    let all = repo.list_all(&ctx).await.map_err(ApiError::Domain)?;

    let items: Vec<FolderDto> = all
        .into_iter()
        .filter(|f| f.project_id == Some(project_id))
        .map(folder_to_dto)
        .collect();

    Ok(Json(Page {
        items,
        next_cursor: None,
        has_more: false,
    }))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/folders/{folder_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/folders/{folder_id}",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("folder_id" = uuid::Uuid, Path, description = "Folder ID"),
    ),
    responses(
        (status = 200, description = "Folder", body = FolderDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Not found"),
        (status = 422, description = "Invalid UUID"),
    )
)]
pub(crate) async fn get_folder(
    auth: Authorized<FolderRes, ViewerMin>,
) -> Result<Json<FolderDto>, ApiError> {
    Ok(Json(folder_to_dto(auth.resource.0)))
}

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/folders/{folder_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/folders/{folder_id}",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("folder_id" = uuid::Uuid, Path, description = "Folder ID"),
    ),
    request_body = RenameFolderRequest,
    responses(
        (status = 200, description = "Updated folder", body = FolderDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn rename_folder(
    auth: Authorized<FolderRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<RenameFolderRequest>,
) -> Result<Json<FolderDto>, ApiError> {
    validate_name("name", &body.name)?;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let folder_id = auth.resource.0.id;
    let trimmed = body.name.trim().to_string();

    let repo = PgFolderRepo {
        conn: (*state.db).clone(),
    };

    repo.rename(&ctx, folder_id, trimmed)
        .await
        .map_err(ApiError::Domain)?;

    let updated = repo
        .find(&ctx, folder_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(folder_to_dto(updated)))
}

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/folders/{folder_id}/move
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/folders/{folder_id}/move",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("folder_id" = uuid::Uuid, Path, description = "Folder ID"),
    ),
    request_body = MoveFolderRequest,
    responses(
        (status = 200, description = "Moved folder", body = FolderDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 422, description = "Cycle detected or invalid parent"),
    )
)]
pub(crate) async fn move_folder(
    auth: Authorized<FolderRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<MoveFolderRequest>,
) -> Result<Json<FolderDto>, ApiError> {
    let folder_id = auth.resource.0.id;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    if let Some(new_parent_uuid) = body.parent_folder_id {
        let new_parent = FolderId(new_parent_uuid);

        if new_parent == folder_id {
            return Err(ApiError::InvalidInput {
                message: "a folder cannot be moved into itself".into(),
            });
        }

        authorize_folder_destination(
            &state.db,
            &auth.principal,
            auth.membership.clone(),
            &auth.workspace,
            new_parent,
            EditorMin::ROLE,
        )
        .await?;

        let ancestry = resolve_folder_ancestry(&state.db, auth.workspace.id, new_parent).await?;
        let is_descendant = ancestry.iter().any(|f| f.id == folder_id);
        if is_descendant {
            return Err(ApiError::InvalidInput {
                message: "moving a folder into one of its descendants would create a cycle".into(),
            });
        }
    }

    let repo = PgFolderRepo {
        conn: (*state.db).clone(),
    };

    repo.move_to(&ctx, folder_id, body.parent_folder_id.map(FolderId))
        .await
        .map_err(ApiError::Domain)?;

    let updated = repo
        .find(&ctx, folder_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(folder_to_dto(updated)))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/folders/{folder_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/folders/{folder_id}",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("folder_id" = uuid::Uuid, Path, description = "Folder ID"),
    ),
    responses(
        (status = 204, description = "Folder deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Not found"),
    )
)]
pub(crate) async fn delete_folder(
    auth: Authorized<FolderRes, EditorMin>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let folder_id = auth.resource.0.id;

    let repo = PgFolderRepo {
        conn: (*state.db).clone(),
    };

    repo.soft_delete(&ctx, folder_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
