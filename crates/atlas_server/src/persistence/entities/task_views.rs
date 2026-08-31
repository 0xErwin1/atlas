// R1 scaffolding: the task_views entity struct, its `_from` conversion, and
// `owner_from_columns` now live in `atlas_acta_postgres::entities::task_views`
// (S4 PR5). Re-exporting here keeps every existing
// `crate::persistence::entities::task_views::*` call site unaffected by the
// move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::task_views::*;
