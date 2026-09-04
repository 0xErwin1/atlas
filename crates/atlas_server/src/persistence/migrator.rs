//! Composes the historical migration block with Custos-owned migrations.
//!
//! `crates/migration` stays frozen (D5): it keeps every migration authored
//! before S3, unmodified, because sea-orm records applied migrations by name
//! in one `seaql_migrations` table and re-partitioning history by owner would
//! reorder what already applied to every existing database.
//!
//! `ComposedMigrator::migrations()` is `historical() ++ custos_new() ++
//! acta_new()`: the composed list always starts with every historical
//! migration name, in order, followed by whatever Custos owns, followed by
//! whatever Acta owns (S4 §D3).

use sea_orm_migration::prelude::{MigrationTrait, MigratorTrait};

pub struct ComposedMigrator;

impl MigratorTrait for ComposedMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations = migration::Migrator::migrations();
        migrations.extend(atlas_custos_postgres::migrations::custos_new());
        migrations.extend(atlas_acta_postgres::migrations::acta_new());
        migrations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composed_migrations_are_historical_then_custos_new_then_acta_new_in_order() {
        let historical_names: Vec<_> = migration::Migrator::migrations()
            .iter()
            .map(|m| m.name().to_string())
            .collect();
        let custos_names: Vec<_> = atlas_custos_postgres::migrations::custos_new()
            .iter()
            .map(|m| m.name().to_string())
            .collect();
        let acta_names: Vec<_> = atlas_acta_postgres::migrations::acta_new()
            .iter()
            .map(|m| m.name().to_string())
            .collect();
        let composed_names: Vec<_> = ComposedMigrator::migrations()
            .iter()
            .map(|m| m.name().to_string())
            .collect();

        let expected: Vec<_> = historical_names
            .iter()
            .cloned()
            .chain(custos_names.iter().cloned())
            .chain(acta_names.iter().cloned())
            .collect();

        assert_eq!(
            composed_names, expected,
            "the composed list must be exactly historical() ++ custos_new() ++ acta_new(), in order"
        );
    }

    /// T3.1: `acta_new()` must carry the idempotency-keys migration as one
    /// additional entry, without disturbing the historical/custos/acta
    /// three-segment shape the previous test already pins.
    #[test]
    fn acta_new_contains_the_idempotency_keys_migration() {
        let acta_names: Vec<_> = atlas_acta_postgres::migrations::acta_new()
            .iter()
            .map(|m| m.name().to_string())
            .collect();

        assert!(
            acta_names.contains(&"m20260906_000058_acta_platform_idempotency_keys".to_string()),
            "acta_new() must contain the idempotency-keys migration, got: {acta_names:?}"
        );
    }
}

#[cfg(test)]
mod binary_source_tests {
    /// Every binary that migrates a database must run the composed list:
    /// the historical `migration::Migrator` alone leaves the Custos and
    /// Acta schema moves unapplied on a live database while reporting
    /// "no pending migrations".
    #[test]
    fn every_binary_migrates_through_the_composed_migrator() {
        let sources = [
            include_str!("../main.rs"),
            include_str!("../bin/seed_dev.rs"),
        ];

        for source in sources {
            assert!(
                !source.contains("migration::Migrator"),
                "a binary still migrates through the historical migrator alone"
            );
            assert!(
                source.contains("ComposedMigrator::up("),
                "a binary does not migrate through ComposedMigrator"
            );
        }
    }
}
