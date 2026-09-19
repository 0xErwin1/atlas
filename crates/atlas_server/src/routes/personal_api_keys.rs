//! The personal API-key route family (`v2-e4-s3b`): every handler is a thin
//! utoipa-annotated wrapper passing `ApiKeyKind::Personal` into the shared
//! handler set in [`super::api_keys`]. A personal key binds to the caller's
//! own user principal and needs no agent; the route family itself is the
//! credential-kind decision, so the request body carries no `key_kind`.
//!
//! A key of the other family addressed through these paths is invisible —
//! the shared core resolves the target inside the caller's own visible set
//! and answers 404, never a 403 that would confirm the key exists.

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
};

use atlas_api::dtos::{
    ApiKeyCreated, ApiKeyDto, ApiKeyGrantDto, CreatePersonalApiKeyRequest, UpdateApiKeyRequest,
};
use atlas_api::pagination::Page;
use atlas_custos::entities::identity::ApiKeyKind;

use crate::auth::middleware::Principal as AuthPrincipal;
use crate::error::ApiError;
use crate::routes::api_keys::{
    ApiKeyGrantPath, PaginationQuery, TopLevelRevokeKeyPath, create_personal_key,
    delete_key_grant_core, list_key_grants_core, list_keys_core, revoke_key_core, update_key_core,
};
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/personal-api-keys",
    tag = "personal-api-keys",
    security(("bearer_auth" = [])),
    request_body = CreatePersonalApiKeyRequest,
    responses(
        (status = 201, description = "Personal API key created (secret shown once)", body = ApiKeyCreated),
        (status = 400, description = "Invalid key type or role"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot create keys"),
        (status = 422, description = "Unknown scope value"),
    )
)]
pub(crate) async fn create_personal_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreatePersonalApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    create_personal_key(state, principal, body).await
}

#[utoipa::path(
    get,
    path = "/personal-api-keys",
    tag = "personal-api-keys",
    security(("bearer_auth" = [])),
    params(
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated list of the caller's personal API keys", body = Page<ApiKeyDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot list keys"),
    )
)]
pub(crate) async fn list_personal_api_keys(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ApiKeyDto>>, ApiError> {
    list_keys_core(state, principal, ApiKeyKind::Personal, q).await
}

#[utoipa::path(
    delete,
    path = "/personal-api-keys/{key_id}",
    tag = "personal-api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    responses(
        (status = 204, description = "API key revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Key not found, not owned by the caller, or not a personal key"),
    )
)]
pub(crate) async fn revoke_personal_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
) -> Result<axum::http::StatusCode, ApiError> {
    revoke_key_core(state, principal, ApiKeyKind::Personal, params).await
}

#[utoipa::path(
    patch,
    path = "/personal-api-keys/{key_id}",
    tag = "personal-api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    request_body = UpdateApiKeyRequest,
    responses(
        (status = 200, description = "API key updated", body = ApiKeyDto),
        (status = 400, description = "Scopes present but empty; revoke the key instead"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot manage API keys"),
        (status = 404, description = "Key not found, not owned by the caller, or not a personal key"),
        (status = 422, description = "Unknown scope value"),
    )
)]
pub(crate) async fn update_personal_api_key(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
    Json(body): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiKeyDto>, ApiError> {
    update_key_core(state, principal, ApiKeyKind::Personal, params, body).await
}

#[utoipa::path(
    get,
    path = "/personal-api-keys/{key_id}/grants",
    tag = "personal-api-keys",
    security(("bearer_auth" = [])),
    params(("key_id" = uuid::Uuid, Path, description = "API key id")),
    responses(
        (status = 200, description = "Grants belonging to this API key", body = Vec<ApiKeyGrantDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Key not found, not owned by the caller, or not a personal key"),
    )
)]
pub(crate) async fn list_personal_api_key_grants(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<TopLevelRevokeKeyPath>,
) -> Result<Json<Vec<ApiKeyGrantDto>>, ApiError> {
    list_key_grants_core(state, principal, ApiKeyKind::Personal, params).await
}

#[utoipa::path(
    delete,
    path = "/personal-api-keys/{key_id}/grants/{grant_id}",
    tag = "personal-api-keys",
    security(("bearer_auth" = [])),
    params(
        ("key_id" = uuid::Uuid, Path, description = "API key id"),
        ("grant_id" = uuid::Uuid, Path, description = "Grant id to revoke"),
    ),
    responses(
        (status = 204, description = "Grant revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Key or grant not found, key not owned by the caller, or key not a personal key"),
    )
)]
pub(crate) async fn delete_personal_api_key_grant(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<ApiKeyGrantPath>,
) -> Result<axum::http::StatusCode, ApiError> {
    delete_key_grant_core(state, principal, ApiKeyKind::Personal, params).await
}
