// R1 scaffolding: `document`, `document_revision`, `document_link`,
// `attachment`, `attachment_write_intent` (and their `_from` conversions) now
// live in `atlas_acta_postgres` (S4 PR2). Re-exporting them here keeps every
// existing `crate::persistence::entities::documents::*` call site unaffected
// by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::documents::*;
