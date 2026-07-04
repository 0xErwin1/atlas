//! `GET /v1/workspaces/{ws}/events` — the live-updates Server-Sent Events stream.
//!
//! The connect gate is `WorkspaceAccess`: any principal with access to the
//! workspace (a member of any role, or a holder of any grant anywhere in the
//! workspace) may open a stream. Per-event authorization is then applied to every
//! forwarded event, so a principal whose only access is a narrow board/project
//! grant still connects, but sees only the events they may view.
//!
//! For each `LiveEvent` published to the in-process hub the stream:
//! 1. drops cross-tenant events (`workspace_id` mismatch) without a DB hit;
//! 2. resolves the principal's effective role on the event's most-specific
//!    resource (board, else project, else workspace) via the same domain logic the
//!    `Authorized` extractor uses, forwarding only when the role is at least Viewer;
//! 3. forwards the raw envelope JSON verbatim as the SSE `data`, named by the
//!    domain `event_type`.
//!
//! Per-resource decisions are cached per connection with a short TTL so repeated
//! events on the same board do not re-query the database, while a revoked grant
//! still takes effect within the connection's lifetime.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::Stream;
use sea_orm::DatabaseConnection;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::Instant;
use uuid::Uuid;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{MemberRole, Workspace},
    ids::{BoardId, ProjectId, UserId},
    permissions::{ChainSegment, Principal, ResourceChain, ResourceRef, ResourceRole},
};

use crate::{
    authz::{
        authorized::{build_board_chain, resolve_effective_role},
        extractors::WorkspaceAccess,
    },
    error::ApiError,
    live::LiveEvent,
    persistence::repos::{PgProjectRepo, ProjectRepo},
    state::AppState,
};

/// How long a per-resource authorization decision is trusted before it is
/// re-resolved. Short enough that a revoked grant takes effect within the
/// connection's lifetime; long enough that a burst of events on one board does
/// not re-query the database for each event.
const DECISION_TTL: Duration = Duration::from_secs(30);

/// Idle keep-alive interval, so proxies do not drop an otherwise silent stream.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Opens a live-updates SSE stream for the authenticated principal.
///
/// Authorization to open the stream is enforced by the `WorkspaceAccess`
/// extractor before any response body is produced, so an unauthenticated or
/// cross-tenant request is rejected without ever opening a stream.
pub(crate) async fn stream_events(
    access: WorkspaceAccess,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let connection = ConnectionState {
        receiver: state.live.subscribe(),
        db: state.db.clone(),
        principal: access.principal,
        membership: access.membership,
        workspace: access.workspace,
        decisions: HashMap::new(),
    };

    Sse::new(event_stream(connection)).keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL))
}

/// Per-connection state threaded through the event stream.
struct ConnectionState {
    receiver: broadcast::Receiver<LiveEvent>,
    db: Arc<DatabaseConnection>,
    principal: Principal,
    membership: Option<MemberRole>,
    workspace: Workspace,
    /// Cache of per-resource authorization decisions, keyed by the resolved
    /// resource id (board or project) with the instant the decision was made.
    decisions: HashMap<Uuid, (bool, Instant)>,
}

