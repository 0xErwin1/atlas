//! The agent API-key route family (`v2-e4-s3b`): every handler is a thin
//! utoipa-annotated wrapper passing `ApiKeyKind::Agent` into the shared
//! handler set in [`super::api_keys`]. Creating an agent key requires a
//! mandatory `agent_id`; the server resolves that agent through the same
//! visible-set rules `POST /agents` already enforces (owner match, or
//! platform admin), so a foreign or nonexistent agent answers 404 — never a
//! 403 that would confirm it exists.
//!
//! A key of the other family addressed through these paths is invisible for
//! the same structural reason.

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
};

use atlas_api::dtos::{
    ApiKeyCreated, ApiKeyDto, ApiKeyGrantDto, CreateAgentApiKeyRequest, UpdateApiKeyRequest,
};
use atlas_api::pagination::Page;
use atlas_custos::entities::identity::ApiKeyKind;

use crate::auth::middleware::Principal as AuthPrincipal;
use crate::error::ApiError;
use crate::routes::api_keys::{
    ApiKeyGrantPath, PaginationQuery, TopLevelRevokeKeyPath, create_agent_key,
    delete_key_grant_core, list_key_grants_core, list_keys_core, revoke_key_core, update_key_core,
};
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/agent-api-keys",
    tag = "agent-api-keys",
    security(("bearer_auth" = [])),
    request_body = CreateAgentApiKeyRequest,
    responses(
        (status = 201, description = "Agent API key created (secret shown once)", body = ApiKeyCreated),
        (status = 400, description = "Invalid key type or role"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot create keys"),
        (status = 404, description = "Agent does not exist or is not visible to the caller"),
        (status = 422, description = "Unknown scope value"),
    )
)]
pub(crate) async fn create_agent_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreateAgentApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    create_agent_key(state, principal, body).await
}

#[utoipa::path(
    get,
    path = "/agent-api-keys",
    tag = "agent-api-keys",
    security(("bearer_auth" = [])),
    params(
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated list of the caller's agent API keys", body = Page<ApiKeyDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot list keys"),
    )
)]
pub(crate) async fn list_agent_api_keys(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ApiKeyDto>>, ApiError> {
    list_keys_core(state, principal, ApiKeyKind::Agent, q).await
}

#[utoipa::path(
    delete,
    path = "/agent-api-keys/{key_id}",
    tag = "agent-api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    responses(
        (status = 204, description = "API key revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Key not found, not owned by the caller, or not an agent key"),
    )
)]
pub(crate) async fn revoke_agent_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_key_core(state, principal, ApiKeyKind::Agent, params).await
}

#[utoipa::path(
    patch,
    path = "/agent-api-keys/{key_id}",
    tag = "agent-api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    request_body = UpdateApiKeyRequest,
    responses(
        (status = 200, description = "API key updated", body = ApiKeyDto),
        (status = 400, description = "Scopes present but empty; revoke the key instead"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot manage API keys"),
        (status = 404, description = "Key not found, not owned by the caller, or not an agent key"),
        (status = 422, description = "Unknown scope value"),
    )
)]
pub(crate) async fn update_agent_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
    Json(body): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiKeyDto>, ApiError> {
    update_key_core(state, principal, ApiKeyKind::Agent, params, body).await
}

#[utoipa::path(
    get,
    path = "/agent-api-keys/{key_id}/grants",
    tag = "agent-api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    responses(
        (status = 200, description = "Grants belonging to this API key", body = Vec<ApiKeyGrantDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Key not found, not owned by the caller, or not an agent key"),
    )
)]
pub(crate) async fn list_agent_api_key_grants(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
) -> Result<Json<Vec<ApiKeyGrantDto>>, ApiError> {
    list_key_grants_core(state, principal, ApiKeyKind::Agent, params).await
}

#[utoipa::path(
    delete,
    path = "/agent-api-keys/{key_id}/grants/{grant_id}",
    tag = "agent-api-keys",
    security(("bearer_auth" = [])),
    params(
        ("key_id" = uuid::Uuid, Path, description = "API key id"),
        ("grant_id" = uuid::Uuid, Path, description = "Grant id to revoke"),
    ),
    responses(
        (status = 204, description = "Grant revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Key or grant not found, key not owned by the caller, or key not an agent key"),
    )
)]
pub(crate) async fn delete_agent_api_key_grant(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<ApiKeyGrantPath>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_key_grant_core(state, principal, ApiKeyKind::Agent, params).await
}
