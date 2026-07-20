//! `GET /api/workspaces/{ws}/events` — the live-updates Server-Sent Events stream.
//!
//! The connect gate is `WorkspaceAccess`: any principal with access to the
//! workspace (a member of any role, or a holder of any grant anywhere in the
//! workspace) may open a stream. Per-event authorization is then applied to every
//! forwarded event, so a principal whose only access is a narrow board/project
//! grant still connects, but sees only the events they may view.
//!
//! For each `LiveEvent` published to the in-process hub the stream:
//! 1. drops cross-tenant events (`workspace_id` mismatch) without a DB hit;
//! 2. for an API-key (agent) connection, drops events for families the key does
//!    not hold `{family}:read` on — a capability pre-filter applied before the
//!    role check; human, root, and group principals carry no scope axis and skip it;
//! 3. resolves the principal's effective role on the event's most-specific
//!    resource (board, else project, else workspace) via the same domain logic the
//!    `Authorized` extractor uses, forwarding only when the role is at least Viewer;
//! 4. forwards the raw envelope JSON verbatim as the SSE `data`, named by the
//!    domain `event_type`.
//!
//! The API-key read-capability set is loaded once, when the connect gate
//! (`WorkspaceAccess`) admits the stream, and reused for every event: capability
//! scopes are static for the life of a key, so — unlike the per-resource ROLE
//! decision — they are never re-loaded per event nor cached in the TTL map below.
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
    permissions::{
        CapabilityFamily, ChainSegment, Principal, ResourceChain, ResourceRef, ResourceRole,
    },
};

