//! Facade: `PgSearchIndexQueueRepo` moved to
//! `atlas_acta_postgres::repos::search_index_queue` (S4 PR8). Re-exported
//! here so existing `crate::persistence::repos::*` call sites keep
//! resolving unchanged.

pub use atlas_acta_postgres::repos::search_index_queue::{
    PgSearchIndexQueueRepo, QueuedResource, WorkspaceIndexPlan,
};
