// R1 scaffolding: the status-templates entity structs and their `_from`
// conversions now live in `atlas_acta_postgres::entities::status_templates`
// (S4 PR3). Re-exporting here keeps every existing
// `crate::persistence::entities::status_templates::*` call site unaffected by
// the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::status_templates::*;
