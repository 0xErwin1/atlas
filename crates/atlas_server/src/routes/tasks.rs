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
        ActivityEntryDto, AddAssigneeRequest, AssigneeDto, ChecklistItemDto,
        CreateChecklistItemRequest, CreateReferenceRequest, CreateSubtaskRequest,
        CreateTaskRequest, MoveTaskRequest, PromoteChecklistItemRequest, PromotionDto,
        ReferenceDto, TaskBacklinkDto, TaskDto, TaskSummaryDto, UpdateChecklistItemRequest,
        UpdateTaskRequest, WorkspaceTaskQueryParams,
    },
    dtos::documents::ActorDto,
    pagination::{Cursor, Page, SearchCursor, SortKey},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::{
        AssigneeRef, Board, BoardColumn, NewTask, NewTaskChecklistItem, NewTaskReference,
        PositionBetween, Priority, Task, TaskActivity, TaskAssignee, TaskChecklistItem, TaskPatch,
        TaskReference,
    },
    entities::task_views::{ActorTypeFilter, AssigneeFilter, TaskSort, TaskViewFilters},
    ids::{
        ApiKeyId, BoardId, ChecklistItemId, ColumnId, DocumentId, TaskActivityId, TaskId,
        TaskReferenceId, UserId,
    },
    permissions::Principal,
};

use crate::{
    authz::{
        Authorized, BoardRes, EditorMin, MinRole, TaskRes, ViewerMin, WorkspaceMember,
        authorize_board_destination,
    },
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, DocumentRepo, MembershipRepo, PgApiKeyRepo, PgBoardRepo, PgDocumentRepo,
        PgMembershipRepo, PgTaskAssigneeRepo, PgTaskChecklistRepo, PgTaskReferenceRepo, PgTaskRepo,
        PgUserRepo, TaskAssigneeRepo, TaskChecklistRepo, TaskReferenceRepo, TaskRepo, UserRepo,
    },
    routes::validation::{
        validate_custom_entry_count, validate_description, validate_labels, validate_name,
    },
    state::AppState,
};

// ---------------------------------------------------------------------------
// Shared path structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
pub(crate) struct AssigneePath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    /// Encoded as `user:{uuid}` or `api_key:{uuid}`.
    assignee_ref: String,
}

#[derive(Deserialize)]
pub(crate) struct ReferencePath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    reference_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct ChecklistItemPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    item_id: uuid::Uuid,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    }
}

fn actor_to_dto(actor: &Actor) -> ActorDto {
    match actor {
        Actor::User(uid) => ActorDto {
            r#type: "user".into(),
            id: uid.0,
            display_name: None,
        },
        Actor::ApiKey(kid) => ActorDto {
            r#type: "api_key".into(),
            id: kid.0,
            display_name: None,
        },
    }
}

fn task_to_dto(t: Task, board_name: String, column_name: String) -> TaskDto {
    TaskDto {
        id: t.id.0,
        workspace_id: t.workspace_id.0,
        project_id: t.project_id.0,
        board_id: t.board_id.0,
        column_id: t.column_id.0,
        parent_task_id: t.parent_task_id.map(|p| p.0),
        readable_id: t.readable_id,
        title: t.title,
        description: t.description,
        priority: t.priority.map(|p| p.as_str().to_string()),
        due_date: t.due_date,
        estimate: t.estimate,
        labels: t.labels,
        properties: t.properties,
        created_by: actor_to_dto(&t.created_by),
        created_at: t.created_at,
        updated_at: t.updated_at,
        board_name,
        column_name,
    }
}

/// Builds a `ReferenceDto` from a `TaskReference` and pre-resolved liveness info.
///
/// `target_resolved` and `target_readable_id` must be computed by the caller
/// against the live DB state so that soft-deleted targets are treated as broken.
fn reference_to_dto(
    r: TaskReference,
    target_resolved: bool,
    target_readable_id: Option<String>,
    target_title: Option<String>,
) -> ReferenceDto {
    ReferenceDto {
        id: r.id.0,
        kind: r.kind.as_str().to_string(),
        target_task_id: r.target_task_id.map(|id| id.0),
        target_readable_id,
        target_document_id: r.target_document_id.map(|id| id.0),
        target_title,
        target_resolved,
        created_by: actor_to_dto(&r.created_by),
        created_at: r.created_at,
    }
}

fn assignee_ref_to_actor(r: AssigneeRef) -> Actor {
    match r {
        AssigneeRef::User(uid) => Actor::User(uid),
        AssigneeRef::ApiKey(kid) => Actor::ApiKey(kid),
    }
}

/// Builds an `ActorDto` with the actor's display name resolved from the database
/// (a user's `display_name` or an api key's `name`). The base `actor_to_dto` only
/// carries the id, so without this the client has no name to show and falls back
/// to a generic "User"/"Agent" label.
async fn resolve_actor_dto(state: &AppState, ctx: &WorkspaceCtx, actor: &Actor) -> ActorDto {
    let display_name = match actor {
        Actor::User(uid) => {
            let repo = PgUserRepo {
                conn: (*state.db).clone(),
            };
            repo.find_by_id(*uid)
                .await
                .ok()
                .flatten()
                .map(|u| u.display_name)
        }
        Actor::ApiKey(kid) => {
            let repo = PgApiKeyRepo {
                conn: (*state.db).clone(),
            };
            repo.list(ctx)
                .await
                .ok()
                .and_then(|keys| keys.into_iter().find(|k| k.id == *kid).map(|k| k.name))
        }
    };

    let mut dto = actor_to_dto(actor);
    dto.display_name = display_name;
    dto
}

