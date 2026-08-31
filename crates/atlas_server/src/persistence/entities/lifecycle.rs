// R1 scaffolding: the purge_operations/purge_operation_digests entity structs
// now live in `atlas_acta_postgres::entities::lifecycle` (S4 PR5).
// Re-exporting here keeps every existing
// `crate::persistence::entities::lifecycle::*` call site unaffected by the
// move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::lifecycle::*;
