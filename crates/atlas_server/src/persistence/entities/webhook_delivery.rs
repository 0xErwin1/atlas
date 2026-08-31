// R1 scaffolding: the webhook_delivery_log entity struct now lives in
// `atlas_acta_postgres::entities::webhook_delivery` (S4 PR5). Re-exporting
// here keeps every existing
// `crate::persistence::entities::webhook_delivery::*` call site unaffected by
// the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::webhook_delivery::*;
