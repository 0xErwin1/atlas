use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use rand::RngCore;
use rand::rngs::OsRng;
use serde::Deserialize;
use uuid::Uuid;

use atlas_api::{
    dtos::webhooks::{
        CreateWebhookRequest, UpdateWebhookRequest, WebhookCreatedDto, WebhookDeliveryDto,
        WebhookDto,
    },
    pagination::{Cursor, Page},
};
use atlas_domain::permissions::Principal;

use crate::{
    authz::{
        AdminMinAgentEditor, Authorized, WebhooksCreate, WebhooksDelete, WebhooksRead,
        WebhooksUpdate, WorkspaceRes,
    },
    error::ApiError,
    persistence::{
        entities::webhook_delivery::webhook_delivery_log,
        entities::webhook_subscription::webhook_subscriptions,
        repos::{PgWebhookDeliveryRepo, PgWebhookSubscriptionRepo, WebhookSubscriptionPatch},
    },
    state::AppState,
};

const PAGE_LIMIT: u64 = 50;

/// Known event-type strings from the domain catalog.
const KNOWN_EVENT_TYPES: &[&str] = &[
    "task.created",
    "task.updated",
    "task.moved",
    "task.deleted",
    "document.created",
    "document.updated",
    "document.moved",
    "document.deleted",
    "board.created",
    "board.updated",
    "board.deleted",
    "board.moved",
    "column.created",
    "column.deleted",
    "folder.created",
    "folder.deleted",
];

fn principal_to_actor(p: &Principal) -> atlas_domain::Actor {
    match p {
        Principal::User(uid) => atlas_domain::Actor::User(*uid),
        Principal::ApiKey(kid) => atlas_domain::Actor::ApiKey(*kid),
        Principal::Group(_) => atlas_domain::Actor::User(atlas_domain::ids::UserId(Uuid::nil())),
    }
}

