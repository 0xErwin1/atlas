// R1 scaffolding: `PgOutboxRepo` now lives in
// `atlas_acta_postgres::repos::outbox` (S4 PR7), moved alongside
// `boards_tasks.rs` — every `PgOutboxRepo::insert_in` call site in that file
// needed it in the same crate. Re-exporting it here keeps every existing
// `crate::persistence::repos::*` call site unaffected.
pub use atlas_acta_postgres::repos::outbox::PgOutboxRepo;
