//! Composes the historical migration block with Custos-owned migrations.
//!
//! `crates/migration` stays frozen (D5): it keeps every migration authored
//! before S3, unmodified, because sea-orm records applied migrations by name
//! in one `seaql_migrations` table and re-partitioning history by owner would
//! reorder what already applied to every existing database.
//!
//! `ComposedMigrator::migrations()` is `historical() ++ custos_new()`. Custos
//! has shipped no new DDL yet in this slice, so `custos_new()` is empty and
//! the composed list is identical to the historical one; the ownership
//! boundary exists so the first real Custos migration has somewhere to land.

use sea_orm_migration::prelude::{MigrationTrait, MigratorTrait};

pub struct ComposedMigrator;

impl MigratorTrait for ComposedMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations = migration::Migrator::migrations();
        migrations.extend(atlas_custos_postgres::migrations::custos_new());
        migrations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composed_migrations_equal_the_historical_list_while_custos_new_is_empty() {
        let historical_names: Vec<_> = migration::Migrator::migrations()
            .iter()
            .map(|m| m.name().to_string())
            .collect();
        let composed_names: Vec<_> = ComposedMigrator::migrations()
            .iter()
            .map(|m| m.name().to_string())
            .collect();

        assert!(
            atlas_custos_postgres::migrations::custos_new().is_empty(),
            "this slice ships no Custos DDL yet"
        );
        assert_eq!(
            composed_names, historical_names,
            "the composed list must equal the historical list exactly while custos_new() is empty"
        );
    }
}
