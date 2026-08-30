//! Facade over the Custos-owned grant and group repos.
//!
//! `PgGroupRepo` moved to `atlas_custos_postgres` in S3a (R1 scaffolding).
//! `PermissionGrant`/`PermissionGrantRepo`/`PgPermissionGrantRepo` stayed
//! parked at `atlas_server` composition through S3a/S3b/S3c because they
//! named five Acta id fields; T6.9 unparked the whole cluster once
//! `resource_ref`/`WorkspaceScope` removed the last Acta type from it. This
//! file re-exports the moved types so every existing
//! `crate::persistence::repos::{...}` import path keeps resolving unchanged.

pub use atlas_custos::ports::grant_repo::PermissionGrantRepo;
pub use atlas_custos_postgres::repos::grant_hygiene::PgGrantHygiene;
pub use atlas_custos_postgres::repos::permissions::{PgGroupRepo, PgPermissionGrantRepo};
