//! Custos-owned migrations.
//!
//! `crates/migration` stays frozen as the historical block (D5): every
//! migration authored before S3 keeps living there, unmodified, because
//! sea-orm records applied migrations by name in a single `seaql_migrations`
//! table and re-partitioning history by owner would reorder what already
//! applied to every existing database.
//!
//! New Custos DDL is owned here instead. `atlas_server` composes
//! `historical() ++ custos_new()` into one `MigratorTrait`. This slice makes
//! no schema changes, so `custos_new()` is empty; it exists so the ownership
//! boundary is real before the first Custos migration lands.

use sea_orm_migration::prelude::MigrationTrait;

/// Migrations owned by Custos, applied after the historical block.
pub fn custos_new() -> Vec<Box<dyn MigrationTrait>> {
    vec![]
}
