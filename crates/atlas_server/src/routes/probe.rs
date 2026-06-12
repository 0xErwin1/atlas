use axum::{Json, response::IntoResponse};
use serde_json::json;

use crate::{authz::WorkspaceMember, error::ApiError};

/// Temporary probe endpoint for extractor e2e tests.
///
/// Returns 200 when the caller is a valid workspace member. Removed after T-13
/// when real workspace routes replace it in the route matrix.
pub(crate) async fn probe(_member: WorkspaceMember) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(json!({"status": "ok"})))
}
