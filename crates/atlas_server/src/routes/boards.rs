#![allow(clippy::indexing_slicing)]

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::{
    dtos::boards_tasks::{
        BoardDto, BoardSummaryDto, ColumnDto, CreateBoardRequest, CreateColumnRequest,
        MoveBoardRequest, UpdateBoardRequest, UpdateColumnRequest,
    },
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::{Board, BoardColumn, ColumnPatch, NewBoard, PositionBetween},
    ids::{BoardId, ColumnId, FolderId},
    permissions::Principal,
};

use crate::{
    authz::{
        Authorized, BoardRes, BoardsCreate, BoardsDelete, BoardsRead, BoardsUpdate, EditorMin,
        MinRole, ProjectRes, ViewerMin, authorize_folder_destination, resolve_folder_ancestry,
    },
    error::ApiError,
    persistence::repos::{BoardRepo, PgBoardRepo},
    routes::validation::{validate_name, validate_swatch},
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
pub(crate) struct ColumnPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    board_id: uuid::Uuid,
    column_id: uuid::Uuid,
}

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => Actor::User(atlas_domain::ids::UserId(uuid::Uuid::nil())),
    }
}

fn board_to_dto(b: Board) -> BoardDto {
    let (actor_type, actor_id) = match b.created_by {
        Actor::User(uid) => ("user".into(), uid.0),
        Actor::ApiKey(kid) => ("api_key".into(), kid.0),
    };
    BoardDto {
        id: b.id.0,
        workspace_id: b.workspace_id.0,
        project_id: b.project_id.0,
        folder_id: b.folder_id.map(|id| id.0),
        name: b.name,
        created_by: atlas_api::dtos::documents::ActorDto {
            r#type: actor_type,
            id: actor_id,
            display_name: None,
            key_type: None,
            account_status: None,
        },
        created_at: b.created_at,
        updated_at: b.updated_at,
    }
}

