// R1 scaffolding: the `tag` entity struct and its `_from` conversion now
// live in `atlas_acta_postgres::entities::tags` (S4 PR4). Re-exporting here
// keeps every existing `crate::persistence::entities::tags::*` call site
// unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::tags::*;
