//! Facade: `PgWebhookDeliveryRepo` moved to
//! `atlas_acta_postgres::repos::webhook_delivery` (S4 PR8). Re-exported here
//! so existing `crate::persistence::repos::*` call sites keep resolving
//! unchanged.

pub use atlas_acta_postgres::repos::webhook_delivery::PgWebhookDeliveryRepo;
