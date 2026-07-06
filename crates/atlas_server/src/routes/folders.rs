use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::{
    dtos::folders::{
        CopyFolderRequest, CreateFolderRequest, FolderDto, MoveFolderRequest, RenameFolderRequest,
    },
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::workspace_core::{Folder, NewFolder},
    ids::FolderId,
    permissions::Principal,
};

use crate::{
    authz::{
        Authorized, EditorMin, FolderRes, FoldersCreate, FoldersDelete, FoldersRead, FoldersUpdate,
        MinRole, ViewerMin, authorize_folder_destination, authorized::ProjectRes,
        resolve_folder_ancestry,
    },
    error::ApiError,
    persistence::repos::{DocumentRepo, FolderRepo, PgDocumentRepo, PgFolderRepo},
    routes::{documents::copy_document_into, validation::validate_name},
    services::DocumentService,
    state::AppState,
};

/// Maximum folder nesting depth honored by the recursive copy. Mirrors the
/// 32-level bound used by `resolve_folder_ancestry` so a pathological or cyclic
/// tree cannot recurse without limit.
const MAX_COPY_DEPTH: usize = 32;

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
        Principal::Group(_) => Actor::User(atlas_domain::ids::UserId(uuid::Uuid::nil())),
    }
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/projects/{project_slug}/folders
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/projects/{project_slug}/folders",
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
    auth: Authorized<ProjectRes, EditorMin, FoldersCreate>,
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
// GET /api/workspaces/{ws}/projects/{project_slug}/folders
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/projects/{project_slug}/folders",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated folder list", body = Page<FolderDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_folders(
    auth: Authorized<ProjectRes, ViewerMin, FoldersRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<FolderDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q
        .cursor
        .as_deref()
        .and_then(Cursor::decode)
        .map(|c| FolderId(c.0));

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let project_id = auth.resource.0.id;

    let repo = PgFolderRepo {
        conn: (*state.db).clone(),
    };

    let mut folders = repo
        .list_paginated_by_project(&ctx, project_id, after_id, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = folders.len() > limit as usize;
    if has_more {
        folders.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        folders.last().map(|f| Cursor(f.id.0))
    } else {
        None
    };

    let items: Vec<FolderDto> = folders.into_iter().map(folder_to_dto).collect();

    Ok(Json(Page::new(items, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/folders/{folder_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/folders/{folder_id}",
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
    auth: Authorized<FolderRes, ViewerMin, FoldersRead>,
) -> Result<Json<FolderDto>, ApiError> {
    Ok(Json(folder_to_dto(auth.resource.0)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/folders/{folder_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/folders/{folder_id}",
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
    auth: Authorized<FolderRes, EditorMin, FoldersUpdate>,
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
// PATCH /api/workspaces/{ws}/folders/{folder_id}/move
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/folders/{folder_id}/move",
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
    auth: Authorized<FolderRes, EditorMin, FoldersUpdate>,
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
// POST /api/workspaces/{ws}/folders/{folder_id}/copy
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/folders/{folder_id}/copy",
    tag = "folders",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("folder_id" = uuid::Uuid, Path, description = "Source folder ID"),
    ),
    request_body = CopyFolderRequest,
    responses(
        (status = 201, description = "Folder copied", body = FolderDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Not found"),
    )
)]
pub(crate) async fn copy_folder(
    auth: Authorized<FolderRes, EditorMin, FoldersCreate>,
    State(state): State<AppState>,
    Json(body): Json<CopyFolderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let source = auth.resource.0;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let parent_folder_id = match body.parent_folder_id {
        Some(pid) => Some(FolderId(pid)),
        None => source.parent_folder_id,
    };

    if let Some(pid) = body.parent_folder_id {
        authorize_folder_destination(
            &state.db,
            &auth.principal,
            auth.membership.clone(),
            &auth.workspace,
            FolderId(pid),
            EditorMin::ROLE,
        )
        .await?;
    }

    let folder_repo = PgFolderRepo {
        conn: (*state.db).clone(),
    };
    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);
    let doc_svc = state.document_service();

    let top_name = format!("{} (copy)", source.name);

    let new_top = folder_repo
        .create(
            &ctx,
            NewFolder {
                project_id: source.project_id,
                parent_folder_id,
                name: top_name,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    let copy_deps = CopyDeps {
        state: &state,
        ctx: &ctx,
        folder_repo: &folder_repo,
        doc_repo: &doc_repo,
        doc_svc: &doc_svc,
    };
    copy_folder_subtree(&copy_deps, &source, &new_top, 0).await?;

    Ok((StatusCode::CREATED, Json(folder_to_dto(new_top))))
}

/// Recursively recreates the documents and subfolders of `source` underneath the
/// already-created `dest` folder.
///
/// Descendant subfolders and documents preserve their original names verbatim
/// (only the top-level copy carries the " (copy)" suffix, applied by the caller).
/// Every created entity gets a fresh id and, for documents, a fresh slug and
/// first revision. Bounded by `MAX_COPY_DEPTH` to guard against cyclic trees.
/// Borrowed handles threaded through the recursive folder copy.
struct CopyDeps<'a> {
    state: &'a AppState,
    ctx: &'a WorkspaceCtx,
    folder_repo: &'a PgFolderRepo,
    doc_repo: &'a PgDocumentRepo,
    doc_svc: &'a DocumentService,
}

async fn copy_folder_subtree(
    deps: &CopyDeps<'_>,
    source: &Folder,
    dest: &Folder,
    depth: usize,
) -> Result<(), ApiError> {
    if depth >= MAX_COPY_DEPTH {
        return Err(ApiError::InvalidInput {
            message: "folder nesting is too deep to copy".to_string(),
        });
    }

    let documents = deps
        .doc_repo
        .list_in_folder(deps.ctx, source.id)
        .await
        .map_err(ApiError::Domain)?;

    for doc in &documents {
        copy_document_into(
            deps.state,
            deps.ctx,
            deps.doc_svc,
            doc,
            Some(dest.id),
            dest.project_id,
        )
        .await?;
    }

    let children = deps
        .folder_repo
        .list_children(deps.ctx, Some(source.id))
        .await
        .map_err(ApiError::Domain)?;

    for child in &children {
        let new_child = deps
            .folder_repo
            .create(
                deps.ctx,
                NewFolder {
                    project_id: child.project_id,
                    parent_folder_id: Some(dest.id),
                    name: child.name.clone(),
                },
            )
            .await
            .map_err(ApiError::Domain)?;

        Box::pin(copy_folder_subtree(deps, child, &new_child, depth + 1)).await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/folders/{folder_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/folders/{folder_id}",
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
    auth: Authorized<FolderRes, EditorMin, FoldersDelete>,
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
