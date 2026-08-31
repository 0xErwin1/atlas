// R1 scaffolding: the automation_rules entity struct now lives in
// `atlas_acta_postgres::entities::automation_rule` (S4 PR5). Re-exporting
// here keeps every existing `crate::persistence::entities::automation_rule::*`
// call site unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::automation_rule::*;
