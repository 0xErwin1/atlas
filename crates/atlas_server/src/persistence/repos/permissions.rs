//! Domain-crate re-export for the permission-grant port.
//!
//! `PgGroupRepo`/`PgGrantHygiene`/`PgPermissionGrantRepo` moved to
//! `atlas_custos_postgres` (S3a, then S5 PR4 retired this facade's re-export
//! of them; call sites now import `atlas_custos_postgres` directly).
//! `PermissionGrantRepo` is an `atlas_custos` domain-crate port, not a
//! Postgres-owned type, so it stays re-exported here for
//! `crate::persistence::repos::PermissionGrantRepo` call sites.

pub use atlas_custos::ports::grant_repo::PermissionGrantRepo;
