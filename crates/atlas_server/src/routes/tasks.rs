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
        CreateChecklistItemRequest, CreateReferenceRequest, CreateTaskRequest, MoveTaskRequest,
        PromoteChecklistItemRequest, PromotionDto, ReferenceDto, TaskBacklinkDto, TaskDto,
        TaskSummaryDto, UpdateChecklistItemRequest, UpdateTaskRequest,
    },
    dtos::documents::ActorDto,
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::{
        AssigneeRef, NewTask, NewTaskChecklistItem, NewTaskReference, PositionBetween, Priority,
        Task, TaskActivity, TaskAssignee, TaskChecklistItem, TaskPatch, TaskReference,
    },
    ids::{ApiKeyId, BoardId, ChecklistItemId, ColumnId, TaskActivityId, TaskReferenceId, UserId},
    permissions::Principal,
};

use crate::{
    authz::{Authorized, BoardRes, EditorMin, TaskRes, ViewerMin},
    error::ApiError,
    persistence::repos::{
        PgTaskAssigneeRepo, PgTaskChecklistRepo, PgTaskReferenceRepo, PgTaskRepo, TaskAssigneeRepo,
        TaskChecklistRepo, TaskReferenceRepo, TaskRepo,
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

fn task_to_dto(t: Task) -> TaskDto {
    TaskDto {
        id: t.id.0,
        workspace_id: t.workspace_id.0,
        project_id: t.project_id.0,
        board_id: t.board_id.0,
        column_id: t.column_id.0,
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
    }
}

fn reference_to_dto(r: TaskReference) -> ReferenceDto {
    ReferenceDto {
        id: r.id.0,
        kind: r.kind.as_str().to_string(),
        target_task_id: r.target_task_id.map(|id| id.0),
        target_readable_id: None,
        target_document_id: r.target_document_id.map(|id| id.0),
        target_resolved: r.target_task_id.is_some() || r.target_document_id.is_some(),
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

fn assignee_to_dto(a: TaskAssignee) -> AssigneeDto {
    AssigneeDto {
        assignee: actor_to_dto(&assignee_ref_to_actor(a.assignee)),
        assigned_by: actor_to_dto(&a.assigned_by),
        assigned_at: a.assigned_at,
    }
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

    let task = state
        .task_service()
        .create(
            &ctx,
            NewTask {
                project_id: board.project_id,
                board_id: board.id,
                column_id: ColumnId(body.column_id),
                title: body.title,
                description: body.description.unwrap_or_default(),
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

    Ok((StatusCode::CREATED, Json(task_to_dto(task))))
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
        (status = 200, description = "Paginated task list"),
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

    let dtos = all
        .into_iter()
        .map(|t| TaskSummaryDto {
            id: t.id.0,
            readable_id: t.readable_id,
            column_id: t.column_id.0,
            title: t.title,
            priority: t.priority.map(|p| p.as_str().to_string()),
            updated_at: t.updated_at,
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
) -> Result<Json<TaskDto>, ApiError> {
    Ok(Json(task_to_dto(auth.resource.0)))
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

    let priority = parse_priority(body.priority)?;
    let due_date = parse_due_date(body.due_date)?;
    let estimate = parse_estimate(body.estimate)?;

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

    Ok(Json(task_to_dto(updated)))
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

    Ok(Json(task_to_dto(moved)))
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
        (status = 200, description = "Assignee list"),
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

    Ok(Json(items.into_iter().map(assignee_to_dto).collect()))
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

    Ok((StatusCode::CREATED, Json(assignee_to_dto(result))))
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
        (status = 200, description = "Reference list"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_references(
    auth: Authorized<TaskRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ReferenceDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskReferenceRepo::new((*state.db).clone());

    let refs = repo
        .list_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(refs.into_iter().map(reference_to_dto).collect()))
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

    let target_task_id = if let Some(rid) = body.target_task_readable_id {
        let repo = PgTaskRepo::new((*state.db).clone());
        repo.find_by_readable_id(&ctx, &rid)
            .await
            .map_err(ApiError::Domain)?
            .map(|t| t.id)
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
                target_document_id: body.target_document_id.map(atlas_domain::ids::DocumentId),
            },
        )
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::Forbidden { .. } => ApiError::Conflict,
            other => ApiError::Domain(other),
        })?;

    Ok((StatusCode::CREATED, Json(reference_to_dto(reference))))
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
    use atlas_domain::entities::boards_tasks::ReferenceKind;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let ref_id = TaskReferenceId(p.reference_id);

    state
        .task_service()
        .remove_reference(&ctx, auth.resource.0.id, ref_id, ReferenceKind::Relates)
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
        (status = 200, description = "Checklist items"),
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

    let result = state
        .task_service()
        .promote_checklist_item(
            &ctx,
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

    let dto = PromotionDto {
        task: task_to_dto(result.task),
        parent_reference: result.parent_reference.map(reference_to_dto),
        checklist_item: checklist_item_to_dto(result.checklist_item),
    };

    Ok((StatusCode::CREATED, Json(dto)))
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
        (status = 200, description = "Activity feed"),
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