fn row_to_dto(row: webhook_subscriptions::Model) -> WebhookDto {
    WebhookDto {
        id: row.id,
        workspace_id: row.workspace_id,
        target_url: row.target_url,
        event_types: row.event_types,
        scope_type: row.scope_type,
        scope_id: row.scope_id,
        is_active: row.is_active,
        label: row.label,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn delivery_row_to_dto(row: webhook_delivery_log::Model) -> WebhookDeliveryDto {
    WebhookDeliveryDto {
        id: row.id,
        subscription_id: row.subscription_id,
        outbox_event_id: row.outbox_event_id,
        attempt_no: row.attempt_no,
        outcome: row.outcome,
        status_code: row.status_code,
        response_snippet: row.response_snippet,
        error: row.error,
        duration_ms: row.duration_ms,
        created_at: row.created_at,
    }
}

/// Validates that `event_types` is non-empty and all values are in the catalog.
fn validate_event_types(event_types: &[String]) -> Result<(), ApiError> {
    if event_types.is_empty() {
        return Err(ApiError::InvalidInput {
            message: "event_types must contain at least one event type".into(),
        });
    }

    for et in event_types {
        if !KNOWN_EVENT_TYPES.contains(&et.as_str()) {
            return Err(ApiError::InvalidInput {
                message: format!(
                    "unknown event type '{et}'; valid types: {}",
                    KNOWN_EVENT_TYPES.join(", ")
                ),
            });
        }
    }

    Ok(())
}

/// Validates scope_type + scope_id coherence:
/// - `"workspace"` requires `scope_id` absent.
/// - `"project"` and `"board"` require `scope_id` present.
fn validate_scope(scope_type: &str, scope_id: Option<Uuid>) -> Result<(), ApiError> {
    match scope_type {
        "workspace" => {
            if scope_id.is_some() {
                return Err(ApiError::InvalidInput {
                    message: "scope_id must be absent when scope_type is 'workspace'".into(),
                });
            }
        }
        "project" | "board" => {
            if scope_id.is_none() {
                return Err(ApiError::InvalidInput {
                    message: format!("scope_id is required when scope_type is '{scope_type}'"),
                });
            }
        }
        other => {
            return Err(ApiError::InvalidInput {
                message: format!(
                    "unknown scope_type '{other}'; valid values: workspace, project, board"
                ),
            });
        }
    }

    Ok(())
}

/// Generates a fresh HMAC signing secret as `whsec_<64-lowercase-hex-chars>`.
fn generate_secret() -> String {
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    format!("whsec_{hex}")
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/webhooks
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/webhooks",
    tag = "webhooks",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateWebhookRequest,
    responses(
        (status = 201, description = "Subscription created; secret is returned here and never again", body = WebhookCreatedDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a workspace admin or owner"),
        (status = 404, description = "Workspace not found or caller is not a member"),
        (status = 422, description = "Validation error (invalid event types, scope, or URL)"),
    )
)]
pub(crate) async fn create_webhook(
    auth: Authorized<WorkspaceRes, AdminMinAgentEditor, WebhooksCreate>,
    State(state): State<AppState>,
    Json(body): Json<CreateWebhookRequest>,
) -> Result<impl IntoResponse, ApiError> {
    crate::webhook_url::validate_target_url(&body.target_url, state.allow_private_webhook_targets)
        .await?;
    validate_event_types(&body.event_types)?;
    validate_scope(&body.scope_type, body.scope_id)?;

    let actor = principal_to_actor(&auth.principal);
    let ws_id = auth.workspace.id.0;

    let plaintext_secret = generate_secret();
    let (encrypted_secret, secret_nonce) = state
        .webhook_crypto
        .encrypt(plaintext_secret.as_bytes())
        .map_err(|e| ApiError::Internal { message: e })?;

    let row = PgWebhookSubscriptionRepo::create(
        &*state.db,
        ws_id,
        body.target_url,
        body.event_types,
        body.scope_type,
        body.scope_id,
        encrypted_secret,
        secret_nonce,
        body.label,
        &actor,
    )
    .await
    .map_err(ApiError::Domain)?;

    let dto = WebhookCreatedDto {
        id: row.id,
        workspace_id: row.workspace_id,
        target_url: row.target_url,
        event_types: row.event_types,
        scope_type: row.scope_type,
        scope_id: row.scope_id,
        is_active: row.is_active,
        label: row.label,
        secret: plaintext_secret,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok((StatusCode::CREATED, Json(dto)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/webhooks
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct ListWebhooksQuery {
    after: Option<String>,
    limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/webhooks",
    tag = "webhooks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("after" = Option<String>, Query, description = "Opaque cursor for the next page"),
        ("limit" = Option<u64>, Query, description = "Page size, max 50"),
    ),
    responses(
        (status = 200, description = "Paginated list of subscriptions (no secret)", body = Page<WebhookDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a workspace admin or owner"),
        (status = 404, description = "Workspace not found or caller is not a member"),
    )
)]
pub(crate) async fn list_webhooks(
    auth: Authorized<WorkspaceRes, AdminMinAgentEditor, WebhooksRead>,
    State(state): State<AppState>,
    Query(params): Query<ListWebhooksQuery>,
) -> Result<Json<Page<WebhookDto>>, ApiError> {
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

    let rows = PgWebhookSubscriptionRepo::list_active(&*state.db, ws_id, after_id, fetch_limit)
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
// GET /api/workspaces/{ws}/webhooks/{webhook_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/webhooks/{webhook_id}",
    tag = "webhooks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("webhook_id" = Uuid, Path, description = "Subscription id"),
    ),
    responses(
        (status = 200, description = "Webhook subscription (no secret)", body = WebhookDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a workspace admin or owner"),
        (status = 404, description = "Workspace or subscription not found"),
    )
)]
pub(crate) async fn get_webhook(
    auth: Authorized<WorkspaceRes, AdminMinAgentEditor, WebhooksRead>,
    Path((_ws, webhook_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
) -> Result<Json<WebhookDto>, ApiError> {
    let ws_id = auth.workspace.id.0;

    let row = PgWebhookSubscriptionRepo::get_by_id(&*state.db, ws_id, webhook_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(row_to_dto(row)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/webhooks/{webhook_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/webhooks/{webhook_id}",
    tag = "webhooks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("webhook_id" = Uuid, Path, description = "Subscription id"),
    ),
    request_body = UpdateWebhookRequest,
    responses(
        (status = 200, description = "Updated subscription (no secret)", body = WebhookDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a workspace admin or owner"),
        (status = 404, description = "Workspace or subscription not found"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn update_webhook(
    auth: Authorized<WorkspaceRes, AdminMinAgentEditor, WebhooksUpdate>,
    Path((_ws, webhook_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    Json(body): Json<UpdateWebhookRequest>,
) -> Result<Json<WebhookDto>, ApiError> {
    if let Some(url) = &body.target_url {
        crate::webhook_url::validate_target_url(url, state.allow_private_webhook_targets).await?;
    }

    if let Some(types) = &body.event_types {
        validate_event_types(types)?;
    }

    if let Some(scope_type) = &body.scope_type {
        let scope_id = body.scope_id.and_then(|inner| inner);
        validate_scope(scope_type, scope_id)?;
    }

    let ws_id = auth.workspace.id.0;

    let patch = WebhookSubscriptionPatch {
        target_url: body.target_url,
        event_types: body.event_types,
        scope_type: body.scope_type,
        scope_id: body.scope_id,
        encrypted_secret: None,
        secret_nonce: None,
        is_active: body.is_active,
        label: body.label,
    };

    let row = PgWebhookSubscriptionRepo::update(&*state.db, ws_id, webhook_id, patch)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(row_to_dto(row)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/webhooks/{webhook_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/webhooks/{webhook_id}",
    tag = "webhooks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("webhook_id" = Uuid, Path, description = "Subscription id"),
    ),
    responses(
        (status = 204, description = "Subscription soft-deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a workspace admin or owner"),
        (status = 404, description = "Workspace or subscription not found"),
    )
)]
pub(crate) async fn delete_webhook(
    auth: Authorized<WorkspaceRes, AdminMinAgentEditor, WebhooksDelete>,
    Path((_ws, webhook_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ws_id = auth.workspace.id.0;

    PgWebhookSubscriptionRepo::soft_delete(&*state.db, ws_id, webhook_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/webhooks/{webhook_id}/deliveries
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct ListDeliveriesQuery {
    before: Option<String>,
    limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/webhooks/{webhook_id}/deliveries",
    tag = "webhooks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("webhook_id" = Uuid, Path, description = "Subscription id"),
        ("before" = Option<String>, Query, description = "Opaque cursor (newest-first paging)"),
        ("limit" = Option<u64>, Query, description = "Page size, max 50"),
    ),
    responses(
        (status = 200, description = "Delivery attempts, newest first", body = Page<WebhookDeliveryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a workspace admin or owner"),
        (status = 404, description = "Workspace or subscription not found"),
    )
)]
pub(crate) async fn list_webhook_deliveries(
    auth: Authorized<WorkspaceRes, AdminMinAgentEditor, WebhooksRead>,
    Path((_ws, webhook_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    Query(params): Query<ListDeliveriesQuery>,
) -> Result<Json<Page<WebhookDeliveryDto>>, ApiError> {
    let ws_id = auth.workspace.id.0;

    PgWebhookSubscriptionRepo::get_by_id(&*state.db, ws_id, webhook_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    let before_id = params
        .before
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

    let rows = PgWebhookDeliveryRepo::list_for_subscription(
        &*state.db,
        ws_id,
        webhook_id,
        before_id,
        fetch_limit,
    )
    .await
    .map_err(ApiError::Domain)?;

    let has_more = rows.len() as u64 > limit;
    let trimmed: Vec<_> = rows.into_iter().take(limit as usize).collect();
    let next_cursor = if has_more {
        trimmed.last().map(|r| Cursor(r.id))
    } else {
        None
    };

    let items = trimmed.into_iter().map(delivery_row_to_dto).collect();

    Ok(Json(Page::new(items, next_cursor, has_more)))
}
