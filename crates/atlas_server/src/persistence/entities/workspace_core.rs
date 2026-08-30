// R1 scaffolding: every workspace_core entity and `_from` conversion now
// lives in `atlas_acta_postgres` (S4 PR1). Re-exporting them here keeps every
// existing `crate::persistence::entities::workspace_core::*` call site
// unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::workspace_core::*;