fn column_to_dto(c: BoardColumn) -> ColumnDto {
    ColumnDto {
        id: c.id.0,
        board_id: c.board_id.0,
        name: c.name,
        position_key: c.position_key,
        color: c.color,
        created_at: c.created_at,
        updated_at: c.updated_at,
    }
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/projects/{project_slug}/boards
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/projects/{project_slug}/boards",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    request_body = CreateBoardRequest,
    responses(
        (status = 201, description = "Board created", body = BoardDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found"),
    )
)]
pub(crate) async fn create_board(
    auth: Authorized<ProjectRes, EditorMin, BoardsCreate>,
    State(state): State<AppState>,
    Json(body): Json<CreateBoardRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let project_id = auth.resource.0.id;

    if let Some(folder_uuid) = body.folder_id {
        let ancestry =
            resolve_folder_ancestry(&state.db, auth.workspace.id, FolderId(folder_uuid)).await?;

        let folder_project = ancestry.last().and_then(|f| f.project_id);
        if folder_project != Some(project_id) {
            return Err(ApiError::InvalidInput {
                message: "target folder does not exist in this workspace".to_string(),
            });
        }
    }

    let repo = PgBoardRepo::new((*state.db).clone());

    let board = repo
        .create_board(
            &ctx,
            NewBoard {
                project_id,
                folder_id: body.folder_id.map(FolderId),
                name: body.name,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(board_to_dto(board))))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/projects/{project_slug}/boards
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/projects/{project_slug}/boards",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated board list", body = Page<BoardSummaryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_boards(
    auth: Authorized<ProjectRes, ViewerMin, BoardsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<BoardSummaryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgBoardRepo::new((*state.db).clone());

    let mut all = repo
        .list_boards(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);
    if let Some(id) = after_id
        && let Some(pos) = all.iter().position(|b| b.id.0 == id)
    {
        all = all.into_iter().skip(pos + 1).collect();
    }

    let has_more = all.len() > limit;
    if has_more {
        all.truncate(limit);
    }

    let next_cursor = if has_more {
        all.last().map(|b| Cursor(b.id.0))
    } else {
        None
    };

    let board_ids: Vec<BoardId> = all.iter().map(|b| b.id).collect();
    let task_counts: std::collections::HashMap<uuid::Uuid, i64> = repo
        .count_top_level_tasks_for_boards(&ctx, &board_ids)
        .await
        .map_err(ApiError::Domain)?
        .into_iter()
        .map(|(id, count)| (id.0, count))
        .collect();

    let dtos = all
        .into_iter()
        .map(|b| BoardSummaryDto {
            id: b.id.0,
            name: b.name,
            folder_id: b.folder_id.map(|id| id.0),
            task_count: task_counts.get(&b.id.0).copied().unwrap_or(0),
            created_at: b.created_at,
            updated_at: b.updated_at,
        })
        .collect();

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/boards/{board_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/boards/{board_id}",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    responses(
        (status = 200, description = "Board", body = BoardDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn get_board(
    auth: Authorized<BoardRes, ViewerMin, BoardsRead>,
) -> Result<Json<BoardDto>, ApiError> {
    Ok(Json(board_to_dto(auth.resource.0)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/boards/{board_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/boards/{board_id}",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    request_body = UpdateBoardRequest,
    responses(
        (status = 200, description = "Board updated", body = BoardDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn update_board(
    auth: Authorized<BoardRes, EditorMin, BoardsUpdate>,
    State(state): State<AppState>,
    Json(body): Json<UpdateBoardRequest>,
) -> Result<Json<BoardDto>, ApiError> {
    if let Some(ref name) = body.name {
        validate_name("name", name)?;
    }

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgBoardRepo::new((*state.db).clone());

    let board = if let Some(name) = body.name {
        repo.patch_board(&ctx, auth.resource.0.id, name)
            .await
            .map_err(ApiError::Domain)?
    } else {
        auth.resource.0
    };

    Ok(Json(board_to_dto(board)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/boards/{board_id}/move
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/boards/{board_id}/move",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    request_body = MoveBoardRequest,
    responses(
        (status = 200, description = "Board moved", body = BoardDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
        (status = 422, description = "Invalid destination folder"),
    )
)]
pub(crate) async fn move_board(
    auth: Authorized<BoardRes, EditorMin, BoardsUpdate>,
    State(state): State<AppState>,
    Json(body): Json<MoveBoardRequest>,
) -> Result<Json<BoardDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    if let Some(fid) = body.folder_id {
        authorize_folder_destination(
            &state.db,
            &auth.principal,
            auth.membership.clone(),
            &auth.workspace,
            FolderId(fid),
            EditorMin::ROLE,
        )
        .await?;
    }

    let repo = PgBoardRepo::new((*state.db).clone());

    let board = repo
        .move_board(&ctx, auth.resource.0.id, body.folder_id.map(FolderId))
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(board_to_dto(board)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/boards/{board_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/boards/{board_id}",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    responses(
        (status = 204, description = "Board deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn delete_board(
    auth: Authorized<BoardRes, EditorMin, BoardsDelete>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgBoardRepo::new((*state.db).clone());

    repo.soft_delete_board(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/boards/{board_id}/columns
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/boards/{board_id}/columns",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    request_body = CreateColumnRequest,
    responses(
        (status = 201, description = "Column created", body = ColumnDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
        (status = 409, description = "Position exhausted — retry"),
    )
)]
pub(crate) async fn create_column(
    auth: Authorized<BoardRes, EditorMin, BoardsUpdate>,
    State(state): State<AppState>,
    Json(body): Json<CreateColumnRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;

    if let Some(ref color) = body.color {
        validate_swatch("color", color)?;
    }

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgBoardRepo::new((*state.db).clone());

    let col = repo
        .add_column(
            &ctx,
            auth.resource.0.id,
            body.name,
            body.color,
            PositionBetween {
                before: body.before,
                after: body.after,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(column_to_dto(col))))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/boards/{board_id}/columns
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/boards/{board_id}/columns",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    responses(
        (status = 200, description = "Column list", body = Vec<ColumnDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn list_columns(
    auth: Authorized<BoardRes, ViewerMin, BoardsRead>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ColumnDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgBoardRepo::new((*state.db).clone());

    let cols = repo
        .list_columns(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(cols.into_iter().map(column_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/boards/{board_id}/columns/{column_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
        ("column_id" = String, Path, description = "Column UUID"),
    ),
    request_body = UpdateColumnRequest,
    responses(
        (status = 200, description = "Column updated", body = ColumnDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Column not found"),
        (status = 409, description = "Position exhausted during reorder — retry"),
    )
)]
pub(crate) async fn update_column(
    auth: Authorized<BoardRes, EditorMin, BoardsUpdate>,
    Path(p): Path<ColumnPath>,
    State(state): State<AppState>,
    Json(body): Json<UpdateColumnRequest>,
) -> Result<Json<ColumnDto>, ApiError> {
    if let Some(ref name) = body.name {
        validate_name("name", name)?;
    }

    // Decode the color patch value from the JSON-level representation:
    //   absent key          → None           (leave unchanged)
    //   explicit JSON null  → Some(None)     (clear color)
    //   string              → Some(Some(id)) (set color; validate swatch id)
    let color_patch: Option<Option<String>> = match body.color {
        None => None,
        Some(serde_json::Value::Null) => Some(None),
        Some(serde_json::Value::String(id)) => {
            validate_swatch("color", &id)?;
            Some(Some(id))
        }
        Some(other) => {
            return Err(ApiError::InvalidInput {
                message: format!("color must be a swatch id string or null, got {other}"),
            });
        }
    };

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgBoardRepo::new((*state.db).clone());
    let board_id = auth.resource.0.id;
    let col_id = ColumnId(p.column_id);

    if body.before.is_some() || body.after.is_some() {
        repo.move_column(
            &ctx,
            board_id,
            col_id,
            PositionBetween {
                before: body.before,
                after: body.after,
            },
        )
        .await
        .map_err(ApiError::Domain)?;
    }

    let has_patch = body.name.is_some() || color_patch.is_some();

    let col = if has_patch {
        repo.patch_column(
            &ctx,
            board_id,
            col_id,
            ColumnPatch {
                name: body.name,
                color: color_patch,
            },
        )
        .await
        .map_err(ApiError::Domain)?
    } else {
        let cols = repo
            .list_columns(&ctx, board_id)
            .await
            .map_err(ApiError::Domain)?;
        cols.into_iter()
            .find(|c| c.id == col_id)
            .ok_or(ApiError::NotFound)?
    };

    Ok(Json(column_to_dto(col)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/boards/{board_id}/columns/{column_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
    tag = "boards",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
        ("column_id" = String, Path, description = "Column UUID"),
    ),
    responses(
        (status = 204, description = "Column deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Column not found"),
    )
)]
pub(crate) async fn delete_column(
    auth: Authorized<BoardRes, EditorMin, BoardsUpdate>,
    Path(p): Path<ColumnPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgBoardRepo::new((*state.db).clone());

    repo.soft_delete_column(&ctx, auth.resource.0.id, ColumnId(p.column_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
