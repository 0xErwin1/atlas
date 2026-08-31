// R1 scaffolding: the webhook_subscriptions entity struct now lives in
// `atlas_acta_postgres::entities::webhook_subscription` (S4 PR5).
// Re-exporting here keeps every existing
// `crate::persistence::entities::webhook_subscription::*` call site
// unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::webhook_subscription::*;