async fn assignee_to_dto(state: &AppState, ctx: &WorkspaceCtx, a: TaskAssignee) -> AssigneeDto {
    AssigneeDto {
        assignee: resolve_actor_dto(state, ctx, &assignee_ref_to_actor(a.assignee)).await,
        assigned_by: resolve_actor_dto(state, ctx, &a.assigned_by).await,
        assigned_at: a.assigned_at,
    }
}

/// Batch-loads the assignees for a page of tasks and resolves their display
/// names in a fixed number of queries (the assignee rows, the referenced users,
/// and the workspace's api keys), grouped by task id — so a board listing never
/// issues one query per card.
async fn board_assignees_by_task(
    state: &AppState,
    ctx: &WorkspaceCtx,
    tasks: &[Task],
) -> Result<std::collections::HashMap<uuid::Uuid, Vec<ActorDto>>, ApiError> {
    use std::collections::HashMap;

    let task_ids: Vec<TaskId> = tasks.iter().map(|t| t.id).collect();

    let rows = PgTaskAssigneeRepo::new((*state.db).clone())
        .list_for_tasks(ctx, &task_ids)
        .await
        .map_err(ApiError::Domain)?;

    let mut user_ids: Vec<UserId> = Vec::new();
    let mut needs_keys = false;
    for r in &rows {
        match r.assignee {
            AssigneeRef::User(uid) => user_ids.push(uid),
            AssigneeRef::ApiKey(_) => needs_keys = true,
        }
    }
    user_ids.sort_by_key(|u| u.0);
    user_ids.dedup_by_key(|u| u.0);

    let user_names: HashMap<uuid::Uuid, String> = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&user_ids)
    .await
    .map_err(ApiError::Domain)?
    .into_iter()
    .map(|u| (u.id.0, u.display_name))
    .collect();

    let key_names: HashMap<uuid::Uuid, String> = if needs_keys {
        PgApiKeyRepo {
            conn: (*state.db).clone(),
        }
        .list(ctx)
        .await
        .map_err(ApiError::Domain)?
        .into_iter()
        .map(|k| (k.id.0, k.name))
        .collect()
    } else {
        HashMap::new()
    };

    let mut by_task: HashMap<uuid::Uuid, Vec<ActorDto>> = HashMap::new();
    for r in rows {
        let actor = assignee_ref_to_actor(r.assignee);
        let mut dto = actor_to_dto(&actor);
        dto.display_name = match &actor {
            Actor::User(uid) => user_names.get(&uid.0).cloned(),
            Actor::ApiKey(kid) => key_names.get(&kid.0).cloned(),
        };
        by_task.entry(r.task_id.0).or_default().push(dto);
    }

    Ok(by_task)
}

/// Batch-loads the board name and column name for a page of tasks.
///
/// Issues two `IN (...)` queries — one for columns, one for boards — both
/// scoped to `ctx.workspace_id`, then builds a map from column id to
/// `(board_name, column_name)`. The caller can then populate `board_name` and
/// `column_name` on each `TaskSummaryDto` without an N+1 query.
///
/// When a column or board row cannot be found (data-integrity error), the
/// entry is absent from the returned map; callers substitute a fallback
/// so a missing row does not abort the listing.
///
/// Returns a map keyed by `column_id` → `(board_id, board_name, column_name)`.
async fn board_column_names_by_task(
    state: &AppState,
    ctx: &WorkspaceCtx,
    tasks: &[Task],
) -> Result<std::collections::HashMap<uuid::Uuid, (uuid::Uuid, String, String)>, ApiError> {
    use std::collections::HashMap;

    let mut col_ids: Vec<uuid::Uuid> = tasks.iter().map(|t| t.column_id.0).collect();
    col_ids.sort_unstable();
    col_ids.dedup();

    let repo = PgBoardRepo::new((*state.db).clone());

    let columns: Vec<BoardColumn> = repo
        .list_columns_by_ids(ctx.workspace_id.0, &col_ids)
        .await
        .map_err(ApiError::Domain)?;

    let mut board_ids: Vec<uuid::Uuid> = columns.iter().map(|c| c.board_id.0).collect();
    board_ids.sort_unstable();
    board_ids.dedup();

    let boards: Vec<Board> = repo
        .list_boards_by_ids(ctx.workspace_id.0, &board_ids)
        .await
        .map_err(ApiError::Domain)?;

    let board_names: HashMap<uuid::Uuid, String> =
        boards.into_iter().map(|b| (b.id.0, b.name)).collect();

    let by_column: HashMap<uuid::Uuid, (uuid::Uuid, String, String)> = columns
        .into_iter()
        .filter_map(|col| {
            let board_name = board_names.get(&col.board_id.0)?.clone();
            Some((col.id.0, (col.board_id.0, board_name, col.name)))
        })
        .collect();

    Ok(by_column)
}

fn checklist_item_to_dto(item: TaskChecklistItem) -> ChecklistItemDto {
    ChecklistItemDto {
        id: item.id.0,
        task_id: item.task_id.0,
        title: item.title,
        checked: item.checked,
        position_key: item.position_key,
        promoted_task_id: item.promoted_task_id.map(|id| id.0),
        promoted_readable_id: None,
        created_at: item.created_at,
        updated_at: item.updated_at,
    }
}

fn activity_to_dto(a: TaskActivity) -> ActivityEntryDto {
    ActivityEntryDto {
        id: a.id.0,
        kind: a.kind.as_str().to_string(),
        actor: actor_to_dto(&a.actor),
        payload: serde_json::to_value(&a.payload).unwrap_or(serde_json::Value::Null),
        created_at: a.created_at,
    }
}

