use axum::{Json, response::IntoResponse};
use serde_json::json;

use atlas_api::dtos::ServerMetaDto;

#[utoipa::path(
    get,
    path = "/health",
    responses((status = 200, description = "Service is healthy"))
)]
pub(crate) async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

#[utoipa::path(
    get,
    path = "/version",
    responses((status = 200, description = "Service version"))
)]
pub(crate) async fn version() -> impl IntoResponse {
    Json(json!({"version": env!("CARGO_PKG_VERSION")}))
}

#[utoipa::path(
    get,
    path = "/v1/meta",
    tag = "meta",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Server build information", body = ServerMetaDto),
        (status = 401, description = "Unauthenticated"),
    )
)]
pub(crate) async fn meta() -> impl IntoResponse {
    Json(ServerMetaDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build: std::env::var("ATLAS_BUILD").ok(),
    })
}
