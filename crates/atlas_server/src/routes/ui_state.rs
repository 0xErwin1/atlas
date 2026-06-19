use axum::{
    Json,
    extract::{Extension, State},
};

use atlas_api::dtos::{UiStateDto, UpdateUiStateRequest};

use crate::{
    auth::middleware::Principal,
    error::ApiError,
    persistence::repos::{PgUiStateRepo, UiStateRepo},
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/v1/me/ui-state",
    tag = "ui-state",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The caller's stored UI state, or an empty object", body = UiStateDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys have no UI state"),
    )
)]
/// Returns the authenticated human user's stored UI state.
///
/// The state is an opaque JSON object owned by the client. When the user has no
/// row yet, an empty object `{}` is returned. API keys (agents) are rejected
/// with 403: agents have no UI.
pub(crate) async fn get_ui_state(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<UiStateDto>, ApiError> {
    let Principal::User(user_id) = principal else {
        return Err(ApiError::Forbidden {
            message: "API keys have no UI state".into(),
        });
    };

    let repo = PgUiStateRepo {
        conn: (*state.db).clone(),
    };

    let stored = repo.find(user_id).await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let state = stored
        .map(|row| row.state)
        .unwrap_or_else(|| serde_json::json!({}));

    Ok(Json(UiStateDto { state }))
}

#[utoipa::path(
    put,
    path = "/v1/me/ui-state",
    tag = "ui-state",
    security(("bearer_auth" = [])),
    request_body = UpdateUiStateRequest,
    responses(
        (status = 200, description = "The stored UI state, echoed back", body = UiStateDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys have no UI state"),
    )
)]
/// Inserts or replaces the authenticated human user's UI state.
///
/// The `state` JSON object is stored verbatim; the server does not validate its
/// inner shape. The stored value is echoed back. API keys (agents) are rejected
/// with 403: agents have no UI.
pub(crate) async fn set_ui_state(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<UpdateUiStateRequest>,
) -> Result<Json<UiStateDto>, ApiError> {
    let Principal::User(user_id) = principal else {
        return Err(ApiError::Forbidden {
            message: "API keys have no UI state".into(),
        });
    };

    let repo = PgUiStateRepo {
        conn: (*state.db).clone(),
    };

    let stored = repo
        .upsert(user_id, body.state)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(UiStateDto {
        state: stored.state,
    }))
}
