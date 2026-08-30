//! `WorkspaceCtx` relocated to `atlas_acta::actor` (D3: it is `{ WorkspaceId,
//! Attribution }`, Acta-shaped by construction). Re-exported here to keep
//! every existing `crate::actor::*` import path compiling.
pub use atlas_acta::actor::{Actor, ApiKeyAttributionId, UserAttributionId, WorkspaceCtx};
