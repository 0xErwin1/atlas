//! In-memory board presence registry and the `presence.updated` broadcast helper.
//!
//! Presence is ephemeral awareness state: which principals are currently viewing
//! a given board. It is deliberately **not** persisted — it lives only in this
//! process, keyed `workspace → board → principal`, and is refreshed by client
//! heartbeats. This module provides the registry, the change signal, and the
//! live-event fan-out; the [`tasks`] submodule adds the TTL sweeper and the
//! agent-activity consumer that drive them from background tasks.

mod tasks;

pub use tasks::{run_presence_agent_consumer, run_presence_sweeper};

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::time::Instant;
use uuid::Uuid;

use atlas_api::dtos::documents::ActorDto;

use crate::live::LiveEvent;
use crate::state::AppState;

/// Stable identity of a present principal within a board's presence set.
///
/// Derived from an actor's type and id, so the same principal heartbeating twice
/// is one entry rather than two. Keyed separately by kind because a user id and an
/// api-key id are distinct namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrincipalKey {
    User(Uuid),
    ApiKey(Uuid),
}

impl PrincipalKey {
    /// Derives the presence key from a resolved `ActorDto`.
    ///
    /// `ActorDto::type` is only ever `"user"` or `"api_key"` in this codebase; any
    /// other value is treated as a user so a malformed actor can never be keyed as
    /// a distinct api-key principal.
    pub fn from_actor_dto(actor: &ActorDto) -> Self {
        match actor.r#type.as_str() {
            "api_key" => PrincipalKey::ApiKey(actor.id),
            _ => PrincipalKey::User(actor.id),
        }
    }
}

/// One principal's presence on a board: who they are plus when they were last seen.
#[derive(Debug, Clone)]
struct PresenceEntry {
    actor: ActorDto,
    last_seen: Instant,
}

type BoardPresence = HashMap<PrincipalKey, PresenceEntry>;
type WorkspacePresence = HashMap<Uuid, BoardPresence>;

/// In-memory, process-local registry of who is present on which board.
///
/// All operations take a short critical section over a `std::sync::Mutex` and
/// never await while holding the lock, so the standard mutex is sufficient and a
/// slow client can never stall an unrelated board's updates.
#[derive(Default)]
pub struct PresenceRegistry {
    by_workspace: Mutex<HashMap<Uuid, WorkspacePresence>>,
}

impl PresenceRegistry {
    /// Recovers the guard even if a prior holder panicked; presence is advisory
    /// state whose partial contents remain safe to read and prune.
    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, WorkspacePresence>> {
        self.by_workspace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records that `actor` is present on `board` in `workspace`, refreshing their
    /// `last_seen`.
    ///
    /// Returns `true` when this changes the visible set (a principal that was not
    /// already present joined), and `false` when it only refreshes an existing
    /// presence. Callers broadcast only on `true`.
    pub fn heartbeat(&self, workspace: Uuid, board: Uuid, actor: ActorDto) -> bool {
        let key = PrincipalKey::from_actor_dto(&actor);
        let entry = PresenceEntry {
            actor,
            last_seen: Instant::now(),
        };

        let mut guard = self.lock();
        let board_presence = guard
            .entry(workspace)
            .or_default()
            .entry(board)
            .or_default();

        match board_presence.get_mut(&key) {
            Some(existing) => {
                existing.last_seen = entry.last_seen;
                existing.actor = entry.actor;
                false
            }
            None => {
                board_presence.insert(key, entry);
                true
            }
        }
    }

    /// Removes `key` from `board`'s presence set, pruning any now-empty maps.
    ///
    /// Returns `true` when something was removed (the visible set changed), and
    /// `false` when the principal was not present.
    pub fn leave(&self, workspace: Uuid, board: Uuid, key: &PrincipalKey) -> bool {
        let mut guard = self.lock();

        let Some(workspace_presence) = guard.get_mut(&workspace) else {
            return false;
        };

        let Some(board_presence) = workspace_presence.get_mut(&board) else {
            return false;
        };

        let removed = board_presence.remove(key).is_some();

        if board_presence.is_empty() {
            workspace_presence.remove(&board);
        }
        if workspace_presence.is_empty() {
            guard.remove(&workspace);
        }

        removed
    }

    /// Returns the current present actors on `board`, sorted by principal key so the
    /// output is deterministic and testable.
    pub fn snapshot(&self, workspace: Uuid, board: Uuid) -> Vec<ActorDto> {
        let guard = self.lock();

        let Some(board_presence) = guard.get(&workspace).and_then(|w| w.get(&board)) else {
            return Vec::new();
        };

        let mut entries: Vec<(PrincipalKey, ActorDto)> = board_presence
            .iter()
            .map(|(key, entry)| (*key, entry.actor.clone()))
            .collect();

        entries.sort_by_key(|(key, _)| *key);
        entries.into_iter().map(|(_, actor)| actor).collect()
    }

    /// Removes every entry not seen within `ttl` and returns the `(workspace, board)`
    /// pairs whose visible set changed as a result, sorted for determinism.
    ///
    /// Empty inner maps are pruned so a board with no remaining presence leaves no
    /// residue. [`run_presence_sweeper`] drives this on a fixed interval.
    pub fn sweep(&self, ttl: Duration) -> Vec<(Uuid, Uuid)> {
        let mut changed: Vec<(Uuid, Uuid)> = Vec::new();
        let mut guard = self.lock();

        let workspaces: Vec<Uuid> = guard.keys().copied().collect();

        for workspace in workspaces {
            let Some(workspace_presence) = guard.get_mut(&workspace) else {
                continue;
            };

            let boards: Vec<Uuid> = workspace_presence.keys().copied().collect();

            for board in boards {
                let Some(board_presence) = workspace_presence.get_mut(&board) else {
                    continue;
                };

                let before = board_presence.len();
                board_presence.retain(|_, entry| entry.last_seen.elapsed() < ttl);

                if board_presence.len() != before {
                    changed.push((workspace, board));
                }

                if board_presence.is_empty() {
                    workspace_presence.remove(&board);
                }
            }

            if workspace_presence.is_empty() {
                guard.remove(&workspace);
            }
        }

        changed.sort();
        changed
    }
}

/// Publishes a `presence.updated` live event carrying the current presence of
/// `board`, restricted to viewers of that board by the SSE authorization filter.
///
/// The event is published directly to the in-process hub (never persisted to the
/// outbox): presence is transient and must not survive a restart. `project` is
/// forwarded as the event's `project_id` when known so the SSE filter can resolve
/// board access without an extra board lookup. The payload mirrors the persisted
/// envelopes (`event_type` + `workspace_id` + `board_id` + `data`) so clients parse
/// every live frame uniformly.
pub fn broadcast_board_presence(
    state: &AppState,
    workspace: Uuid,
    board: Uuid,
    project: Option<Uuid>,
) {
    let actors = state.presence.snapshot(workspace, board);

    let payload = serde_json::json!({
        "event_type": "presence.updated",
        "workspace_id": workspace,
        "board_id": board,
        "data": {
            "board_id": board,
            "actors": actors,
        },
    })
    .to_string();

    state.live.publish(LiveEvent {
        workspace_id: workspace,
        project_id: project,
        board_id: Some(board),
        event_type: "presence.updated".to_string(),
        payload: Arc::from(payload.as_str()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_actor(id: Uuid) -> ActorDto {
        ActorDto {
            r#type: "user".into(),
            id,
            display_name: Some("User".into()),
            key_type: None,
            account_status: None,
        }
    }

    fn api_key_actor(id: Uuid) -> ActorDto {
        ActorDto {
            r#type: "api_key".into(),
            id,
            display_name: Some("Agent".into()),
            key_type: Some("agent".into()),
            account_status: None,
        }
    }

    #[test]
    fn heartbeat_reports_join_then_refresh() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = Uuid::now_v7();
        let actor = user_actor(Uuid::now_v7());

        assert!(
            registry.heartbeat(ws, board, actor.clone()),
            "first heartbeat for a principal is a change"
        );
        assert!(
            !registry.heartbeat(ws, board, actor),
            "re-heartbeat of the same principal is only a refresh"
        );
    }

    #[test]
    fn distinct_principals_each_count_as_a_change() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = Uuid::now_v7();

        let user_id = Uuid::now_v7();

        assert!(registry.heartbeat(ws, board, user_actor(user_id)));
        // Same id but a different principal kind is a distinct presence.
        assert!(registry.heartbeat(ws, board, api_key_actor(user_id)));

        assert_eq!(registry.snapshot(ws, board).len(), 2);
    }

    #[test]
    fn snapshot_is_sorted_by_principal_key() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = Uuid::now_v7();

        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);

        registry.heartbeat(ws, board, user_actor(second));
        registry.heartbeat(ws, board, user_actor(first));

        let ids: Vec<Uuid> = registry
            .snapshot(ws, board)
            .into_iter()
            .map(|a| a.id)
            .collect();

        assert_eq!(ids, vec![first, second], "snapshot is ordered by id");
    }

