//! Background tasks that keep the presence registry fresh.
//!
//! Two long-lived loops build on the registry and broadcast helper defined in the
//! parent module:
//! - [`run_presence_sweeper`] periodically expires entries whose last heartbeat is
//!   older than [`PRESENCE_TTL`] and re-broadcasts the affected resources;
//! - [`run_presence_agent_consumer`] watches the live-event stream and marks an
//!   api-key principal present on the board or document it keeps mutating, for as
//!   long as it keeps mutating it, with no agent-side changes.
//!
//! Both mirror the shutdown discipline of `crate::live::run_listener`: each
//! blocking await is raced against a `watch` shutdown signal, the loop never
//! panics, and errors are logged rather than swallowed.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tracing::{debug, info};
use uuid::Uuid;

use atlas_api::dtos::documents::ActorDto;
use atlas_domain::{
    Actor, WorkspaceCtx,
    ids::{ApiKeyId, WorkspaceId},
};

use crate::live::LiveEvent;
use crate::routes::tasks::resolve_actor_dto;
use crate::state::AppState;

use super::{PresenceResource, broadcast_presence};

/// A presence entry expires this long after its last heartbeat or agent activity.
///
/// The web client heartbeats every ~20s, comfortably under this window, so a
/// still-present viewer is never swept.
const PRESENCE_TTL: Duration = Duration::from_secs(45);

/// How often the sweeper scans for and removes expired presence entries.
const PRESENCE_SWEEP_INTERVAL: Duration = Duration::from_secs(15);

/// Periodically removes presence entries older than [`PRESENCE_TTL`] and
/// broadcasts a fresh snapshot for every resource whose visible set changed.
///
/// Runs until `shutdown` flips to `true`. The acting project is unknown at sweep
/// time, so `broadcast_presence` is called with `None`; the SSE filter then
/// resolves the resource's scope itself, which is acceptable at this cadence.
pub async fn run_presence_sweeper(state: AppState, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(PRESENCE_SWEEP_INTERVAL) => {
                let changed = state.presence.sweep(PRESENCE_TTL);

                for (workspace, resource) in changed {
                    broadcast_presence(&state, workspace, resource, None);
                }
            }
        }
    }

    info!("presence sweeper shutting down");
}

/// Marks an api-key principal present on the board or document it keeps mutating,
/// for as long as it keeps mutating it, driven entirely by the live-event stream.
///
/// Subscribes once to the live hub and, until `shutdown` flips to `true`, refreshes
/// the acting agent's presence on every qualifying event's resource. A genuinely new
/// presence triggers a `presence.updated` broadcast; the far more common refresh
/// only bumps `last_seen`, leaving the sweeper to remove the agent 45s after its
/// last action. A lagging subscriber skips the missed events (presence is
/// best-effort); a closed channel ends the loop.
pub async fn run_presence_agent_consumer(state: AppState, mut shutdown: watch::Receiver<bool>) {
    let mut receiver = state.live.subscribe();
    let mut resolved: HashMap<Uuid, ActorDto> = HashMap::new();

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            received = receiver.recv() => match received {
                Ok(event) => handle_agent_event(&state, &mut resolved, event).await,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        }
    }

    info!("presence agent consumer shutting down");
}

/// The minimal actor projection parsed out of a live event's payload envelope.
///
/// Only the acting principal is read; every other envelope field is ignored so a
/// new event shape never breaks presence tracking.
#[derive(Deserialize)]
struct PayloadActor {
    actor: ActorRef,
}

#[derive(Deserialize)]
struct ActorRef {
    #[serde(rename = "type")]
    actor_type: String,
    id: Uuid,
}

/// The document id read out of a `document.*` event's `data` payload, used to key
/// document presence for the acting agent. Every other field is ignored.
#[derive(Deserialize)]
struct PayloadDocument {
    data: DocumentRef,
}

#[derive(Deserialize)]
struct DocumentRef {
    document_id: Uuid,
}

/// Determines which resource an agent event marks the acting agent present on, or
/// `None` when the event is not presence-relevant.
///
/// A board-scoped event keys board presence directly from the routing envelope. A
/// `document.*` mutation keys document presence from `data.document_id`; a delete
/// is excluded, since presence on a removed document would never reach a viewer.
fn agent_presence_resource(event: &LiveEvent) -> Option<PresenceResource> {
    if let Some(board) = event.board_id {
        return Some(PresenceResource::Board(board));
    }

    let is_document_edit = matches!(
        event.event_type.as_str(),
        "document.created" | "document.updated" | "document.moved"
    );
    if !is_document_edit {
        return None;
    }

    match serde_json::from_str::<PayloadDocument>(&event.payload) {
        Ok(parsed) => Some(PresenceResource::Document(parsed.data.document_id)),
        Err(error) => {
            debug!(error = %error, "presence consumer could not parse event document id");
            None
        }
    }
}

/// Processes one live event for the agent-activity consumer, refreshing or
/// establishing the acting agent's presence on the event's resource when applicable.
///
/// Events are ignored unless they resolve to a presence resource (a board, or a
/// document being edited) and carry an api-key actor. `presence.*` events are
/// always skipped so the consumer never reacts to its own broadcasts, which would
/// otherwise form a feedback loop.
async fn handle_agent_event(
    state: &AppState,
    resolved: &mut HashMap<Uuid, ActorDto>,
    event: LiveEvent,
) {
    if event.event_type.starts_with("presence.") {
        return;
    }

    let Some(resource) = agent_presence_resource(&event) else {
        return;
    };

    let actor_ref = match serde_json::from_str::<PayloadActor>(&event.payload) {
        Ok(parsed) => parsed.actor,
        Err(error) => {
            debug!(error = %error, "presence consumer could not parse event actor");
            return;
        }
    };

    if actor_ref.actor_type != "api_key" {
        return;
    }

    let Some(actor_dto) =
        resolve_agent_actor(state, resolved, event.workspace_id, actor_ref.id).await
    else {
        return;
    };

    let changed = state
        .presence
        .heartbeat(event.workspace_id, resource, actor_dto);
    if changed {
        broadcast_presence(state, event.workspace_id, resource, event.project_id);
    }
}

/// Resolves an api-key id to its `ActorDto`, caching successful resolutions for
/// the loop's lifetime so a burst of agent edits does not re-hit the database.
///
/// Returns `None` when the key cannot be resolved (deleted or unknown). Api-key
/// names are `NOT NULL`, so a resolved key always carries a `display_name`; its
/// absence is the signal that the key row is gone, and such an event is skipped
/// rather than tearing down the loop.
async fn resolve_agent_actor(
    state: &AppState,
    resolved: &mut HashMap<Uuid, ActorDto>,
    workspace_id: Uuid,
    api_key_id: Uuid,
) -> Option<ActorDto> {
    if let Some(dto) = resolved.get(&api_key_id) {
        return Some(dto.clone());
    }

    let actor = Actor::ApiKey(ApiKeyId(api_key_id));
    let ctx = WorkspaceCtx::new(WorkspaceId(workspace_id), actor);
    let dto = resolve_actor_dto(state, &ctx, &actor).await;

    if dto.display_name.is_none() {
        debug!(api_key_id = %api_key_id, "presence consumer skipping unresolved api key");
        return None;
    }

    resolved.insert(api_key_id, dto.clone());
    Some(dto)
}
