// R1 scaffolding: the `event_outbox` entity struct now lives in
// `atlas_acta_postgres::entities::events_outbox` (S4 PR4). Re-exporting here
// keeps every existing `crate::persistence::entities::events_outbox::*` call
// site unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::events_outbox::*;