    #[test]
    fn snapshot_of_unknown_board_is_empty() {
        let registry = PresenceRegistry::default();
        assert!(registry.snapshot(Uuid::now_v7(), Uuid::now_v7()).is_empty());
    }

    #[test]
    fn leave_reports_removal_and_absence() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = Uuid::now_v7();
        let id = Uuid::now_v7();

        registry.heartbeat(ws, board, user_actor(id));

        let key = PrincipalKey::User(id);
        assert!(
            registry.leave(ws, board, &key),
            "removing a present principal is a change"
        );
        assert!(
            !registry.leave(ws, board, &key),
            "removing an absent principal is not a change"
        );
        assert!(registry.snapshot(ws, board).is_empty());
    }

    #[test]
    fn leave_prunes_empty_maps() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = Uuid::now_v7();
        let id = Uuid::now_v7();

        registry.heartbeat(ws, board, user_actor(id));
        registry.leave(ws, board, &PrincipalKey::User(id));

        // The workspace map is pruned once its last board empties: a fresh sweep sees
        // nothing to do.
        assert!(registry.sweep(Duration::from_secs(0)).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_removes_stale_entries_and_reports_boards() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let stale_board = Uuid::now_v7();
        let fresh_board = Uuid::now_v7();

        let stale_id = Uuid::now_v7();
        registry.heartbeat(ws, stale_board, user_actor(stale_id));

        tokio::time::advance(Duration::from_secs(60)).await;

        // Heartbeated after the advance, so this one is within the TTL window.
        registry.heartbeat(ws, fresh_board, user_actor(Uuid::now_v7()));

        let changed = registry.sweep(Duration::from_secs(30));

        assert_eq!(
            changed,
            vec![(ws, stale_board)],
            "only the board with an expired entry is reported changed"
        );
        assert!(
            registry.snapshot(ws, stale_board).is_empty(),
            "the stale entry is removed"
        );
        assert_eq!(
            registry.snapshot(ws, fresh_board).len(),
            1,
            "the fresh entry survives"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_reports_nothing_when_all_fresh() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = Uuid::now_v7();

        registry.heartbeat(ws, board, user_actor(Uuid::now_v7()));

        tokio::time::advance(Duration::from_secs(5)).await;

        assert!(
            registry.sweep(Duration::from_secs(30)).is_empty(),
            "no entry is older than the TTL"
        );
        assert_eq!(registry.snapshot(ws, board).len(), 1);
    }
}