/// Parses the `{assignee_ref}` path segment (`user:{uuid}` or `api_key:{uuid}`).
fn parse_assignee_ref(s: &str) -> Result<AssigneeRef, ApiError> {
    if let Some(rest) = s.strip_prefix("user:") {
        let uuid = rest.parse::<uuid::Uuid>().map_err(|_| ApiError::NotFound)?;
        return Ok(AssigneeRef::User(UserId(uuid)));
    }
    if let Some(rest) = s.strip_prefix("api_key:") {
        let uuid = rest.parse::<uuid::Uuid>().map_err(|_| ApiError::NotFound)?;
        return Ok(AssigneeRef::ApiKey(ApiKeyId(uuid)));
    }
    Err(ApiError::NotFound)
}

/// Checks that an `AssigneeRef` is a member of `ctx.workspace_id`.
///
/// A user must have a membership row; an api_key must belong to the workspace
/// (not revoked). Either missing → 404 to conceal principal existence across
/// workspaces.
async fn validate_assignee_is_workspace_member(
    assignee: &AssigneeRef,
    ctx: &WorkspaceCtx,
    state: &AppState,
) -> Result<(), ApiError> {
    match assignee {
        AssigneeRef::User(uid) => {
            let membership_repo = PgMembershipRepo {
                conn: (*state.db).clone(),
            };
            let member = membership_repo
                .find(ctx, *uid)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;
            if member.is_none() {
                return Err(ApiError::NotFound);
            }
        }
        AssigneeRef::ApiKey(kid) => {
            let api_key_repo = PgApiKeyRepo {
                conn: (*state.db).clone(),
            };
            let keys = api_key_repo
                .list(ctx)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;
            if !keys.iter().any(|k| k.id == *kid) {
                return Err(ApiError::NotFound);
            }
        }
    }
    Ok(())
}

/// Parses `priority` from the wire representation (a nullable string).
fn parse_priority(val: Option<serde_json::Value>) -> Result<Option<Option<Priority>>, ApiError> {
    match val {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(s)) => {
            let p: Priority = s.parse().map_err(|_| ApiError::InvalidInput {
                message: format!("unknown priority: {s}; must be low|medium|high|urgent"),
            })?;
            Ok(Some(Some(p)))
        }
        _ => Err(ApiError::InvalidInput {
            message: "priority must be a string or null".into(),
        }),
    }
}

/// Parses `estimate` from the wire representation (a nullable non-negative integer).
fn parse_estimate(val: Option<serde_json::Value>) -> Result<Option<Option<i32>>, ApiError> {
    match val {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::Number(n)) => {
            let i = n.as_i64().ok_or_else(|| ApiError::InvalidInput {
                message: "estimate must be an integer".into(),
            })?;
            if i < 0 {
                return Err(ApiError::InvalidInput {
                    message: "estimate must be non-negative".into(),
                });
            }
            Ok(Some(Some(i32::try_from(i).map_err(|_| {
                ApiError::InvalidInput {
                    message: "estimate out of range".into(),
                }
            })?)))
        }
        _ => Err(ApiError::InvalidInput {
            message: "estimate must be a number or null".into(),
        }),
    }
}

