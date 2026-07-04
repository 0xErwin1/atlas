//! In-memory presence registry and the `presence.updated` broadcast helper.
//!
//! Presence is ephemeral awareness state: which principals are currently viewing
//! a given resource — a board, or a document being edited. It is deliberately
//! **not** persisted — it lives only in this process, keyed
//! `workspace → resource → principal`, and is refreshed by client heartbeats.
//! This module provides the registry, the change signal, and the live-event
//! fan-out; the [`tasks`] submodule adds the TTL sweeper and the agent-activity
//! consumer that drive them from background tasks.

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

/// A presence-tracked resource within a workspace.
///
/// Board and document ids share the `Uuid` type but are distinct namespaces, so
/// the kind is kept in the key: a board and a document that happened to share an
/// id would never collide, and the broadcast helper knows which id field to route
/// the resulting live event on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PresenceResource {
    Board(Uuid),
    Document(Uuid),
}

/// One principal's presence on a resource: who they are plus when they were last seen.
#[derive(Debug, Clone)]
struct PresenceEntry {
    actor: ActorDto,
    last_seen: Instant,
}

type ResourcePresence = HashMap<PrincipalKey, PresenceEntry>;
type WorkspacePresence = HashMap<PresenceResource, ResourcePresence>;

/// In-memory, process-local registry of who is present on which resource.
///
/// All operations take a short critical section over a `std::sync::Mutex` and
/// never await while holding the lock, so the standard mutex is sufficient and a
/// slow client can never stall an unrelated resource's updates.
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

    /// Records that `actor` is present on `resource` in `workspace`, refreshing
    /// their `last_seen`.
    ///
    /// Returns `true` when this changes the visible set (a principal that was not
    /// already present joined), and `false` when it only refreshes an existing
    /// presence. Callers broadcast only on `true`.
    pub fn heartbeat(&self, workspace: Uuid, resource: PresenceResource, actor: ActorDto) -> bool {
        let key = PrincipalKey::from_actor_dto(&actor);
        let entry = PresenceEntry {
            actor,
            last_seen: Instant::now(),
        };

        let mut guard = self.lock();
        let resource_presence = guard
            .entry(workspace)
            .or_default()
            .entry(resource)
            .or_default();

        match resource_presence.get_mut(&key) {
            Some(existing) => {
                existing.last_seen = entry.last_seen;
                existing.actor = entry.actor;
                false
            }
            None => {
                resource_presence.insert(key, entry);
                true
            }
        }
    }

    /// Removes `key` from `resource`'s presence set, pruning any now-empty maps.
    ///
    /// Returns `true` when something was removed (the visible set changed), and
    /// `false` when the principal was not present.
    pub fn leave(&self, workspace: Uuid, resource: PresenceResource, key: &PrincipalKey) -> bool {
        let mut guard = self.lock();

        let Some(workspace_presence) = guard.get_mut(&workspace) else {
            return false;
        };

        let Some(resource_presence) = workspace_presence.get_mut(&resource) else {
            return false;
        };

        let removed = resource_presence.remove(key).is_some();

        if resource_presence.is_empty() {
            workspace_presence.remove(&resource);
        }
        if workspace_presence.is_empty() {
            guard.remove(&workspace);
        }

        removed
    }

    /// Returns the current present actors on `resource`, sorted by principal key so
    /// the output is deterministic and testable.
    pub fn snapshot(&self, workspace: Uuid, resource: PresenceResource) -> Vec<ActorDto> {
        let guard = self.lock();

        let Some(resource_presence) = guard.get(&workspace).and_then(|w| w.get(&resource)) else {
            return Vec::new();
        };

        let mut entries: Vec<(PrincipalKey, ActorDto)> = resource_presence
            .iter()
            .map(|(key, entry)| (*key, entry.actor.clone()))
            .collect();

        entries.sort_by_key(|(key, _)| *key);
        entries.into_iter().map(|(_, actor)| actor).collect()
    }

    /// Removes every entry not seen within `ttl` and returns the
    /// `(workspace, resource)` pairs whose visible set changed as a result, sorted
    /// for determinism.
    ///
    /// Empty inner maps are pruned so a resource with no remaining presence leaves
    /// no residue. [`run_presence_sweeper`] drives this on a fixed interval.
    pub fn sweep(&self, ttl: Duration) -> Vec<(Uuid, PresenceResource)> {
        let mut changed: Vec<(Uuid, PresenceResource)> = Vec::new();
        let mut guard = self.lock();

        let workspaces: Vec<Uuid> = guard.keys().copied().collect();

        for workspace in workspaces {
            let Some(workspace_presence) = guard.get_mut(&workspace) else {
                continue;
            };

            let resources: Vec<PresenceResource> = workspace_presence.keys().copied().collect();

            for resource in resources {
                let Some(resource_presence) = workspace_presence.get_mut(&resource) else {
                    continue;
                };

                let before = resource_presence.len();
                resource_presence.retain(|_, entry| entry.last_seen.elapsed() < ttl);

                if resource_presence.len() != before {
                    changed.push((workspace, resource));
                }

                if resource_presence.is_empty() {
                    workspace_presence.remove(&resource);
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
/// `resource`, restricted by the SSE authorization filter to principals who may
/// view that resource.
///
/// The event is published directly to the in-process hub (never persisted to the
/// outbox): presence is transient and must not survive a restart. It carries the
/// resource id on the matching routing field (`board_id` for a board,
/// `document_id` for a document) so the SSE filter authorizes it against that same
/// resource; `project` is forwarded as the event's `project_id` when known so a
/// board decision can resolve without an extra board lookup. Document events omit
/// the project hint so they are always gated by the finer per-document chain. The
/// payload mirrors the persisted envelopes (`event_type` + `workspace_id` +
/// routing ids + `data`) so clients parse every live frame uniformly.
pub fn broadcast_presence(
    state: &AppState,
    workspace: Uuid,
    resource: PresenceResource,
    project: Option<Uuid>,
) {
    let actors = state.presence.snapshot(workspace, resource);

    let (board_id, document_id, project_id, data) = match resource {
        PresenceResource::Board(board) => (
            Some(board),
            None,
            project,
            serde_json::json!({ "board_id": board, "actors": actors }),
        ),
        // A document carries no project hint: document presence must be gated by the
        // finer per-document chain, never authorized at the coarser project scope.
        PresenceResource::Document(document) => (
            None,
            Some(document),
            None,
            serde_json::json!({ "document_id": document, "actors": actors }),
        ),
    };

    let payload = serde_json::json!({
        "event_type": "presence.updated",
        "workspace_id": workspace,
        "board_id": board_id,
        "document_id": document_id,
        "data": data,
    })
    .to_string();

    state.live.publish(LiveEvent {
        workspace_id: workspace,
        project_id,
        board_id,
        document_id,
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
        let board = PresenceResource::Board(Uuid::now_v7());
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
        let board = PresenceResource::Board(Uuid::now_v7());

        let user_id = Uuid::now_v7();

        assert!(registry.heartbeat(ws, board, user_actor(user_id)));
        // Same id but a different principal kind is a distinct presence.
        assert!(registry.heartbeat(ws, board, api_key_actor(user_id)));

        assert_eq!(registry.snapshot(ws, board).len(), 2);
    }

    #[test]
    fn same_id_on_board_and_document_are_distinct_resources() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let shared = Uuid::now_v7();
        let board = PresenceResource::Board(shared);
        let document = PresenceResource::Document(shared);

        assert!(registry.heartbeat(ws, board, user_actor(Uuid::now_v7())));
        assert!(registry.heartbeat(ws, document, user_actor(Uuid::now_v7())));

        // A board and a document sharing a raw id are separate presence sets.
        assert_eq!(registry.snapshot(ws, board).len(), 1);
        assert_eq!(registry.snapshot(ws, document).len(), 1);
    }

    #[test]
    fn snapshot_is_sorted_by_principal_key() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = PresenceResource::Board(Uuid::now_v7());

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
    fn snapshot_of_unknown_resource_is_empty() {
        let registry = PresenceRegistry::default();
        assert!(
            registry
                .snapshot(Uuid::now_v7(), PresenceResource::Board(Uuid::now_v7()))
                .is_empty()
        );
    }

    #[test]
    fn leave_reports_removal_and_absence() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let board = PresenceResource::Board(Uuid::now_v7());
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
        let board = PresenceResource::Board(Uuid::now_v7());
        let id = Uuid::now_v7();

        registry.heartbeat(ws, board, user_actor(id));
        registry.leave(ws, board, &PrincipalKey::User(id));

        // The workspace map is pruned once its last resource empties: a fresh sweep
        // sees nothing to do.
        assert!(registry.sweep(Duration::from_secs(0)).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_removes_stale_entries_and_reports_resources() {
        let registry = PresenceRegistry::default();
        let ws = Uuid::now_v7();
        let stale_board = PresenceResource::Board(Uuid::now_v7());
        let fresh_board = PresenceResource::Board(Uuid::now_v7());

        let stale_id = Uuid::now_v7();
        registry.heartbeat(ws, stale_board, user_actor(stale_id));

        tokio::time::advance(Duration::from_secs(60)).await;

        // Heartbeated after the advance, so this one is within the TTL window.
        registry.heartbeat(ws, fresh_board, user_actor(Uuid::now_v7()));

        let changed = registry.sweep(Duration::from_secs(30));

        assert_eq!(
            changed,
            vec![(ws, stale_board)],
            "only the resource with an expired entry is reported changed"
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
        let board = PresenceResource::Board(Uuid::now_v7());

        registry.heartbeat(ws, board, user_actor(Uuid::now_v7()));

        tokio::time::advance(Duration::from_secs(5)).await;

        assert!(
            registry.sweep(Duration::from_secs(30)).is_empty(),
            "no entry is older than the TTL"
        );
        assert_eq!(registry.snapshot(ws, board).len(), 1);
    }
}
