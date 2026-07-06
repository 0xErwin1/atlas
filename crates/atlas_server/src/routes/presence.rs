//! Presence endpoints: heartbeats that mark the caller present on a board or a
//! document, and leaves that remove them.
//!
//! Each is gated by an `Authorized<_, ViewerMin>` extractor, so a principal can
//! only signal presence on a resource they may view. A change to the visible set
//! publishes a `presence.updated` live event, which the SSE layer forwards only to
//! other viewers of that same resource — board presence gated against the board
//! chain, document presence against the per-document chain.

use axum::{Json, extract::State, http::StatusCode};

use atlas_api::dtos::{boards_tasks::BoardPresenceResponse, documents::DocumentPresenceResponse};
use atlas_domain::{WorkspaceCtx, permissions::Principal};

use crate::{
    authz::{Authorized, BoardRes, BoardsRead, DocsRead, ViewerMin, authorized::DocumentSlugRes},
    error::ApiError,
    presence::{PresenceResource, PrincipalKey, broadcast_presence},
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
    path = "/api/workspaces/{ws}/boards/{board_id}/presence",
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
    auth: Authorized<BoardRes, ViewerMin, BoardsRead>,
    State(state): State<AppState>,
) -> Result<Json<BoardPresenceResponse>, ApiError> {
    let board = &auth.resource.0;
    let workspace_id = auth.workspace.id.0;
    let board_id = board.id.0;
    let project_id = board.project_id.0;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let actor_dto = resolve_actor_dto(&state, &ctx, &actor).await;

    let resource = PresenceResource::Board(board_id);
    let changed = state.presence.heartbeat(workspace_id, resource, actor_dto);
    if changed {
        broadcast_presence(&state, workspace_id, resource, Some(project_id));
    }

    let actors = state.presence.snapshot(workspace_id, resource);
    Ok(Json(BoardPresenceResponse { actors }))
}

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/boards/{board_id}/presence",
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
    auth: Authorized<BoardRes, ViewerMin, BoardsRead>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let board = &auth.resource.0;
    let workspace_id = auth.workspace.id.0;
    let board_id = board.id.0;
    let project_id = board.project_id.0;

    let resource = PresenceResource::Board(board_id);
    let key = principal_key(&auth.principal);
    let changed = state.presence.leave(workspace_id, resource, &key);
    if changed {
        broadcast_presence(&state, workspace_id, resource, Some(project_id));
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/documents/{slug}/presence",
    tag = "presence",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug or UUID"),
    ),
    responses(
        (status = 200, description = "Current document presence", body = DocumentPresenceResponse),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn document_heartbeat(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(state): State<AppState>,
) -> Result<Json<DocumentPresenceResponse>, ApiError> {
    let workspace_id = auth.workspace.id.0;
    // The slug segment accepts a UUID or a human slug; the resolved document's
    // canonical UUID keys presence and is returned so a slug-addressed client can
    // correlate the UUID-keyed `presence.updated` broadcasts that follow.
    let document_id = auth.resource.0.id.0;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let actor_dto = resolve_actor_dto(&state, &ctx, &actor).await;

    let resource = PresenceResource::Document(document_id);
    let changed = state.presence.heartbeat(workspace_id, resource, actor_dto);
    if changed {
        broadcast_presence(&state, workspace_id, resource, None);
    }

    let actors = state.presence.snapshot(workspace_id, resource);
    Ok(Json(DocumentPresenceResponse {
        document_id,
        actors,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/documents/{slug}/presence",
    tag = "presence",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug or UUID"),
    ),
    responses(
        (status = 204, description = "Presence cleared"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn document_leave(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let workspace_id = auth.workspace.id.0;
    let document_id = auth.resource.0.id.0;

    let resource = PresenceResource::Document(document_id);
    let key = principal_key(&auth.principal);
    let changed = state.presence.leave(workspace_id, resource, &key);
    if changed {
        broadcast_presence(&state, workspace_id, resource, None);
    }

    Ok(StatusCode::NO_CONTENT)
}
