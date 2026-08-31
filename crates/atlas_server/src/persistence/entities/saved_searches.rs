// R1 scaffolding: the saved_searches entity struct, its `_from` conversion,
// and `owner_from_columns` now live in
// `atlas_acta_postgres::entities::saved_searches` (S4 PR5). Re-exporting here
// keeps every existing `crate::persistence::entities::saved_searches::*` call
// site unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::saved_searches::*;
