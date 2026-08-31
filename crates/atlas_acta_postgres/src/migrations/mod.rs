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
mod m20260901_000053_acta_identity_workspaces_set_schema;
mod m20260902_000054_acta_documents_set_schema;
mod m20260903_000055_acta_boards_tasks_set_schema;
mod m20260904_000056_acta_comments_events_tags_set_schema;
mod m20260905_000057_acta_search_attachments_lifecycle_set_schema;

use sea_orm_migration::prelude::MigrationTrait;

/// Migrations owned by Acta, applied after `custos_new()`.
pub fn acta_new() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m20260831_000052_acta_platform_ui_state::Migration),
        Box::new(m20260901_000053_acta_identity_workspaces_set_schema::Migration),
        Box::new(m20260902_000054_acta_documents_set_schema::Migration),
        Box::new(m20260903_000055_acta_boards_tasks_set_schema::Migration),
        Box::new(m20260904_000056_acta_comments_events_tags_set_schema::Migration),
        Box::new(m20260905_000057_acta_search_attachments_lifecycle_set_schema::Migration),
    ]
}
