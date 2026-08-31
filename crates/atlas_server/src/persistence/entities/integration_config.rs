// R1 scaffolding: the integration_configs entity struct now lives in
// `atlas_acta_postgres::entities::integration_config` (S4 PR5). Re-exporting
// here keeps every existing
// `crate::persistence::entities::integration_config::*` call site unaffected
// by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::integration_config::*;
