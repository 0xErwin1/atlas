#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod capability;
pub mod entities;
pub mod ids;
pub mod ports;

/// The opaque scope key Custos rows are filed under. Custos never dereferences
/// this to a `Workspace` entity (an Acta type) — it is treated purely as a
/// partitioning key, per D2. `atlas_server` constructs it as
/// `WorkspaceScope(ctx.workspace_id.0)` at the composition layer, so the same
/// uuid reaches the same predicate as the pre-move `WorkspaceCtx`-scoped query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct WorkspaceScope(pub uuid::Uuid);

#[cfg(test)]
mod workspace_scope_tests {
    use super::WorkspaceScope;
    use uuid::Uuid;

    #[test]
    fn workspace_scope_round_trips_the_same_uuid() {
        let uuid = Uuid::now_v7();
        let scope = WorkspaceScope(uuid);

        assert_eq!(scope.0, uuid);
    }

    #[test]
    fn distinct_uuids_produce_distinct_scopes() {
        let a = WorkspaceScope(Uuid::now_v7());
        let b = WorkspaceScope(Uuid::now_v7());

        assert_ne!(a, b);
    }
}
