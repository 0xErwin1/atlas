// R1 scaffolding: the boards/tasks entity structs, their `_from` conversions,
// and `actor_from_columns` now live in `atlas_acta_postgres::entities::boards_tasks`
// (S4 PR3). Re-exporting here keeps every existing
// `crate::persistence::entities::boards_tasks::*` call site unaffected by the
// move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::boards_tasks::*;