/// Builds the SSE event stream from the connection's broadcast receiver.
///
/// Filtering is expressed by *not* yielding an item; only forwarded events and
/// the lag `resync` signal produce SSE frames, so the stream item is infallible.
/// A `Lagged` receiver emits a single `resync` event (the client should reload),
/// then continues; a `Closed` channel ends the stream.
fn event_stream(state: ConnectionState) -> impl Stream<Item = Result<Event, Infallible>> {
    futures::stream::unfold(state, |mut state| async move {
        loop {
            match state.receiver.recv().await {
                Ok(event) => {
                    if event.workspace_id != state.workspace.id.0 {
                        continue;
                    }

                    if authorize_event(&mut state, &event).await {
                        let frame = Event::default()
                            .event(event.event_type.as_str())
                            .data(event.payload.as_ref());
                        return Some((Ok(frame), state));
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    let frame = Event::default().event("resync").data("");
                    return Some((Ok(frame), state));
                }
                Err(RecvError::Closed) => return None,
            }
        }
    })
}

/// Decides whether `event` may be forwarded to this connection's principal,
/// consulting and refreshing the per-resource decision cache.
///
/// A workspace-level event (no board or project scope) is always forwarded: the
/// connect gate already proved the principal has workspace access.
async fn authorize_event(state: &mut ConnectionState, event: &LiveEvent) -> bool {
    let Some(resource_id) = event.board_id.or(event.project_id) else {
        return true;
    };

    if let Some((allowed, decided_at)) = state.decisions.get(&resource_id)
        && decided_at.elapsed() < DECISION_TTL
    {
        return *allowed;
    }

    let allowed = resolve_event_access(state, event).await;
    state
        .decisions
        .insert(resource_id, (allowed, Instant::now()));
    allowed
}

/// Resolves the principal's effective role on the event's most-specific resource
/// and returns whether it is at least Viewer. Any resolution failure — a deleted
/// or unresolvable resource, or a database error — is treated as "not viewable"
/// (fail closed).
async fn resolve_event_access(state: &ConnectionState, event: &LiveEvent) -> bool {
    let chain = match build_event_chain(&state.db, &state.workspace, event).await {
        Ok(Some(chain)) => chain,
        Ok(None) | Err(_) => return false,
    };

    let effective = resolve_effective_role(
        &state.db,
        &state.principal,
        state.membership.clone(),
        &state.workspace,
        &chain,
    )
    .await;

    matches!(effective, Ok(Some(role)) if role >= ResourceRole::Viewer)
}

/// Builds the permission chain for the event's most-specific resource, reusing
/// the same chain builders the request extractors use.
///
/// Board-scoped events resolve against the `board → project → workspace` chain.
/// Because the chain is built from the event's routing ids (not a fresh existence
/// check), a `*.deleted` event whose resource row is already soft-deleted still
/// resolves against the surviving parent scope — so a principal learns of a
/// deletion they could previously see, without leaking one they could not.
/// Returns `Ok(None)` when the scope cannot be resolved (the event is skipped).
async fn build_event_chain(
    db: &DatabaseConnection,
    workspace: &Workspace,
    event: &LiveEvent,
) -> Result<Option<ResourceChain>, ApiError> {
    if let Some(board_uuid) = event.board_id {
        let board_id = BoardId(board_uuid);

        let project_id = match event.project_id {
            Some(project_uuid) => ProjectId(project_uuid),
            None => match load_board_project(db, workspace, board_id).await? {
                Some(project_id) => project_id,
                None => return Ok(None),
            },
        };

        let chain = build_board_chain(db, workspace, board_id, project_id).await?;
        return Ok(Some(chain));
    }

    if let Some(project_uuid) = event.project_id {
        return build_project_chain(db, workspace, ProjectId(project_uuid)).await;
    }

    Ok(None)
}

/// Looks up the surviving (non-deleted) board's project, used for board-scoped
/// events whose routing omits the project id (e.g. `column.deleted`).
async fn load_board_project(
    db: &DatabaseConnection,
    workspace: &Workspace,
    board_id: BoardId,
) -> Result<Option<ProjectId>, ApiError> {
    use crate::persistence::entities::boards_tasks::board;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let row = board::Entity::find_by_id(board_id.0)
        .filter(board::Column::WorkspaceId.eq(workspace.id.0))
        .filter(board::Column::DeletedAt.is_null())
        .one(db)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(row.map(|r| ProjectId(r.project_id)))
}

/// Builds the `project → workspace` chain for a project-scoped event, carrying
/// the project's visibility so member visibility contributes to resolution.
/// Returns `Ok(None)` when the project is absent (deleted or cross-tenant).
async fn build_project_chain(
    db: &DatabaseConnection,
    workspace: &Workspace,
    project_id: ProjectId,
) -> Result<Option<ResourceChain>, ApiError> {
    let repo = PgProjectRepo { conn: db.clone() };
    let ctx = WorkspaceCtx::new(workspace.id, Actor::User(UserId::new()));

    let Some(project) = repo
        .find(&ctx, project_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
    else {
        return Ok(None);
    };

    let chain = ResourceChain {
        segments: vec![
            ChainSegment {
                resource: ResourceRef::Project(project_id),
                visibility: Some(project.visibility.clone()),
            },
            ChainSegment {
                resource: ResourceRef::Workspace,
                visibility: None,
            },
        ],
    };

    Ok(Some(chain))
}
