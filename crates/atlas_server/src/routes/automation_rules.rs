use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

use atlas_api::{
    dtos::automation_rules::{
        AutomationRuleDto, CreateAutomationRuleRequest, PatchAutomationRuleRequest,
    },
    pagination::{Cursor, Page},
};
use atlas_domain::permissions::Principal;

use crate::{
    authz::{AdminMin, Authorized, WorkspaceRes},
    error::ApiError,
    persistence::{entities::automation_rule::automation_rules, repos::PgAutomationRuleRepo},
    state::AppState,
};

const PAGE_LIMIT: u64 = 50;

fn row_to_dto(row: automation_rules::Model) -> AutomationRuleDto {
    AutomationRuleDto {
        id: row.id,
        workspace_id: row.workspace_id,
        name: row.name,
        is_active: row.is_active,
        trigger_event_type: row.trigger_event_type,
        trigger_filter: row.trigger_filter,
        project_id: row.project_id,
        action_type: row.action_type,
        action_params: row.action_params,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Validates the create request at the handler boundary.
///
/// `trigger_event_type` prefix check is also enforced at the repo layer, but
/// validating here lets us return a user-facing 422 before touching the DB.
/// `action_type` and `action_params` shape are handler-only checks.
fn validate_create(req: &CreateAutomationRuleRequest) -> Result<(), ApiError> {
    if !req.trigger_event_type.starts_with("external.") {
        return Err(ApiError::InvalidInput {
            message: format!(
                "trigger_event_type must start with 'external.', got: {}",
                req.trigger_event_type
            ),
        });
    }

    if req.project_id.is_some() {
        return Err(ApiError::InvalidInput {
            message: "external automation rules must be workspace-scoped in v1; omit project_id"
                .into(),
        });
    }

    match req.action_type.as_str() {
        "create_task" => require_action_params(
            &req.action_params,
            "create_task",
            &["board_id", "column_id", "title_template"],
        ),
        "add_comment" => require_action_params(
            &req.action_params,
            "add_comment",
            &["task_id", "body_template"],
        ),
        other => Err(ApiError::InvalidInput {
            message: format!(
                "unsupported action_type '{other}'; supported: 'create_task', 'add_comment'"
            ),
        }),
    }
}

/// Rejects an `action_params` object that is missing any of the fields the given
/// action requires, so a misconfigured rule fails with a 422 at the request
/// boundary instead of silently no-op'ing later during event processing.
fn require_action_params(
    params: &serde_json::Value,
    action: &str,
    required: &[&str],
) -> Result<(), ApiError> {
    let missing = required
        .iter()
        .filter(|k| params.get(*k).is_none())
        .cloned()
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        return Err(ApiError::InvalidInput {
            message: format!(
                "action_params for '{action}' is missing required fields: {}",
                missing.join(", ")
            ),
        });
    }

    Ok(())
}

fn principal_user_id(auth: &Authorized<WorkspaceRes, AdminMin>) -> Result<Uuid, ApiError> {
    match &auth.principal {
        Principal::User(uid) => Ok(uid.0),
        _ => Err(ApiError::InvalidInput {
            message: "only user principals can manage automation rules".into(),
        }),
    }
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/automation-rules
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/automation-rules",
    tag = "automation-rules",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateAutomationRuleRequest,
    responses(
        (status = 201, description = "Automation rule created", body = AutomationRuleDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not an admin"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn create_automation_rule(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateAutomationRuleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_create(&body)?;

    let ws_id = auth.workspace.id.0;
    let user_id = principal_user_id(&auth)?;

    let row = PgAutomationRuleRepo::create(
        &*state.db,
        ws_id,
        body.name,
        body.trigger_event_type,
        body.trigger_filter,
        body.project_id,
        body.action_type,
        body.action_params,
        user_id,
    )
    .await
    .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(row_to_dto(row))))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/automation-rules
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct ListAutomationRulesQuery {
    after: Option<String>,
    limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/automation-rules",
    tag = "automation-rules",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("after" = Option<String>, Query, description = "Opaque cursor for the next page"),
        ("limit" = Option<u64>, Query, description = "Page size, max 50"),
    ),
    responses(
        (status = 200, description = "Paginated list of automation rules", body = Page<AutomationRuleDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not an admin"),
    )
)]
pub(crate) async fn list_automation_rules(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Query(params): Query<ListAutomationRulesQuery>,
) -> Result<Json<Page<AutomationRuleDto>>, ApiError> {
    let ws_id = auth.workspace.id.0;

    let after_id = params
        .after
        .as_deref()
        .map(|s| {
            Cursor::decode(s)
                .map(|c| c.0)
                .ok_or_else(|| ApiError::BadRequest {
                    message: "invalid cursor".into(),
                })
        })
        .transpose()?;

    let limit = params.limit.unwrap_or(PAGE_LIMIT).min(PAGE_LIMIT);
    let fetch_limit = limit + 1;

    let rows = PgAutomationRuleRepo::list(&*state.db, ws_id, after_id, fetch_limit)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = rows.len() as u64 > limit;
    let trimmed: Vec<_> = rows.into_iter().take(limit as usize).collect();
    let next_cursor = if has_more {
        trimmed.last().map(|r| Cursor(r.id))
    } else {
        None
    };

    let items = trimmed.into_iter().map(row_to_dto).collect();

    Ok(Json(Page::new(items, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/automation-rules/{rule_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/automation-rules/{rule_id}",
    tag = "automation-rules",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("rule_id" = Uuid, Path, description = "Automation rule id"),
    ),
    responses(
        (status = 200, description = "Automation rule", body = AutomationRuleDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Rule not found or caller is not an admin"),
    )
)]
pub(crate) async fn get_automation_rule(
    auth: Authorized<WorkspaceRes, AdminMin>,
    Path((_ws, rule_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
) -> Result<Json<AutomationRuleDto>, ApiError> {
    let ws_id = auth.workspace.id.0;

    let row = PgAutomationRuleRepo::get(&*state.db, ws_id, rule_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(row_to_dto(row)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/automation-rules/{rule_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/automation-rules/{rule_id}",
    tag = "automation-rules",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("rule_id" = Uuid, Path, description = "Automation rule id"),
    ),
    request_body = PatchAutomationRuleRequest,
    responses(
        (status = 200, description = "Updated automation rule", body = AutomationRuleDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Rule not found or caller is not an admin"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn patch_automation_rule(
    auth: Authorized<WorkspaceRes, AdminMin>,
    Path((_ws, rule_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    Json(body): Json<PatchAutomationRuleRequest>,
) -> Result<Json<AutomationRuleDto>, ApiError> {
    let ws_id = auth.workspace.id.0;

    let patch = crate::persistence::repos::AutomationRulePatch {
        name: body.name,
        is_active: body.is_active,
        trigger_filter: body.trigger_filter,
        action_params: body.action_params,
    };

    let row = PgAutomationRuleRepo::patch(&*state.db, ws_id, rule_id, patch)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(row_to_dto(row)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/automation-rules/{rule_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/automation-rules/{rule_id}",
    tag = "automation-rules",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("rule_id" = Uuid, Path, description = "Automation rule id"),
    ),
    responses(
        (status = 204, description = "Automation rule soft-deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Rule not found or caller is not an admin"),
    )
)]
pub(crate) async fn delete_automation_rule(
    auth: Authorized<WorkspaceRes, AdminMin>,
    Path((_ws, rule_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ws_id = auth.workspace.id.0;

    PgAutomationRuleRepo::soft_delete(&*state.db, ws_id, rule_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
