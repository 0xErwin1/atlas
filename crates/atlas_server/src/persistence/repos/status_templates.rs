// R1 scaffolding: `PgStatusTemplateRepo` (and the `list_templates_for_workspace`
// free function) now live in `atlas_acta_postgres::repos::status_templates`
// (orchestrator-mandated addition to S4 PR9). Re-exporting them here keeps
// every existing `crate::persistence::repos::*` call site unaffected by the
// move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::repos::status_templates::{
    PgStatusTemplateRepo, StatusTemplateRepo, list_templates_for_workspace,
};
