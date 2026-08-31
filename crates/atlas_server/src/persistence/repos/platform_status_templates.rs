// R1 scaffolding: `PgPlatformStatusTemplateRepo` now lives in
// `atlas_acta_postgres::repos::platform_status_templates`
// (orchestrator-mandated addition to S4 PR9). Re-exporting it here keeps
// every existing `crate::persistence::repos::*` call site unaffected by the
// move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::repos::platform_status_templates::{
    PgPlatformStatusTemplateRepo, PlatformStatusTemplateRepo,
};
