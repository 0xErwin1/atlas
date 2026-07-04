//! Board presence endpoints: a heartbeat that marks the caller present on a board
//! and a leave that removes them.
//!
//! Both are gated by `Authorized<BoardRes, ViewerMin>`, so a principal can only
//! signal presence on a board they may view. A change to the visible set publishes
//! a `presence.updated` live event, which the SSE layer forwards only to other
//! viewers of that board.

use axum::{Json, extract::State, http::StatusCode};

use atlas_api::dtos::boards_tasks::BoardPresenceResponse;
use atlas_domain::{WorkspaceCtx, permissions::Principal};

use crate::{
    authz::{Authorized, BoardRes, ViewerMin},
    error::ApiError,
    presence::{PrincipalKey, broadcast_board_presence},
    routes::tasks::{principal_to_actor, resolve_actor_dto},
    state::AppState,
};

/// Derives the presence key for the request's principal, mirroring the actor
/// mapping used to resolve the stored `ActorDto` so heartbeat and leave key the
/// same principal identically.
fn principal_key(principal: &Principal) -> PrincipalKey {
    match principal {
        Principal::User(uid) => PrincipalKey::User(uid.0),
        Principal::ApiKey(kid) => PrincipalKey::ApiKey(kid.0),
        // Groups are never request principals; mirror `principal_to_actor`'s fallback.
        Principal::Group(_) => PrincipalKey::User(uuid::Uuid::nil()),
    }
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/boards/{board_id}/presence",
    tag = "presence",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    responses(
        (status = 200, description = "Current board presence", body = BoardPresenceResponse),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn heartbeat(
    auth: Authorized<BoardRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<BoardPresenceResponse>, ApiError> {
    let board = &auth.resource.0;
    let workspace_id = auth.workspace.id.0;
    let board_id = board.id.0;
    let project_id = board.project_id.0;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let actor_dto = resolve_actor_dto(&state, &ctx, &actor).await;

    let changed = state.presence.heartbeat(workspace_id, board_id, actor_dto);
    if changed {
        broadcast_board_presence(&state, workspace_id, board_id, Some(project_id));
    }

    let actors = state.presence.snapshot(workspace_id, board_id);
    Ok(Json(BoardPresenceResponse { actors }))
}

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/boards/{board_id}/presence",
    tag = "presence",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    responses(
        (status = 204, description = "Presence cleared"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn leave(
    auth: Authorized<BoardRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let board = &auth.resource.0;
    let workspace_id = auth.workspace.id.0;
    let board_id = board.id.0;
    let project_id = board.project_id.0;

    let key = principal_key(&auth.principal);
    let changed = state.presence.leave(workspace_id, board_id, &key);
    if changed {
        broadcast_board_presence(&state, workspace_id, board_id, Some(project_id));
    }

    Ok(StatusCode::NO_CONTENT)
}
