//! Facade: `PgSearchRepo` moved to `atlas_acta_postgres::repos::search` (S4
//! PR8). Re-exported here so existing `crate::persistence::repos::*` call
//! sites keep resolving unchanged. `build_doc_permission`/
//! `build_task_permission` are re-exported at `pub(crate)` visibility
//! (widened to `pub` at the new crate-local home, mirroring PR7's
//! `live_ancestors` widening) because `workspace_attachments.rs`, which
//! stays server-side, imports them via
//! `crate::persistence::repos::search::{build_doc_permission,
//! build_task_permission}`.

pub use atlas_acta_postgres::repos::search::PgSearchRepo;
pub(crate) use atlas_acta_postgres::repos::search::{build_doc_permission, build_task_permission};
