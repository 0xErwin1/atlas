//! Acta-owned migrations.
//!
//! `crates/migration` stays frozen as the historical block (D5 from S3,
//! unchanged rationale): every migration authored before S3 keeps living
//! there, unmodified, because sea-orm records applied migrations by name in
//! a single `seaql_migrations` table and re-partitioning history by owner
//! would reorder what already applied to every existing database.
//!
//! `ComposedMigrator::migrations()` is `historical() ++ custos_new() ++
//! acta_new()`: the composed list always starts with every historical
//! migration name, in order, followed by whatever Custos owns, followed by
//! whatever Acta owns.

mod m20260831_000052_acta_platform_ui_state;

use sea_orm_migration::prelude::MigrationTrait;

/// Migrations owned by Acta, applied after `custos_new()`.
pub fn acta_new() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(m20260831_000052_acta_platform_ui_state::Migration)]
}
