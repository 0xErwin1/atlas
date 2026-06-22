use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::dtos::status_templates::{
    CreateStatusTemplateRequest, StatusTemplateDto, UpdateStatusTemplateRequest,
};
use atlas_domain::{
    Actor, StatusTemplateId, WorkspaceCtx,
    entities::boards_tasks::{BoardColumn, PositionBetween},
    entities::status_templates::{NewStatusTemplate, StatusTemplate, StatusTemplatePatch},
    permissions::Principal,
};

use crate::{
    authz::{Authorized, BoardRes, EditorMin, ViewerMin, authorized::WorkspaceRes},
    error::ApiError,
    persistence::repos::{
        BoardRepo, PgBoardRepo, PgStatusTemplateRepo, StatusTemplateRepo,
        list_templates_for_workspace,
    },
    routes::validation::{validate_name, validate_swatch},
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct TemplatePath {
    #[allow(dead_code)]
    ws: String,
    template_id: uuid::Uuid,
}

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    }
}

fn template_to_dto(t: StatusTemplate) -> StatusTemplateDto {
    StatusTemplateDto {
        id: t.id.0,
        workspace_id: t.workspace_id.0,
        name: t.name,
        color: t.color,
        position_key: t.position_key,
        created_at: t.created_at,
        updated_at: t.updated_at,
    }
}

fn column_to_dto(c: BoardColumn) -> atlas_api::dtos::boards_tasks::ColumnDto {
    atlas_api::dtos::boards_tasks::ColumnDto {
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
// GET /v1/workspaces/{ws}/status-templates
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/status-templates",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 200, description = "Workspace status templates ordered by position", body = [StatusTemplateDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_status_templates(
    auth: Authorized<WorkspaceRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<StatusTemplateDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgStatusTemplateRepo::new((*state.db).clone());

    let templates = repo.list(&ctx).await.map_err(ApiError::Domain)?;

    Ok(Json(templates.into_iter().map(template_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/status-templates
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/status-templates",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateStatusTemplateRequest,
    responses(
        (status = 201, description = "Status template created", body = StatusTemplateDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn create_status_template(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateStatusTemplateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;

    if let Some(ref color) = body.color {
        validate_swatch("color", color)?;
    }

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let existing = PgStatusTemplateRepo::new((*state.db).clone())
        .list(&ctx)
        .await
        .map_err(ApiError::Domain)?;

    let last_key = existing.last().map(|t| t.position_key.clone());
    let position_key = atlas_domain::position::between(last_key.as_deref(), None);

    let repo = PgStatusTemplateRepo::new((*state.db).clone());
    let template = repo
        .create(
            &ctx,
            NewStatusTemplate {
                name: body.name,
                color: body.color,
                position_key,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(template_to_dto(template))))
}

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/status-templates/{template_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/status-templates/{template_id}",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("template_id" = uuid::Uuid, Path, description = "Template ID"),
    ),
    request_body = UpdateStatusTemplateRequest,
    responses(
        (status = 200, description = "Status template updated", body = StatusTemplateDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Template not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn update_status_template(
    auth: Authorized<WorkspaceRes, EditorMin>,
    Path(p): Path<TemplatePath>,
    State(state): State<AppState>,
    Json(body): Json<UpdateStatusTemplateRequest>,
) -> Result<Json<StatusTemplateDto>, ApiError> {
    if let Some(ref name) = body.name {
        validate_name("name", name)?;
    }

    let color_patch: Option<Option<String>> = match body.color {
        None => None,
        Some(serde_json::Value::Null) => Some(None),
        Some(serde_json::Value::String(s)) => {
            validate_swatch("color", &s)?;
            Some(Some(s))
        }
        Some(other) => {
            return Err(ApiError::InvalidInput {
                message: format!("color must be a swatch id string or null, got {other}"),
            });
        }
    };

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let id = StatusTemplateId(p.template_id);
    let repo = PgStatusTemplateRepo::new((*state.db).clone());

    if body.before.is_some() || body.after.is_some() {
        repo.move_template(
            &ctx,
            id,
            PositionBetween {
                before: body.before,
                after: body.after,
            },
        )
        .await
        .map_err(ApiError::Domain)?;
    }

    let has_patch = body.name.is_some() || color_patch.is_some();

    let template = if has_patch {
        repo.patch(
            &ctx,
            id,
            StatusTemplatePatch {
                name: body.name,
                color: color_patch,
            },
        )
        .await
        .map_err(ApiError::Domain)?
    } else {
        let list = repo.list(&ctx).await.map_err(ApiError::Domain)?;
        list.into_iter()
            .find(|t| t.id == id)
            .ok_or(ApiError::NotFound)?
    };

    Ok(Json(template_to_dto(template)))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/status-templates/{template_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/status-templates/{template_id}",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("template_id" = uuid::Uuid, Path, description = "Template ID"),
    ),
    responses(
        (status = 204, description = "Template soft-deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Template not found"),
    )
)]
pub(crate) async fn delete_status_template(
    auth: Authorized<WorkspaceRes, EditorMin>,
    Path(p): Path<TemplatePath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgStatusTemplateRepo::new((*state.db).clone());

    repo.soft_delete(&ctx, StatusTemplateId(p.template_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/boards/{board_id}/apply-status-templates
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/boards/{board_id}/apply-status-templates",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = uuid::Uuid, Path, description = "Board UUID"),
    ),
    responses(
        (status = 200, description = "Resulting column list after apply", body = [atlas_api::dtos::boards_tasks::ColumnDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn apply_status_templates(
    auth: Authorized<BoardRes, EditorMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<atlas_api::dtos::boards_tasks::ColumnDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let board_id = auth.resource.0.id;

    let board_repo = PgBoardRepo::new((*state.db).clone());

    let existing_cols = board_repo
        .list_columns(&ctx, board_id)
        .await
        .map_err(ApiError::Domain)?;

    let templates = list_templates_for_workspace(&*state.db, ctx.workspace_id.0)
        .await
        .map_err(ApiError::Domain)?;

    let existing_names_lower: std::collections::HashSet<String> = existing_cols
        .iter()
        .map(|c| c.name.to_lowercase())
        .collect();

    let last_key = existing_cols.last().map(|c| c.position_key.clone());

    let mut prev_key = last_key;

    for tpl in templates {
        if existing_names_lower.contains(&tpl.name.to_lowercase()) {
            continue;
        }

        let added = board_repo
            .add_column(
                &ctx,
                board_id,
                tpl.name,
                tpl.color,
                PositionBetween {
                    before: prev_key.clone(),
                    after: None,
                },
            )
            .await
            .map_err(ApiError::Domain)?;

        prev_key = Some(added.position_key);
    }

    let final_cols = board_repo
        .list_columns(&ctx, board_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(final_cols.into_iter().map(column_to_dto).collect()))
}
