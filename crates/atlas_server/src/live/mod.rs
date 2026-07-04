use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::{broadcast, watch};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Postgres `LISTEN`/`NOTIFY` channel carrying `events_outbox` inserts.
///
/// Kept in sync with the trigger installed by the
/// `m20260702_000036_events_outbox_notify` migration.
const EVENT_CHANNEL: &str = "atlas_events";

/// Backoff applied before the listener reconnects after its `LISTEN`
/// connection drops, so a persistently unavailable database does not turn into
/// a reconnect busy-loop.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(1);

/// Default capacity of the hub's broadcast ring buffer.
///
/// The channel is bounded: once full, the oldest events are overwritten and a
/// lagging subscriber observes `broadcast::error::RecvError::Lagged` on its next
/// `recv`. Publishing never blocks on a slow subscriber. Surfacing a resync
/// signal to clients on lag is the SSE layer's responsibility (a later work
/// unit); the hub only guarantees non-blocking fan-out.
pub const DEFAULT_HUB_CAPACITY: usize = 1024;

/// A live event ready to be fanned out to streaming clients.
///
/// Carries the routing metadata needed for later per-resource filtering plus the
/// raw envelope JSON in `payload`, which is forwarded verbatim to SSE clients.
/// `payload` is an `Arc<str>` so cloning the event across many broadcast
/// subscribers does not re-copy the JSON.
#[derive(Clone, Debug)]
pub struct LiveEvent {
    pub workspace_id: Uuid,
    pub project_id: Option<Uuid>,
    pub board_id: Option<Uuid>,
    /// Set only by document-scoped presence broadcasts, which the SSE filter gates
    /// against the per-document permission chain. Outbox-sourced events never carry
    /// it (their `document_id` lives inside `data`, not the routing envelope), so it
    /// stays `None` for every event fanned out by [`forward_notification`].
    pub document_id: Option<Uuid>,
    pub event_type: String,
    pub payload: Arc<str>,
}

/// In-process fan-out hub for live events.
///
/// Wraps a `tokio::sync::broadcast` sender. Cloning the hub is cheap and shares
/// the same underlying channel, so any clone can publish and every subscriber
/// receives every event published after it subscribed.
#[derive(Clone)]
pub struct LiveEventHub {
    sender: broadcast::Sender<LiveEvent>,
}

impl LiveEventHub {
    /// Creates a hub whose broadcast ring buffer holds up to `capacity` events.
    pub fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Registers a new subscriber that will receive every event published from
    /// now on.
    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.sender.subscribe()
    }

    /// Publishes `event` to all current subscribers.
    ///
    /// `broadcast::Sender::send` returns `SendError` only when there are no
    /// active subscribers. That is the expected state whenever nobody is
    /// streaming, so it is dropped silently rather than surfaced as an error.
    pub fn publish(&self, event: LiveEvent) {
        if self.sender.send(event).is_err() {
            debug!("live event published with no active subscribers");
        }
    }
}

impl Default for LiveEventHub {
    fn default() -> Self {
        Self::new(DEFAULT_HUB_CAPACITY)
    }
}

/// The routing subset of an `EventEnvelope`, extracted from the notification
/// payload for per-resource filtering.
///
/// Only the fields needed to route the event are deserialized; the untagged
/// `data` variant of the full envelope is intentionally not parsed here, so a
/// new event shape never breaks listening. The raw JSON is forwarded to clients
/// unchanged.
#[derive(Deserialize)]
struct EventRouting {
    workspace_id: Uuid,
    event_type: String,
    #[serde(default)]
    project_id: Option<Uuid>,
    #[serde(default)]
    board_id: Option<Uuid>,
}

/// Why a single `LISTEN` session ended.
enum SessionEnd {
    /// A shutdown signal was observed; the run loop must exit.
    Shutdown,
    /// The connection failed; the run loop should reconnect after a backoff.
    Disconnected(sqlx::Error),
}

/// Runs the Postgres `LISTEN` consumer until `shutdown` is set to `true`.
///
/// Mirrors the webhook dispatcher's shutdown discipline: each `recv` is raced
/// against the shutdown signal, and a dropped connection is logged and retried
/// after a bounded backoff (never a busy-loop, and always yielding to shutdown).
/// Errors are logged rather than swallowed; the loop itself never panics.
pub async fn run_listener(pool: PgPool, hub: LiveEventHub, mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            break;
        }

        match listen_session(&pool, &hub, &mut shutdown).await {
            SessionEnd::Shutdown => break,
            SessionEnd::Disconnected(error) => {
                warn!(error = %error, "live event listener disconnected; reconnecting");

                tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    _ = tokio::time::sleep(RECONNECT_BACKOFF) => {}
                }
            }
        }
    }

    info!("live event listener shutting down");
}

/// Opens one `LISTEN` session and forwards notifications until the connection
/// drops or shutdown is signalled.
async fn listen_session(
    pool: &PgPool,
    hub: &LiveEventHub,
    shutdown: &mut watch::Receiver<bool>,
) -> SessionEnd {
    let mut listener = match PgListener::connect_with(pool).await {
        Ok(listener) => listener,
        Err(error) => return SessionEnd::Disconnected(error),
    };

    if let Err(error) = listener.listen(EVENT_CHANNEL).await {
        return SessionEnd::Disconnected(error);
    }

    debug!(channel = EVENT_CHANNEL, "live event listener subscribed");

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => return SessionEnd::Shutdown,
            received = listener.recv() => match received {
                Ok(notification) => forward_notification(hub, notification.payload()),
                Err(error) => return SessionEnd::Disconnected(error),
            },
        }
    }
}

/// Parses the routing fields out of a raw notification payload and publishes a
/// `LiveEvent` carrying the payload verbatim.
///
/// A payload that fails to parse is logged and dropped: it cannot be routed, but
/// it must not tear down the listener.
fn forward_notification(hub: &LiveEventHub, payload: &str) {
    let routing = match serde_json::from_str::<EventRouting>(payload) {
        Ok(routing) => routing,
        Err(error) => {
            error!(error = %error, "failed to parse live event routing fields");
            return;
        }
    };

    hub.publish(LiveEvent {
        workspace_id: routing.workspace_id,
        project_id: routing.project_id,
        board_id: routing.board_id,
        document_id: None,
        event_type: routing.event_type,
        payload: Arc::from(payload),
    });
}
