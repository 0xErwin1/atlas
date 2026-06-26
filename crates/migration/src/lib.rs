#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod m20260612_000001_identity;
pub mod m20260612_000002_workspace_core;
pub mod m20260612_000003_documents;
pub mod m20260612_000004_boards_tasks;
pub mod m20260612_000005_permissions;
pub mod m20260613_000006_document_slug;
pub mod m20260613_000007_boards_tasks_e05;
pub mod m20260614_000008_task_reference_kind_target_check;
pub mod m20260614_000009_attachment_task_fk;
pub mod m20260618_000010_user_email;
pub mod m20260618_000011_user_ui_state;
pub mod m20260619_000012_task_parent;
pub mod m20260620_000013_tags;
pub mod m20260620_000014_saved_searches;
pub mod m20260621_000015_task_views;
pub mod m20260621_000016_column_color;
pub mod m20260621_000017_tag_color;
pub mod m20260622_000018_status_templates;
pub mod m20260622_000019_user_system_admin;
pub mod m20260623_000020_apikey_identity;
pub mod m20260623_000021_user_activation;
pub mod m20260623_000022_workspace_activity_index;
pub mod m20260624_000023_security_audit_log;
pub mod m20260624_000024_groups;
pub mod m20260626_000025_apikey_global;

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
            Box::new(m20260613_000006_document_slug::Migration),
            Box::new(m20260613_000007_boards_tasks_e05::Migration),
            Box::new(m20260614_000008_task_reference_kind_target_check::Migration),
            Box::new(m20260614_000009_attachment_task_fk::Migration),
            Box::new(m20260618_000010_user_email::Migration),
            Box::new(m20260618_000011_user_ui_state::Migration),
            Box::new(m20260619_000012_task_parent::Migration),
            Box::new(m20260620_000013_tags::Migration),
            Box::new(m20260620_000014_saved_searches::Migration),
            Box::new(m20260621_000015_task_views::Migration),
            Box::new(m20260621_000016_column_color::Migration),
            Box::new(m20260621_000017_tag_color::Migration),
            Box::new(m20260622_000018_status_templates::Migration),
            Box::new(m20260622_000019_user_system_admin::Migration),
            Box::new(m20260623_000020_apikey_identity::Migration),
            Box::new(m20260623_000021_user_activation::Migration),
            Box::new(m20260623_000022_workspace_activity_index::Migration),
            Box::new(m20260624_000023_security_audit_log::Migration),
            Box::new(m20260624_000024_groups::Migration),
            Box::new(m20260626_000025_apikey_global::Migration),
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
