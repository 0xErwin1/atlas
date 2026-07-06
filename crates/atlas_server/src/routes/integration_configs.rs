use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use rand::RngCore;
use rand::rngs::OsRng;
use sea_orm::TransactionTrait;
use uuid::Uuid;

use atlas_api::dtos::integrations::{
    CreateIntegrationConfigRequest, IntegrationConfigCreatedDto, IntegrationConfigDto,
    UpdateIntegrationConfigRequest,
};
use atlas_domain::permissions::Principal;

use crate::{
    authz::{AdminMin, Authorized, WorkspaceRes},
    error::ApiError,
    persistence::{
        entities::integration_config::integration_configs, repos::PgIntegrationConfigRepo,
    },
    state::AppState,
};

fn row_to_dto(row: integration_configs::Model) -> IntegrationConfigDto {
    IntegrationConfigDto {
        id: row.id,
        workspace_id: row.workspace_id,
        integration: row.integration,
        integration_api_key_id: row.integration_api_key_id,
        is_active: row.is_active,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Generates a fresh HMAC signing secret as `integ_<64-lowercase-hex-chars>`.
fn generate_integration_secret() -> String {
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    format!("integ_{hex}")
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/integration-configs
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/integration-configs",
    tag = "integrations",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateIntegrationConfigRequest,
    responses(
        (status = 201, description = "Config created; secret returned once", body = IntegrationConfigCreatedDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not an admin"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn create_integration_config(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateIntegrationConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ws_id = auth.workspace.id.0;

    let user_id = match &auth.principal {
        Principal::User(uid) => uid.0,
        _ => {
            return Err(ApiError::InvalidInput {
                message: "only user principals can create integration configs".into(),
            });
        }
    };

    let plaintext_secret = generate_integration_secret();
    let (encrypted_secret, secret_nonce) = state
        .webhook_crypto
        .encrypt(plaintext_secret.as_bytes())
        .map_err(|e| ApiError::Internal { message: e })?;

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let row = PgIntegrationConfigRepo::create(
        &txn,
        ws_id,
        body.integration,
        encrypted_secret,
        secret_nonce,
        user_id,
    )
    .await
    .map_err(ApiError::Domain)?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let dto = IntegrationConfigCreatedDto {
        id: row.id,
        workspace_id: row.workspace_id,
        integration: row.integration,
        integration_api_key_id: row.integration_api_key_id,
        is_active: row.is_active,
        secret: plaintext_secret,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok((StatusCode::CREATED, Json(dto)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/integration-configs
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/integration-configs",
    tag = "integrations",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 200, description = "List of integration configs (no secret)", body = Vec<IntegrationConfigDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not an admin"),
    )
)]
pub(crate) async fn list_integration_configs(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<IntegrationConfigDto>>, ApiError> {
    let ws_id = auth.workspace.id.0;

    let rows = PgIntegrationConfigRepo::list(&*state.db, ws_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(rows.into_iter().map(row_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/integration-configs/{config_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/integration-configs/{config_id}",
    tag = "integrations",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("config_id" = Uuid, Path, description = "Integration config id"),
    ),
    responses(
        (status = 200, description = "Integration config (no secret)", body = IntegrationConfigDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Config not found or caller is not an admin"),
    )
)]
pub(crate) async fn get_integration_config(
    auth: Authorized<WorkspaceRes, AdminMin>,
    Path((_ws, config_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
) -> Result<Json<IntegrationConfigDto>, ApiError> {
    let ws_id = auth.workspace.id.0;

    let row = PgIntegrationConfigRepo::get_by_id(&*state.db, ws_id, config_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(row_to_dto(row)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/integration-configs/{config_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/integration-configs/{config_id}",
    tag = "integrations",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("config_id" = Uuid, Path, description = "Integration config id"),
    ),
    request_body = UpdateIntegrationConfigRequest,
    responses(
        (status = 200, description = "Updated integration config (no secret)", body = IntegrationConfigDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Config not found or caller is not an admin"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn patch_integration_config(
    auth: Authorized<WorkspaceRes, AdminMin>,
    Path((_ws, config_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    Json(body): Json<UpdateIntegrationConfigRequest>,
) -> Result<Json<IntegrationConfigDto>, ApiError> {
    let ws_id = auth.workspace.id.0;

    // The only mutable field is `is_active`; with nothing to change, return the
    // current row so the call is an idempotent read.
    let Some(is_active) = body.is_active else {
        let row = PgIntegrationConfigRepo::get_by_id(&*state.db, ws_id, config_id)
            .await
            .map_err(ApiError::Domain)?
            .ok_or(ApiError::NotFound)?;
        return Ok(Json(row_to_dto(row)));
    };

    let row = PgIntegrationConfigRepo::set_active(&*state.db, ws_id, config_id, is_active)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(row_to_dto(row)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/integration-configs/{config_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/integration-configs/{config_id}",
    tag = "integrations",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("config_id" = Uuid, Path, description = "Integration config id"),
    ),
    responses(
        (status = 204, description = "Config soft-deleted and api key revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Config not found or caller is not an admin"),
    )
)]
pub(crate) async fn delete_integration_config(
    auth: Authorized<WorkspaceRes, AdminMin>,
    Path((_ws, config_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ws_id = auth.workspace.id.0;

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    PgIntegrationConfigRepo::soft_delete_and_revoke_key(&txn, ws_id, config_id)
        .await
        .map_err(ApiError::Domain)?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(StatusCode::NO_CONTENT)
}
