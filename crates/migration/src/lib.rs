#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod m20260612_000001_identity;
pub mod m20260612_000002_workspace_core;
pub mod m20260612_000003_documents;
pub mod m20260612_000004_boards_tasks;
pub mod m20260612_000005_permissions;

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260612_000001_identity::Migration),
            Box::new(m20260612_000002_workspace_core::Migration),
            Box::new(m20260612_000003_documents::Migration),
            Box::new(m20260612_000004_boards_tasks::Migration),
            Box::new(m20260612_000005_permissions::Migration),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_list_is_not_empty() {
        assert!(!Migrator::migrations().is_empty());
    }
}
