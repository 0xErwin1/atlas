use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::status_templates::{
    CreateStatusTemplateRequest, PlatformStatusTemplateDto, UpdateStatusTemplateRequest,
};
use atlas_domain::{
    PlatformStatusTemplateId,
    entities::boards_tasks::PositionBetween,
    entities::status_templates::{NewStatusTemplate, PlatformStatusTemplate, StatusTemplatePatch},
};

use crate::{
    authz::RequireUserAdmin,
    error::ApiError,
    persistence::repos::{PgPlatformStatusTemplateRepo, PlatformStatusTemplateRepo},
    routes::validation::{validate_name, validate_swatch},
    state::AppState,
};

fn template_to_dto(t: PlatformStatusTemplate) -> PlatformStatusTemplateDto {
    PlatformStatusTemplateDto {
        id: t.id.0,
        name: t.name,
        color: t.color,
        position_key: t.position_key,
        created_at: t.created_at,
        updated_at: t.updated_at,
    }
}

// ---------------------------------------------------------------------------
// GET /api/admin/status-templates
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/admin/status-templates",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Atlas default statuses ordered by position", body = [PlatformStatusTemplateDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Admin access required"),
    )
)]
/// Lists the Atlas-wide default statuses new workspaces are seeded from.
pub(crate) async fn list_platform_status_templates(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<PlatformStatusTemplateDto>>, ApiError> {
    let templates = PgPlatformStatusTemplateRepo::new((*state.db).clone())
        .list()
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(templates.into_iter().map(template_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// POST /api/admin/status-templates
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/admin/status-templates",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    request_body = CreateStatusTemplateRequest,
    responses(
        (status = 201, description = "Atlas default status created", body = PlatformStatusTemplateDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Admin access required"),
        (status = 422, description = "Invalid input"),
    )
)]
/// Appends an Atlas-wide default status after the current last one.
///
/// The `before`/`after` anchors of the shared request body are ignored here:
/// creation always appends, and reordering is a PATCH.
pub(crate) async fn create_platform_status_template(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Json(body): Json<CreateStatusTemplateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;

    if let Some(ref color) = body.color {
        validate_swatch("color", color)?;
    }

    let repo = PgPlatformStatusTemplateRepo::new((*state.db).clone());

    let existing = repo.list().await.map_err(ApiError::Domain)?;
    let last_key = existing.last().map(|t| t.position_key.clone());
    let position_key = atlas_domain::position::between(last_key.as_deref(), None);

    let template = repo
        .create(NewStatusTemplate {
            name: body.name,
            color: body.color,
            position_key,
        })
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(template_to_dto(template))))
}

// ---------------------------------------------------------------------------
// PATCH /api/admin/status-templates/{template_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/admin/status-templates/{template_id}",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    params(("template_id" = uuid::Uuid, Path, description = "Template ID")),
    request_body = UpdateStatusTemplateRequest,
    responses(
        (status = 200, description = "Atlas default status updated", body = PlatformStatusTemplateDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Template not found"),
        (status = 422, description = "Invalid input"),
    )
)]
/// Renames, recolors, and/or reorders one Atlas-wide default status.
pub(crate) async fn update_platform_status_template(
    _admin: RequireUserAdmin,
    Path(template_id): Path<uuid::Uuid>,
    State(state): State<AppState>,
    Json(body): Json<UpdateStatusTemplateRequest>,
) -> Result<Json<PlatformStatusTemplateDto>, ApiError> {
    if let Some(ref name) = body.name {
        validate_name("name", name)?;
    }

    let color_patch = parse_color_patch(body.color)?;

    let id = PlatformStatusTemplateId(template_id);
    let repo = PgPlatformStatusTemplateRepo::new((*state.db).clone());

    if body.before.is_some() || body.after.is_some() {
        repo.move_template(
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
            id,
            StatusTemplatePatch {
                name: body.name,
                color: color_patch,
            },
        )
        .await
        .map_err(ApiError::Domain)?
    } else {
        let list = repo.list().await.map_err(ApiError::Domain)?;
        list.into_iter()
            .find(|t| t.id == id)
            .ok_or(ApiError::NotFound)?
    };

    Ok(Json(template_to_dto(template)))
}

/// Maps the tri-state `color` field of the shared PATCH body to a repo patch:
/// absent = leave unchanged, explicit `null` = clear, string = set.
fn parse_color_patch(color: Option<serde_json::Value>) -> Result<Option<Option<String>>, ApiError> {
    match color {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(s)) => {
            validate_swatch("color", &s)?;
            Ok(Some(Some(s)))
        }
        Some(other) => Err(ApiError::InvalidInput {
            message: format!("color must be a swatch id string or null, got {other}"),
        }),
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/admin/status-templates/{template_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/admin/status-templates/{template_id}",
    tag = "status-templates",
    security(("bearer_auth" = [])),
    params(("template_id" = uuid::Uuid, Path, description = "Template ID")),
    responses(
        (status = 204, description = "Atlas default status soft-deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Admin access required"),
        (status = 404, description = "Template not found"),
    )
)]
/// Soft-deletes one Atlas-wide default status. Existing workspaces keep the
/// status templates they were seeded with.
pub(crate) async fn delete_platform_status_template(
    _admin: RequireUserAdmin,
    Path(template_id): Path<uuid::Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    PgPlatformStatusTemplateRepo::new((*state.db).clone())
        .soft_delete(PlatformStatusTemplateId(template_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
