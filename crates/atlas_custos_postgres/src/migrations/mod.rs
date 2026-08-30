//! Custos-owned migrations.
//!
//! `crates/migration` stays frozen as the historical block (D5): every
//! migration authored before S3 keeps living there, unmodified, because
//! sea-orm records applied migrations by name in a single `seaql_migrations`
//! table and re-partitioning history by owner would reorder what already
//! applied to every existing database.
//!
//! New Custos DDL is owned here instead. `atlas_server` composes
//! `historical() ++ custos_new()` into one `MigratorTrait`.

mod m20260830_000050_grant_resource_ref;
mod m20260830_000051_custos_set_schema;

use sea_orm_migration::prelude::MigrationTrait;

/// Migrations owned by Custos, applied after the historical block.
pub fn custos_new() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m20260830_000050_grant_resource_ref::Migration),
        Box::new(m20260830_000051_custos_set_schema::Migration),
    ]
}
