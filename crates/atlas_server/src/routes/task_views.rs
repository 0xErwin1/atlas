use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::task_views::{
    CreateTaskViewRequest, TaskViewDto, TaskViewFiltersDto, UpdateTaskViewRequest,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::Priority,
    entities::task_views::{
        ActorTypeFilter, AssigneeFilter, NewTaskView, TaskSort, TaskView, TaskViewFilters,
    },
    ids::{ApiKeyId, BoardId, ColumnId, UserId},
};

use crate::{
    authz::WorkspaceMember,
    error::ApiError,
    persistence::repos::{PgTaskViewRepo, TaskViewRepo},
    routes::validation::{validate_name, validate_task_view_filters},
    state::AppState,
};

fn actor_from_member(member: &WorkspaceMember) -> Result<Actor, ApiError> {
    match (&member.user, &member.api_key_id) {
        (Some(user), _) => Ok(Actor::User(user.id)),
        (None, Some(key_id)) => Ok(Actor::ApiKey(*key_id)),
        (None, None) => Err(ApiError::Unauthorized),
    }
}

fn filters_dto_to_domain(dto: TaskViewFiltersDto) -> Result<TaskViewFilters, ApiError> {
    let sort = dto
        .sort
        .as_deref()
        .map(|s| {
            TaskSort::from_param_str(s).ok_or_else(|| ApiError::InvalidInput {
                message: format!("invalid sort key '{s}'"),
            })
        })
        .transpose()?;

    let priorities = dto
        .priorities
        .iter()
        .map(|p| {
            p.parse::<Priority>().map_err(|_| ApiError::InvalidInput {
                message: format!("invalid priority '{p}'"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let column_ids = dto.column_ids.into_iter().map(ColumnId).collect();

    let board_id = dto.board_id.map(BoardId);

    let assignee = dto
        .assignee
        .as_deref()
        .map(parse_assignee_filter)
        .transpose()?;

    let actor_type = dto
        .actor_type
        .as_deref()
        .map(|s| match s {
            "user" => Ok(ActorTypeFilter::User),
            "api_key" => Ok(ActorTypeFilter::ApiKey),
            other => Err(ApiError::InvalidInput {
                message: format!("invalid actor_type '{other}'; must be 'user' or 'api_key'"),
            }),
        })
        .transpose()?;

    Ok(TaskViewFilters {
        sort,
        priorities,
        column_ids,
        labels: dto.labels,
        board_id,
        assignee,
        actor_type,
    })
}

fn parse_assignee_filter(s: &str) -> Result<AssigneeFilter, ApiError> {
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

fn filters_domain_to_dto(filters: TaskViewFilters) -> TaskViewFiltersDto {
    let sort = filters.sort.map(|s| s.as_param_str().to_string());

    let priorities = filters
        .priorities
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();

    let column_ids = filters.column_ids.into_iter().map(|c| c.0).collect();

    let board_id = filters.board_id.map(|b| b.0);

    let assignee = filters.assignee.map(|a| match a {
        AssigneeFilter::Me => "me".to_string(),
        AssigneeFilter::User(uid) => format!("user:{}", uid.0),
        AssigneeFilter::ApiKey(kid) => format!("api_key:{}", kid.0),
    });

    let actor_type = filters.actor_type.map(|a| match a {
        ActorTypeFilter::User => "user".to_string(),
        ActorTypeFilter::ApiKey => "api_key".to_string(),
    });

    TaskViewFiltersDto {
        sort,
        priorities,
        labels: filters.labels,
        column_ids,
        board_id,
        assignee,
        actor_type,
    }
}

fn task_view_to_dto(tv: TaskView) -> TaskViewDto {
    TaskViewDto {
        id: tv.id.0,
        workspace_id: tv.workspace_id.0,
        name: tv.name,
        filters: filters_domain_to_dto(tv.filters),
        created_at: tv.created_at,
        updated_at: tv.updated_at,
    }
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/task-views
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/task-views",
    tag = "task-views",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 200, description = "Caller's task views sorted by name (case-insensitive)", body = [TaskViewDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
    )
)]
pub(crate) async fn list_task_views(
    member: WorkspaceMember,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskViewDto>>, ApiError> {
    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgTaskViewRepo::new((*state.db).clone());

    let views = repo.list_for_owner(&ctx).await.map_err(ApiError::Domain)?;

    Ok(Json(views.into_iter().map(task_view_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/task-views
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/task-views",
    tag = "task-views",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateTaskViewRequest,
    responses(
        (status = 201, description = "Task view created", body = TaskViewDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
        (status = 409, description = "A task view with this name already exists for this owner"),
        (status = 422, description = "Validation error or per-owner cap exceeded"),
    )
)]
pub(crate) async fn create_task_view(
    member: WorkspaceMember,
    State(state): State<AppState>,
    Json(body): Json<CreateTaskViewRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;
    validate_task_view_filters(&body.filters)?;

    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgTaskViewRepo::new((*state.db).clone());

    let name = body.name.trim().to_string();
    let filters = filters_dto_to_domain(body.filters)?;

    let view = repo
        .create(&ctx, NewTaskView { name, filters })
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(task_view_to_dto(view))))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/task-views/{id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/task-views/{id}",
    tag = "task-views",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("id" = uuid::Uuid, Path, description = "Task view id"),
    ),
    responses(
        (status = 200, description = "Task view with filters", body = TaskViewDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Task view not found, not owned by caller, or caller is not a member"),
    )
)]
pub(crate) async fn get_task_view(
    member: WorkspaceMember,
    Path((_ws, id)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
) -> Result<Json<TaskViewDto>, ApiError> {
    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgTaskViewRepo::new((*state.db).clone());

    let view = repo
        .find(&ctx, id.into())
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::Domain(atlas_domain::DomainError::NotFound {
            entity: "task_view",
            id,
        }))?;

    Ok(Json(task_view_to_dto(view)))
}

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/task-views/{id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/task-views/{id}",
    tag = "task-views",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("id" = uuid::Uuid, Path, description = "Task view id"),
    ),
    request_body = UpdateTaskViewRequest,
    responses(
        (status = 200, description = "Task view updated", body = TaskViewDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Task view not found, not owned by caller, or caller is not a member"),
        (status = 409, description = "A task view with this name already exists for this owner"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn update_task_view(
    member: WorkspaceMember,
    Path((_ws, id)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
    Json(body): Json<UpdateTaskViewRequest>,
) -> Result<Json<TaskViewDto>, ApiError> {
    validate_name("name", &body.name)?;
    validate_task_view_filters(&body.filters)?;

    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgTaskViewRepo::new((*state.db).clone());

    let name = body.name.trim().to_string();
    let filters = filters_dto_to_domain(body.filters)?;

    let view = repo
        .update(&ctx, id.into(), name, filters)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_view_to_dto(view)))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/task-views/{id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/task-views/{id}",
    tag = "task-views",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("id" = uuid::Uuid, Path, description = "Task view id"),
    ),
    responses(
        (status = 204, description = "Task view deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Task view not found, not owned by caller, or caller is not a member"),
    )
)]
pub(crate) async fn delete_task_view(
    member: WorkspaceMember,
    Path((_ws, id)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgTaskViewRepo::new((*state.db).clone());

    repo.delete(&ctx, id.into())
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
