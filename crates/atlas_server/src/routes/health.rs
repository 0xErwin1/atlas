use axum::{Json, response::IntoResponse};
use serde_json::json;

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