/// Parses `due_date` from the wire representation (a nullable datetime string).
fn parse_due_date(
    val: Option<serde_json::Value>,
) -> Result<Option<Option<chrono::DateTime<chrono::Utc>>>, ApiError> {
    match val {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(s)) => {
            let dt =
                s.parse::<chrono::DateTime<chrono::Utc>>()
                    .map_err(|_| ApiError::InvalidInput {
                        message: format!("invalid due_date: {s}; must be RFC 3339"),
                    })?;
            Ok(Some(Some(dt)))
        }
        _ => Err(ApiError::InvalidInput {
            message: "due_date must be a string or null".into(),
        }),
    }
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/boards/{board_id}/tasks
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/boards/{board_id}/tasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    request_body = CreateTaskRequest,
    responses(
        (status = 201, description = "Task created", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn create_task(
    auth: Authorized<BoardRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let board = &auth.resource.0;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    validate_name("title", &body.title)?;

    let description = body.description.unwrap_or_default();
    validate_description(&description)?;

    let props = body.properties.unwrap_or_default();
    let priority = props
        .priority
        .as_deref()
        .map(|s| {
            s.parse::<Priority>().map_err(|_| ApiError::InvalidInput {
                message: format!("unknown priority: {s}; must be low|medium|high|urgent"),
            })
        })
        .transpose()?;

    if let Some(est) = props.estimate
        && est < 0
    {
        return Err(ApiError::InvalidInput {
            message: "estimate must be non-negative".into(),
        });
    }

    validate_labels(&props.labels)?;

    if let Some(ref custom) = props.custom {
        validate_custom_entry_count(custom)?;
    }

    let task = state
        .task_service()
        .create(
            &ctx,
            NewTask {
                project_id: board.project_id,
                board_id: board.id,
                column_id: ColumnId(body.column_id),
                title: body.title,
                description,
                priority,
                due_date: props.due_date,
                estimate: props.estimate,
                labels: props.labels,
                properties: props.custom,
                position: PositionBetween {
                    before: body.before,
                    after: body.after,
                },
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((
        StatusCode::CREATED,
        Json(task_to_dto(task, String::new(), String::new())),
    ))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/boards/{board_id}/tasks
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/boards/{board_id}/tasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated task list", body = Page<TaskSummaryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn list_tasks(
    auth: Authorized<BoardRes, ViewerMin>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<TaskSummaryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskRepo::new((*state.db).clone());

    let mut all = repo
        .list_by_board(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);
    if let Some(id) = after_id
        && let Some(pos) = all.iter().position(|t| t.id.0 == id)
    {
        all = all.into_iter().skip(pos + 1).collect();
    }

    let has_more = all.len() > limit;
    if has_more {
        all.truncate(limit);
    }

    let next_cursor = if has_more {
        all.last().map(|t| Cursor(t.id.0))
    } else {
        None
    };

    let mut assignees_by_task = board_assignees_by_task(&state, &ctx, &all).await?;
    let board_column_names = board_column_names_by_task(&state, &ctx, &all).await?;
    let board_name_fallback = auth.resource.0.name.clone();

    let board_id_fallback = auth.resource.0.id.0;

    let dtos = all
        .into_iter()
        .map(|t| {
            let (board_id, board_name, column_name) = board_column_names
                .get(&t.column_id.0)
                .cloned()
                .unwrap_or_else(|| {
                    (
                        board_id_fallback,
                        board_name_fallback.clone(),
                        String::new(),
                    )
                });

            TaskSummaryDto {
                id: t.id.0,
                readable_id: t.readable_id,
                board_id,
                column_id: t.column_id.0,
                title: t.title,
                priority: t.priority.map(|p| p.as_str().to_string()),
                estimate: t.estimate,
                labels: t.labels,
                assignees: assignees_by_task.remove(&t.id.0).unwrap_or_default(),
                board_name,
                column_name,
                updated_at: t.updated_at,
            }
        })
        .collect();

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tasks/{readable_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID (e.g. ATL-42)"),
    ),
    responses(
        (status = 200, description = "Task", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn get_task(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<TaskDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let board_column_names =
        board_column_names_by_task(&state, &ctx, std::slice::from_ref(&auth.resource.0)).await?;

    let (_board_id, board_name, column_name) = board_column_names
        .get(&auth.resource.0.column_id.0)
        .cloned()
        .unwrap_or_else(|| (auth.resource.0.board_id.0, String::new(), String::new()));

    Ok(Json(task_to_dto(auth.resource.0, board_name, column_name)))
}

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/tasks/{readable_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = UpdateTaskRequest,
    responses(
        (status = 200, description = "Task updated", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn update_task(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<TaskDto>, ApiError> {
    let task_id = auth.resource.0.id;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    if let Some(ref title) = body.title {
        validate_name("title", title)?;
    }

    if let Some(ref desc) = body.description {
        validate_description(desc)?;
    }

    if let Some(ref labels) = body.labels {
        validate_labels(labels)?;
    }

    let priority = parse_priority(body.priority)?;
    let due_date = parse_due_date(body.due_date)?;
    let estimate = parse_estimate(body.estimate)?;

    if let Some(ref props) = body.properties {
        validate_custom_entry_count(props)?;
    }

    let patch = TaskPatch {
        title: body.title,
        description: body.description,
        priority,
        due_date,
        estimate,
        labels: body.labels,
        properties: body.properties,
    };

    let updated = state
        .task_service()
        .patch(&ctx, task_id, patch)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_to_dto(updated, String::new(), String::new())))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/tasks/{readable_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 204, description = "Task deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn delete_task(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    state
        .task_service()
        .delete_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/tasks/{readable_id}/move
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/move",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = MoveTaskRequest,
    responses(
        (status = 200, description = "Task moved", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Position exhausted — retry"),
    )
)]
pub(crate) async fn move_task(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<MoveTaskRequest>,
) -> Result<Json<TaskDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    // The Authorized<TaskRes, EditorMin> extractor only proves edit rights on the
    // task's current board. A move may target a column on a different board, so
    // independently require edit rights on the destination board too; otherwise a
    // user could relocate a task into a board they cannot access.
    authorize_board_destination(
        &state.db,
        &auth.principal,
        auth.membership.clone(),
        &auth.workspace,
        ColumnId(body.column_id),
        EditorMin::ROLE,
    )
    .await?;

    let moved = state
        .task_service()
        .move_task(
            &ctx,
            auth.resource.0.id,
            ColumnId(body.column_id),
            PositionBetween {
                before: body.before,
                after: body.after,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_to_dto(moved, String::new(), String::new())))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tasks/{readable_id}/assignees
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/assignees",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 200, description = "Assignee list", body = Vec<AssigneeDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_assignees(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<AssigneeDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskAssigneeRepo::new((*state.db).clone());

    let items = repo
        .list_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let mut dtos = Vec::with_capacity(items.len());
    for item in items {
        dtos.push(assignee_to_dto(&state, &ctx, item).await);
    }

    Ok(Json(dtos))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/tasks/{readable_id}/assignees
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/assignees",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = AddAssigneeRequest,
    responses(
        (status = 201, description = "Assignee added", body = AssigneeDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or principal not found"),
        (status = 409, description = "Assignee already added"),
    )
)]
pub(crate) async fn add_assignee(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<AddAssigneeRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let task_id = auth.resource.0.id;

    let assignee = match body.assignee_type.as_str() {
        "user" => AssigneeRef::User(UserId(body.assignee_id)),
        "api_key" => AssigneeRef::ApiKey(ApiKeyId(body.assignee_id)),
        _ => {
            return Err(ApiError::InvalidInput {
                message: "assignee_type must be 'user' or 'api_key'".into(),
            });
        }
    };

    validate_assignee_is_workspace_member(&assignee, &ctx, &state).await?;

    let result = state
        .task_service()
        .assign(&ctx, task_id, assignee)
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::Forbidden { message } if message.contains("already") => {
                ApiError::Conflict
            }
            other => ApiError::Domain(other),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(assignee_to_dto(&state, &ctx, result).await),
    ))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("assignee_ref" = String, Path, description = "user:{uuid} or api_key:{uuid}"),
    ),
    responses(
        (status = 204, description = "Assignee removed"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or assignee not found"),
    )
)]
pub(crate) async fn remove_assignee(
    auth: Authorized<TaskRes, EditorMin>,
    Path(p): Path<AssigneePath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let assignee = parse_assignee_ref(&p.assignee_ref)?;

    state
        .task_service()
        .unassign(&ctx, auth.resource.0.id, assignee)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tasks/{readable_id}/references
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/references",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 200, description = "Reference list", body = Vec<ReferenceDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_references(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ReferenceDto>>, ApiError> {
    use std::collections::{HashMap, HashSet};

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskReferenceRepo::new((*state.db).clone());

    let refs = repo
        .list_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let task_repo = PgTaskRepo::new((*state.db).clone());
    let doc_repo = PgDocumentRepo::new((*state.db).clone(), 0);

    let target_task_ids: Vec<TaskId> = refs
        .iter()
        .filter_map(|r| r.target_task_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let target_doc_ids: Vec<DocumentId> = refs
        .iter()
        .filter_map(|r| r.target_document_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut live_task_readable: HashMap<TaskId, String> = HashMap::new();
    for tid in target_task_ids {
        if let Some(t) = task_repo.find(&ctx, tid).await.map_err(ApiError::Domain)? {
            live_task_readable.insert(tid, t.readable_id);
        }
    }

    let mut live_doc_titles: HashMap<DocumentId, String> = HashMap::new();
    for did in target_doc_ids {
        if let Some(doc) = doc_repo.get(&ctx, did).await.map_err(ApiError::Domain)? {
            live_doc_titles.insert(did, doc.title);
        }
    }

    let dtos = refs
        .into_iter()
        .map(|r| {
            let (target_resolved, target_readable_id, target_title) =
                match (r.target_task_id, r.target_document_id) {
                    (Some(tid), _) => {
                        let readable = live_task_readable.get(&tid).cloned();
                        (readable.is_some(), readable, None)
                    }
                    (_, Some(did)) => {
                        let title = live_doc_titles.get(&did).cloned();
                        (title.is_some(), None, title)
                    }
                    _ => (false, None, None),
                };
            reference_to_dto(r, target_resolved, target_readable_id, target_title)
        })
        .collect();

    Ok(Json(dtos))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/tasks/{readable_id}/references
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/references",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = CreateReferenceRequest,
    responses(
        (status = 201, description = "Reference created", body = ReferenceDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Duplicate reference"),
        (status = 422, description = "Invalid reference kind"),
    )
)]
/// Creates a reference from the authorized source task to exactly one target.
///
/// `task_references` targets are backed by foreign-key columns: the row cannot
/// be stored without a real target in this workspace. Both-null and both-set
/// bodies are rejected here as 422 before reaching the DB, preventing a CHECK
/// constraint violation or a silent both-null insert.
pub(crate) async fn create_reference(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateReferenceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    use atlas_domain::entities::boards_tasks::ReferenceKind;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let kind: ReferenceKind = match body.kind.as_str() {
        "relates" => ReferenceKind::Relates,
        "blocks" => ReferenceKind::Blocks,
        "parent" => ReferenceKind::Parent,
        "spec" => ReferenceKind::Spec,
        other => {
            return Err(ApiError::InvalidInput {
                message: format!(
                    "unknown reference kind: {other}; must be relates|blocks|parent|spec"
                ),
            });
        }
    };

    let has_task_target = body.target_task_readable_id.is_some();
    let has_doc_target = body.target_document_id.is_some();

    if !has_task_target && !has_doc_target {
        return Err(ApiError::InvalidInput {
            message:
                "exactly one of target_task_readable_id or target_document_id must be provided"
                    .into(),
        });
    }
    if has_task_target && has_doc_target {
        return Err(ApiError::InvalidInput {
            message:
                "exactly one of target_task_readable_id or target_document_id must be provided"
                    .into(),
        });
    }

    let mut target_readable_id: Option<String> = None;

    let target_task_id = if let Some(rid) = body.target_task_readable_id {
        let repo = PgTaskRepo::new((*state.db).clone());
        let found = repo
            .find_by_readable_id(&ctx, &rid)
            .await
            .map_err(ApiError::Domain)?;

        match found {
            Some(t) => {
                target_readable_id = Some(t.readable_id.clone());
                Some(t.id)
            }
            None => return Err(ApiError::NotFound),
        }
    } else {
        None
    };

    let mut target_title: Option<String> = None;

    let target_document_id = if let Some(raw_id) = body.target_document_id {
        let doc_id = DocumentId(raw_id);
        let doc_repo = PgDocumentRepo::new((*state.db).clone(), 0);
        let found = doc_repo.get(&ctx, doc_id).await.map_err(ApiError::Domain)?;

        match found {
            Some(doc) => {
                target_title = Some(doc.title);
                Some(doc_id)
            }
            None => {
                return Err(ApiError::Domain(atlas_domain::DomainError::NotFound {
                    entity: "document",
                    id: raw_id,
                }));
            }
        }
    } else {
        None
    };

    let reference = state
        .task_service()
        .add_reference(
            &ctx,
            NewTaskReference {
                source_task_id: auth.resource.0.id,
                kind,
                target_task_id,
                target_document_id,
            },
        )
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::Forbidden { .. } => ApiError::Conflict,
            other => ApiError::Domain(other),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(reference_to_dto(
            reference,
            true,
            target_readable_id,
            target_title,
        )),
    ))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("reference_id" = String, Path, description = "Reference UUID"),
    ),
    responses(
        (status = 204, description = "Reference deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Reference not found"),
    )
)]
pub(crate) async fn delete_reference(
    auth: Authorized<TaskRes, EditorMin>,
    Path(p): Path<ReferencePath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let ref_id = TaskReferenceId(p.reference_id);

    state
        .task_service()
        .remove_reference(&ctx, auth.resource.0.id, ref_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tasks/{readable_id}/backlinks
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/backlinks",
    operation_id = "list_task_backlinks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Inbound reference list"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_backlinks(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<TaskBacklinkDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let ref_repo = PgTaskReferenceRepo::new((*state.db).clone());
    let task_repo = PgTaskRepo::new((*state.db).clone());

    let mut inbound = ref_repo
        .list_inbound(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);
    if let Some(id) = after_id
        && let Some(pos) = inbound.iter().position(|r| r.id.0 == id)
    {
        inbound = inbound.into_iter().skip(pos + 1).collect();
    }

    let has_more = inbound.len() > limit;
    if has_more {
        inbound.truncate(limit);
    }

    let next_cursor = if has_more {
        inbound.last().map(|r| Cursor(r.id.0))
    } else {
        None
    };

    let mut dtos = Vec::with_capacity(inbound.len());
    for r in inbound {
        let source_task = task_repo
            .find(&ctx, r.source_task_id)
            .await
            .map_err(ApiError::Domain)?;
        if let Some(t) = source_task {
            dtos.push(TaskBacklinkDto {
                source_task_id: t.id.0,
                source_readable_id: t.readable_id,
                source_title: t.title,
                kind: r.kind.as_str().to_string(),
            });
        }
    }

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tasks/{readable_id}/checklist
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/checklist",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 200, description = "Checklist items", body = Vec<ChecklistItemDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_checklist(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ChecklistItemDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskChecklistRepo::new((*state.db).clone());

    let items = repo
        .list_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(items.into_iter().map(checklist_item_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/tasks/{readable_id}/checklist
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/checklist",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = CreateChecklistItemRequest,
    responses(
        (status = 201, description = "Checklist item created", body = ChecklistItemDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn create_checklist_item(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateChecklistItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    validate_name("title", &body.title)?;

    let item = state
        .task_service()
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: auth.resource.0.id,
                title: body.title,
                position: PositionBetween {
                    before: body.before,
                    after: body.after,
                },
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(checklist_item_to_dto(item))))
}

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("item_id" = String, Path, description = "Checklist item UUID"),
    ),
    request_body = UpdateChecklistItemRequest,
    responses(
        (status = 200, description = "Checklist item updated", body = ChecklistItemDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Item not found"),
        (status = 409, description = "Position exhausted — retry"),
    )
)]
pub(crate) async fn update_checklist_item(
    auth: Authorized<TaskRes, EditorMin>,
    Path(p): Path<ChecklistItemPath>,
    State(state): State<AppState>,
    Json(body): Json<UpdateChecklistItemRequest>,
) -> Result<Json<ChecklistItemDto>, ApiError> {
    use atlas_domain::entities::boards_tasks::TaskChecklistItemPatch;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    if let Some(ref title) = body.title {
        validate_name("title", title)?;
    }
    let item_id = ChecklistItemId(p.item_id);

    let position = if body.before.is_some() || body.after.is_some() {
        Some(PositionBetween {
            before: body.before,
            after: body.after,
        })
    } else {
        None
    };

    let item = state
        .task_service()
        .patch_checklist_item(
            &ctx,
            auth.resource.0.id,
            item_id,
            TaskChecklistItemPatch {
                title: body.title,
                checked: body.checked,
                position,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(checklist_item_to_dto(item)))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("item_id" = String, Path, description = "Checklist item UUID"),
    ),
    responses(
        (status = 204, description = "Checklist item deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Item not found"),
    )
)]
pub(crate) async fn delete_checklist_item(
    auth: Authorized<TaskRes, EditorMin>,
    Path(p): Path<ChecklistItemPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    state
        .task_service()
        .remove_checklist_item(&ctx, auth.resource.0.id, ChecklistItemId(p.item_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("item_id" = String, Path, description = "Checklist item UUID"),
    ),
    request_body = PromoteChecklistItemRequest,
    responses(
        (status = 201, description = "Checklist item promoted to task", body = PromotionDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Item not found"),
        (status = 409, description = "Item already promoted"),
    )
)]
pub(crate) async fn promote_checklist_item(
    auth: Authorized<TaskRes, EditorMin>,
    Path(p): Path<ChecklistItemPath>,
    State(state): State<AppState>,
    Json(body): Json<PromoteChecklistItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let parent_task = &auth.resource.0;

    if body.board_id != parent_task.board_id.0 {
        return Err(ApiError::InvalidInput {
            message: "promoted task must stay on the parent task's board".into(),
        });
    }

    let result = state
        .task_service()
        .promote_checklist_item(
            &ctx,
            parent_task.id,
            ChecklistItemId(p.item_id),
            parent_task.project_id,
            BoardId(body.board_id),
            ColumnId(body.column_id),
        )
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::Forbidden { message }
                if message.contains("already been promoted") =>
            {
                ApiError::Conflict
            }
            other => ApiError::Domain(other),
        })?;

    let promoted_readable_id = result.task.readable_id.clone();
    let dto = PromotionDto {
        task: task_to_dto(result.task, String::new(), String::new()),
        parent_reference: Some(reference_to_dto(
            result.parent_reference,
            true,
            Some(promoted_readable_id),
            None,
        )),
        checklist_item: checklist_item_to_dto(result.checklist_item),
    };

    Ok((StatusCode::CREATED, Json(dto)))
}

// ---------------------------------------------------------------------------
// Sub-tasks (child tasks)
// ---------------------------------------------------------------------------

/// Builds `TaskSummaryDto`s for the given tasks, resolving assignees and board/
/// column names in a fixed number of batch queries.
async fn tasks_to_summaries(
    state: &AppState,
    ctx: &WorkspaceCtx,
    tasks: Vec<Task>,
) -> Result<Vec<TaskSummaryDto>, ApiError> {
    let mut assignees_by_task = board_assignees_by_task(state, ctx, &tasks).await?;
    let board_column_names = board_column_names_by_task(state, ctx, &tasks).await?;

    Ok(tasks
        .into_iter()
        .map(|t| {
            let (board_id, board_name, column_name) = board_column_names
                .get(&t.column_id.0)
                .cloned()
                .unwrap_or_default();

            TaskSummaryDto {
                id: t.id.0,
                readable_id: t.readable_id,
                board_id,
                column_id: t.column_id.0,
                title: t.title,
                priority: t.priority.map(|p| p.as_str().to_string()),
                estimate: t.estimate,
                labels: t.labels,
                assignees: assignees_by_task.remove(&t.id.0).unwrap_or_default(),
                board_name,
                column_name,
                updated_at: t.updated_at,
            }
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/subtasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Parent task readable ID"),
    ),
    responses(
        (status = 200, description = "Sub-tasks of the task", body = Vec<TaskSummaryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_subtasks(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskSummaryDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskRepo::new((*state.db).clone());

    let children = repo
        .list_children(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let dtos = tasks_to_summaries(&state, &ctx, children).await?;
    Ok(Json(dtos))
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/subtasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Parent task readable ID"),
    ),
    request_body = CreateSubtaskRequest,
    responses(
        (status = 201, description = "Sub-task created", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn create_subtask(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateSubtaskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    validate_name("title", &body.title)?;

    let task = state
        .task_service()
        .create_subtask(&ctx, &auth.resource.0, body.title)
        .await
        .map_err(ApiError::Domain)?;

    Ok((
        StatusCode::CREATED,
        Json(task_to_dto(task, String::new(), String::new())),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/promote",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Sub-task readable ID"),
    ),
    responses(
        (status = 200, description = "Sub-task promoted to a board task", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn promote_subtask(
    auth: Authorized<TaskRes, EditorMin>,
    State(state): State<AppState>,
) -> Result<Json<TaskDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let task = state
        .task_service()
        .promote_subtask(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_to_dto(task, String::new(), String::new())))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tasks/{readable_id}/activity
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks/{readable_id}/activity",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Activity feed", body = Page<ActivityEntryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_activity(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ActivityEntryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let after_id = q
        .cursor
        .as_deref()
        .and_then(Cursor::decode)
        .map(|c| TaskActivityId(c.0));

    let mut entries = state
        .task_service()
        .list_activity(&ctx, auth.resource.0.id, after_id, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = entries.len() > limit as usize;
    if has_more {
        entries.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        entries.last().map(|a| Cursor(a.id.0))
    } else {
        None
    };

    let dtos = entries.into_iter().map(activity_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tasks
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("assignee" = Option<String>, Query, description = "Assignee filter: 'me', 'user:{uuid}', or 'api_key:{uuid}'"),
        ("actor" = Option<String>, Query, description = "Actor type filter: 'user' or 'api_key'"),
        ("column_id" = Option<String>, Query, description = "Filter by column id (repeat for multiple)"),
        ("priority" = Option<String>, Query, description = "Filter by priority (repeat for multiple)"),
        ("label" = Option<String>, Query, description = "Filter by label (repeat for multiple)"),
        ("board_id" = Option<String>, Query, description = "Filter by board id"),
        ("sort" = Option<String>, Query, description = "Sort order: updated_at_desc (default), updated_at_asc, created_at_desc, created_at_asc, priority_desc, title_asc"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor (34-char base64url)"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200, default 50)"),
    ),
    responses(
        (status = 200, description = "Paginated workspace task list", body = Page<TaskSummaryDto>),
        (status = 400, description = "Invalid query parameter (e.g. unknown sort)"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
    )
)]
pub(crate) async fn list_workspace_tasks(
    member: WorkspaceMember,
    State(state): State<AppState>,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
) -> Result<Json<Page<TaskSummaryDto>>, ApiError> {
    let q = parse_workspace_task_query(raw_query.as_deref().unwrap_or(""))?;

    let actor = match (&member.user, &member.api_key_id) {
        (Some(user), _) => Actor::User(user.id),
        (None, Some(key_id)) => Actor::ApiKey(*key_id),
        (None, None) => return Err(ApiError::Unauthorized),
    };

    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;

    let sort = q
        .sort
        .as_deref()
        .map(|s| {
            TaskSort::from_param_str(s).ok_or_else(|| ApiError::BadRequest {
                message: format!("invalid sort key '{s}'"),
            })
        })
        .transpose()?;

    let priorities = q
        .priorities
        .iter()
        .map(|p| {
            p.parse::<Priority>().map_err(|_| ApiError::InvalidInput {
                message: format!("invalid priority '{p}'"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let column_ids = q
        .column_ids
        .iter()
        .map(|s| {
            s.parse::<uuid::Uuid>()
                .map(atlas_domain::ids::ColumnId)
                .map_err(|_| ApiError::InvalidInput {
                    message: format!("invalid column_id '{s}'"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let board_id = q
        .board_id
        .as_deref()
        .map(|s| {
            s.parse::<uuid::Uuid>()
                .map(atlas_domain::ids::BoardId)
                .map_err(|_| ApiError::InvalidInput {
                    message: format!("invalid board_id '{s}'"),
                })
        })
        .transpose()?;

    let assignee = q
        .assignee
        .as_deref()
        .map(parse_workspace_assignee_filter)
        .transpose()?;

    let actor_type = q
        .actor
        .as_deref()
        .map(|s| match s {
            "user" => Ok(ActorTypeFilter::User),
            "api_key" => Ok(ActorTypeFilter::ApiKey),
            other => Err(ApiError::InvalidInput {
                message: format!("invalid actor filter '{other}'; must be 'user' or 'api_key'"),
            }),
        })
        .transpose()?;

    let after = q
        .cursor
        .as_deref()
        .map(|s| {
            SearchCursor::decode(s).ok_or_else(|| ApiError::InvalidInput {
                message: "invalid cursor".to_string(),
            })
        })
        .transpose()?
        .map(|sc| {
            let micros = match sc.key {
                SortKey::Updated(m) => m,
                SortKey::Relevance(_) => {
                    return Err(ApiError::InvalidInput {
                        message: "cursor sort key is not compatible with task listing".to_string(),
                    });
                }
            };
            Ok(atlas_domain::ports::boards_tasks::TaskListCursor {
                sort_value: serde_json::Value::Number(micros.into()),
                id: atlas_domain::ids::TaskId(sc.id),
            })
        })
        .transpose()?;

    let filters = TaskViewFilters {
        sort,
        priorities,
        column_ids,
        labels: q.labels,
        board_id,
        assignee,
        actor_type,
    };

    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgTaskRepo::new((*state.db).clone());

    let mut tasks = repo
        .list_by_workspace_filtered(&ctx, &filters, after, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = tasks.len() > limit as usize;
    if has_more {
        tasks.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        tasks.last().map(|t| SearchCursor {
            key: SortKey::Updated(t.updated_at.timestamp_micros()),
            id: t.id.0,
        })
    } else {
        None
    };

    let mut assignees_by_task = board_assignees_by_task(&state, &ctx, &tasks).await?;
    let board_column_names = board_column_names_by_task(&state, &ctx, &tasks).await?;

    let dtos = tasks
        .into_iter()
        .map(|t| {
            let (board_id, board_name, column_name) = board_column_names
                .get(&t.column_id.0)
                .cloned()
                .unwrap_or_default();

            TaskSummaryDto {
                id: t.id.0,
                readable_id: t.readable_id,
                board_id,
                column_id: t.column_id.0,
                title: t.title,
                priority: t.priority.map(|p| p.as_str().to_string()),
                estimate: t.estimate,
                labels: t.labels,
                assignees: assignees_by_task.remove(&t.id.0).unwrap_or_default(),
                board_name,
                column_name,
                updated_at: t.updated_at,
            }
        })
        .collect();

    Ok(Json(Page::new_search(dtos, next_cursor, has_more)))
}

/// Parses the raw query string for `GET /v1/workspaces/{ws}/tasks`.
///
/// Uses `form_urlencoded` directly to support repeated params (e.g.
/// `?column_id=x&column_id=y`) which `serde_urlencoded` does not handle for
/// `Vec<T>` fields.
fn parse_workspace_task_query(raw: &str) -> Result<WorkspaceTaskQueryParams, ApiError> {
    let mut q = WorkspaceTaskQueryParams::default();
    for pair in raw.split('&').filter(|s| !s.is_empty()) {
        let (key, val) = pair.split_once('=').unwrap_or((pair, ""));
        // Decode `+` as space (form-urlencoded convention). Full percent-decoding
        // is not needed here because all expected values are ASCII-safe (UUIDs,
        // slugs, sort keys, simple labels). A label with non-ASCII chars would
        // arrive as a percent-encoded value; those cases are handled server-side
        // by treating the raw encoded form as the label string.
        let val = val.replace('+', " ");
        match key {
            "assignee" => q.assignee = Some(val),
            "actor" => q.actor = Some(val),
            "column_id" => q.column_ids.push(val),
            "priority" => q.priorities.push(val),
            "label" => q.labels.push(val),
            "board_id" => q.board_id = Some(val),
            "sort" => q.sort = Some(val),
            "cursor" => q.cursor = Some(val),
            "limit" => {
                q.limit = val.parse::<u32>().ok();
            }
            _ => {}
        }
    }
    Ok(q)
}

fn parse_workspace_assignee_filter(s: &str) -> Result<AssigneeFilter, ApiError> {
    if s == "me" {
        return Ok(AssigneeFilter::Me);
    }
    if let Some(rest) = s.strip_prefix("user:") {
        let id = rest
            .parse::<uuid::Uuid>()
            .map_err(|_| ApiError::InvalidInput {
                message: format!("invalid assignee user uuid: {rest}"),
            })?;
        return Ok(AssigneeFilter::User(UserId(id)));
    }
    if let Some(rest) = s.strip_prefix("api_key:") {
        let id = rest
            .parse::<uuid::Uuid>()
            .map_err(|_| ApiError::InvalidInput {
                message: format!("invalid assignee api_key uuid: {rest}"),
            })?;
        return Ok(AssigneeFilter::ApiKey(ApiKeyId(id)));
    }
    Err(ApiError::InvalidInput {
        message: format!(
            "invalid assignee filter '{s}'; must be 'me', 'user:{{uuid}}', or 'api_key:{{uuid}}'"
        ),
    })
}