use crate::{
    authz::{
        authorized::{
            ReadScopeSet, build_board_chain, build_document_chain, resolve_effective_role,
        },
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
        read_scopes: access.read_scopes,
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
    /// The API key's read-capability set, loaded once at connect time and present
    /// only for a `Principal::ApiKey` connection. `None` for users, root, and
    /// groups, which have no scope axis and read every family. Static for the
    /// connection's lifetime, so it is deliberately not part of the TTL-refreshed
    /// `decisions` cache below.
    read_scopes: Option<ReadScopeSet>,
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
    // Capability pre-filter: an agent only receives events for families it holds
    // `{family}:read` on. Applied before the role check and only for a scoped
    // ApiKey connection; humans, root, and groups carry no scope axis (`None`).
    if let Some(read_scopes) = &state.read_scopes
        && !event_allowed_by_read_scopes(read_scopes, event)
    {
        return false;
    }

    // Most-specific resource first: a document-scoped event (presence on a document)
    // is gated by its own per-document chain, never by the coarser board/project.
    let Some(resource_id) = event.document_id.or(event.board_id).or(event.project_id) else {
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

/// The capability family that gates an event's `event_type` for API-key
/// read-scope filtering.
///
/// The `event_type` string is `family.action` (e.g. `task.created`); the family
/// is derived from its prefix. `presence.updated` carries no family in its type —
/// it is resolved from the event's routing ids by the caller. Any prefix outside
/// the known set is `Unknown` and dropped for a scoped agent (fail closed), so a
/// newly introduced event type never leaks to an under-scoped key by default.
enum EventTypeFamily {
    /// Governed by exactly one capability family; require `{family}:read`.
    Family(CapabilityFamily),
    /// `presence.updated`: the family lives in the routing ids, not the type.
    Presence,
    /// No known family and not presence: dropped for a scoped agent (fail closed).
    Unknown,
}

/// Classifies an `event_type` string into the capability family that gates it.
///
/// Boards and columns fold into the `Boards` family, mirroring the route registry
/// gating column and status-template routes as `boards:*`.
fn classify_event_type(event_type: &str) -> EventTypeFamily {
    let prefix = event_type.split('.').next().unwrap_or(event_type);

    match prefix {
        "task" => EventTypeFamily::Family(CapabilityFamily::Tasks),
        "document" => EventTypeFamily::Family(CapabilityFamily::Docs),
        "board" | "column" => EventTypeFamily::Family(CapabilityFamily::Boards),
        "folder" => EventTypeFamily::Family(CapabilityFamily::Folders),
        "project" => EventTypeFamily::Family(CapabilityFamily::Projects),
        "presence" => EventTypeFamily::Presence,
        _ => EventTypeFamily::Unknown,
    }
}

/// The capability family a `presence.updated` event is gated by, resolved from its
/// routing ids most-specific-first — mirroring `authorize_event`'s
/// document → board precedence. `None` for a workspace-level presence event
/// carrying neither id, which the connect gate already authorized.
fn presence_read_family(event: &LiveEvent) -> Option<CapabilityFamily> {
    if event.document_id.is_some() {
        Some(CapabilityFamily::Docs)
    } else if event.board_id.is_some() {
        Some(CapabilityFamily::Boards)
    } else {
        None
    }
}

/// Whether a scoped API-key connection may receive `event` given its read
/// capabilities. Called only for `Principal::ApiKey` connections carrying a
/// `ReadScopeSet`; users, root, and groups bypass this entirely.
///
/// Fails closed: an event whose type maps to no known family (and is not a
/// resolvable workspace-level or presence case) is not forwarded.
fn event_allowed_by_read_scopes(read_scopes: &ReadScopeSet, event: &LiveEvent) -> bool {
    match classify_event_type(&event.event_type) {
        EventTypeFamily::Family(family) => read_scopes.allows(family),
        EventTypeFamily::Presence => match presence_read_family(event) {
            Some(family) => read_scopes.allows(family),
            None => true,
        },
        EventTypeFamily::Unknown => false,
    }
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
    if let Some(document_uuid) = event.document_id {
        return build_document_event_chain(db, workspace, document_uuid).await;
    }

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

/// Builds the full `document → folder ancestry → project → workspace` chain for a
/// document-scoped event, reusing the same builder the document request extractors
/// use so presence visibility exactly equals document view access.
///
/// The document is re-loaded from its id (the event carries no folder/project
/// routing) so its folder ancestry and effective project are resolved fresh.
/// Returns `Ok(None)` when the document is absent (deleted or cross-tenant), which
/// fails the event closed.
async fn build_document_event_chain(
    db: &DatabaseConnection,
    workspace: &Workspace,
    document_uuid: Uuid,
) -> Result<Option<ResourceChain>, ApiError> {
    use crate::persistence::entities::documents::{document, document_from};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let row = document::Entity::find_by_id(document_uuid)
        .filter(document::Column::WorkspaceId.eq(workspace.id.0))
        .filter(document::Column::DeletedAt.is_null())
        .one(db)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let Some(row) = row else {
        return Ok(None);
    };

    let doc = document_from(row).map_err(|message| ApiError::Internal { message })?;
    let chain = build_document_chain(db, workspace, &doc).await?;

    Ok(Some(chain))
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use atlas_domain::entities::events::{
        BoardCreatedPayload, BoardDeletedPayload, BoardMovedPayload, BoardUpdatedPayload,
        ColumnCreatedPayload, ColumnDeletedPayload, DocumentCreatedPayload, DocumentDeletedPayload,
        DocumentMovedPayload, DocumentUpdatedPayload, DomainEvent, FolderCreatedPayload,
        FolderDeletedPayload, TaskCreatedPayload, TaskDeletedPayload, TaskMovedPayload,
        TaskUpdatedPayload,
    };
    use atlas_domain::ids::{
        BoardId, ColumnId, DocumentId, FolderId, ProjectId, RevisionId, TaskId,
    };
    use atlas_domain::permissions::{Capability, CapabilityAction};

    fn nid<T: From<Uuid>>() -> T {
        Uuid::now_v7().into()
    }

    /// Every domain event this system emits, one instance per variant. The
    /// exhaustiveness guard is [`expected_family`]'s catch-all-free match: adding a
    /// `DomainEvent` variant fails to compile until it is consciously classified,
    /// and this list must then be extended for the round-trip assertion to cover it.
    fn all_domain_events() -> Vec<DomainEvent> {
        vec![
            DomainEvent::TaskCreated(TaskCreatedPayload {
                task_id: TaskId(nid()),
                title: "t".into(),
                project_id: ProjectId(nid()),
                board_id: BoardId(nid()),
                column_id: ColumnId(nid()),
            }),
            DomainEvent::TaskUpdated(TaskUpdatedPayload {
                task_id: TaskId(nid()),
                changed_fields: vec!["title".into()],
            }),
            DomainEvent::TaskMoved(TaskMovedPayload {
                task_id: TaskId(nid()),
                from_column_id: ColumnId(nid()),
                to_column_id: ColumnId(nid()),
            }),
            DomainEvent::TaskDeleted(TaskDeletedPayload {
                task_id: TaskId(nid()),
            }),
            DomainEvent::DocumentCreated(DocumentCreatedPayload {
                document_id: DocumentId(nid()),
                slug: "d".into(),
                title: "D".into(),
                project_id: None,
                folder_id: None,
            }),
            DomainEvent::DocumentUpdated(DocumentUpdatedPayload {
                document_id: DocumentId(nid()),
                revision_id: RevisionId(nid()),
                seq: 1,
            }),
            DomainEvent::DocumentDeleted(DocumentDeletedPayload {
                document_id: DocumentId(nid()),
            }),
            DomainEvent::DocumentMoved(DocumentMovedPayload {
                document_id: DocumentId(nid()),
                from_folder_id: None,
                to_folder_id: None,
                project_id: None,
            }),
            DomainEvent::BoardCreated(BoardCreatedPayload {
                board_id: BoardId(nid()),
                project_id: ProjectId(nid()),
                name: "b".into(),
            }),
            DomainEvent::BoardUpdated(BoardUpdatedPayload {
                board_id: BoardId(nid()),
                changed_fields: vec!["name".into()],
            }),
            DomainEvent::BoardDeleted(BoardDeletedPayload {
                board_id: BoardId(nid()),
                project_id: ProjectId(nid()),
            }),
            DomainEvent::BoardMoved(BoardMovedPayload {
                board_id: BoardId(nid()),
                from_folder_id: None,
                to_folder_id: None,
                project_id: ProjectId(nid()),
            }),
            DomainEvent::ColumnCreated(ColumnCreatedPayload {
                board_id: BoardId(nid()),
                column_id: ColumnId(nid()),
                name: "c".into(),
            }),
            DomainEvent::ColumnDeleted(ColumnDeletedPayload {
                board_id: BoardId(nid()),
                column_id: ColumnId(nid()),
            }),
            DomainEvent::FolderCreated(FolderCreatedPayload {
                folder_id: FolderId(nid()),
                project_id: None,
                name: "f".into(),
            }),
            DomainEvent::FolderDeleted(FolderDeletedPayload {
                folder_id: FolderId(nid()),
                project_id: None,
            }),
        ]
    }

    /// The family every domain event must be gated by. The match has no catch-all,
    /// so a new `DomainEvent` variant forces a compile error here — the conscious
    /// classification the design requires — rather than silently failing open or
    /// closed at runtime.
    fn expected_family(event: &DomainEvent) -> CapabilityFamily {
        match event {
            DomainEvent::TaskCreated(_)
            | DomainEvent::TaskUpdated(_)
            | DomainEvent::TaskMoved(_)
            | DomainEvent::TaskDeleted(_) => CapabilityFamily::Tasks,
            DomainEvent::DocumentCreated(_)
            | DomainEvent::DocumentUpdated(_)
            | DomainEvent::DocumentMoved(_)
            | DomainEvent::DocumentDeleted(_) => CapabilityFamily::Docs,
            DomainEvent::BoardCreated(_)
            | DomainEvent::BoardUpdated(_)
            | DomainEvent::BoardDeleted(_)
            | DomainEvent::BoardMoved(_)
            | DomainEvent::ColumnCreated(_)
            | DomainEvent::ColumnDeleted(_) => CapabilityFamily::Boards,
            DomainEvent::FolderCreated(_) | DomainEvent::FolderDeleted(_) => {
                CapabilityFamily::Folders
            }
        }
    }

    fn live_event(
        event_type: &str,
        board_id: Option<Uuid>,
        document_id: Option<Uuid>,
    ) -> LiveEvent {
        LiveEvent {
            workspace_id: Uuid::now_v7(),
            project_id: None,
            board_id,
            document_id,
            event_type: event_type.to_string(),
            payload: Arc::from("{}"),
        }
    }

    fn read_scopes(families: &[CapabilityFamily]) -> ReadScopeSet {
        let scopes: Vec<Capability> = families
            .iter()
            .map(|family| Capability {
                family: *family,
                action: CapabilityAction::Read,
            })
            .collect();

        ReadScopeSet::from_scopes(&scopes)
    }

    #[test]
    fn every_domain_event_type_classifies_to_its_family() {
        for event in all_domain_events() {
            let expected = expected_family(&event);
            match classify_event_type(event.event_type()) {
                EventTypeFamily::Family(family) => assert_eq!(
                    family,
                    expected,
                    "{} must gate on {expected:?}",
                    event.event_type()
                ),
                EventTypeFamily::Presence | EventTypeFamily::Unknown => {
                    panic!("{} must classify as a family", event.event_type())
                }
            }
        }
    }

    #[test]
    fn presence_family_resolves_by_routing_id_most_specific_first() {
        let doc = Uuid::now_v7();
        let board = Uuid::now_v7();

        // Document presence gates on Docs even when a board id is also present.
        assert_eq!(
            presence_read_family(&live_event("presence.updated", Some(board), Some(doc))),
            Some(CapabilityFamily::Docs)
        );
        assert_eq!(
            presence_read_family(&live_event("presence.updated", Some(board), None)),
            Some(CapabilityFamily::Boards)
        );
        assert_eq!(
            presence_read_family(&live_event("presence.updated", None, None)),
            None
        );
    }

    #[test]
    fn scoped_agent_receives_only_held_families() {
        let scopes = read_scopes(&[CapabilityFamily::Tasks, CapabilityFamily::Docs]);

        assert!(event_allowed_by_read_scopes(
            &scopes,
            &live_event("task.created", None, None)
        ));
        assert!(event_allowed_by_read_scopes(
            &scopes,
            &live_event("document.updated", None, None)
        ));
        assert!(!event_allowed_by_read_scopes(
            &scopes,
            &live_event("board.created", None, None)
        ));
        assert!(!event_allowed_by_read_scopes(
            &scopes,
            &live_event("column.created", None, None)
        ));
        assert!(!event_allowed_by_read_scopes(
            &scopes,
            &live_event("folder.created", None, None)
        ));
        assert!(!event_allowed_by_read_scopes(
            &scopes,
            &live_event("project.created", None, None)
        ));
    }

    #[test]
    fn presence_gate_follows_routed_family() {
        let doc_only = read_scopes(&[CapabilityFamily::Docs]);
        let board_only = read_scopes(&[CapabilityFamily::Boards]);
        let doc = Uuid::now_v7();
        let board = Uuid::now_v7();

        assert!(event_allowed_by_read_scopes(
            &doc_only,
            &live_event("presence.updated", None, Some(doc))
        ));
        assert!(!event_allowed_by_read_scopes(
            &doc_only,
            &live_event("presence.updated", Some(board), None)
        ));
        assert!(event_allowed_by_read_scopes(
            &board_only,
            &live_event("presence.updated", Some(board), None)
        ));

        // Workspace-level presence (no routing id) is forwarded once connected,
        // regardless of which families the key holds.
        let none = read_scopes(&[]);
        assert!(event_allowed_by_read_scopes(
            &none,
            &live_event("presence.updated", None, None)
        ));
    }

    #[test]
    fn unknown_event_type_is_dropped_for_scoped_agent() {
        // A key holding every read family still drops an event whose type maps to
        // no known family: fail closed, never fail open.
        let all = read_scopes(&[
            CapabilityFamily::Tasks,
            CapabilityFamily::Docs,
            CapabilityFamily::Boards,
            CapabilityFamily::Folders,
            CapabilityFamily::Projects,
        ]);

        assert!(!event_allowed_by_read_scopes(
            &all,
            &live_event("widget.exploded", None, None)
        ));
        assert!(!event_allowed_by_read_scopes(
            &all,
            &live_event("no_delimiter", None, None)
        ));
    }
}
