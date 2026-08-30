//! O1: replaces `permission_grants`' four target columns
//! (`project_id`/`folder_id`/`document_id`/`board_id`) with one opaque
//! `resource_ref TEXT` column, encoded exactly as
//! `atlas_acta::permissions::resource_ref_codec::to_core` produces
//! (`acta::<kind>::<uuid>`, or `acta::workspace::<workspace_id>` when a grant
//! targets no specific resource). This also drops the nine Custos-outbound FK
//! constraints identified in design D1 — five on `permission_grants`, plus
//! `groups.workspace_id`, `api_keys.workspace_id`,
//! `security_audit_log.workspace_id`, and
//! `purge_operations.commit_audit_id -> security_audit_log(id)` — so no
//! Custos table (nor `purge_operations`, once `security_audit_log` is
//! Custos-owned) carries a foreign key out to a non-Custos table.
//!
//! `permission_grants_uq`'s live definition was verified against
//! `pg_indexes` before this migration was authored (R-e gate): it is
//! `(workspace_id, user_id, api_key_id, group_id, project_id, folder_id,
//! document_id, board_id) NULLS NOT DISTINCT` — `group_id` post-dates the
//! original 7-column DDL and is a member. Step 4 below replaces it with the
//! resource-ref-collapsed equivalent, `(workspace_id, user_id, api_key_id,
//! group_id, resource_ref) NULLS NOT DISTINCT`.
//!
//! Down migration note: rows deleted under the forward schema (after the
//! cascades this migration removes stop firing) cannot be reconstructed by
//! the down migration — it restores columns and data from `resource_ref` for
//! rows that still exist, not rows a since-removed cascade would have
//! deleted.
//!
//! Deployment contract: this is deliberately a single stop-the-world
//! migration, not an expand/contract pair. There is no schema state in which
//! the pre-change and post-change binaries can both serve traffic — an old
//! instance running against the migrated schema fails every grant-touching
//! query with `column ... does not exist`. Atlas deploys as a single
//! instance whose binary starts only after its migrator has run, so no such
//! overlap exists, and a binary rollback must pair with running this
//! migration's `down()`. If Atlas ever moves to rolling multi-instance
//! deploys, schema changes of this shape must switch to expand/contract
//! phases; do not reuse this migration as that precedent.

use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260830_000050_grant_resource_ref"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Step 1: add the new column, nullable first so the backfill can run.
        conn.execute_unprepared("ALTER TABLE permission_grants ADD COLUMN resource_ref TEXT")
            .await?;

        // Step 2: backfill, matching `resource_ref_codec::to_core` byte-for-byte
        // (pinned by the golden test in `grant_resource_ref_migration.rs`).
        conn.execute_unprepared(
            r#"
            UPDATE permission_grants
            SET resource_ref = CASE
                WHEN project_id IS NOT NULL THEN 'acta::project::' || project_id::text
                WHEN folder_id IS NOT NULL THEN 'acta::folder::' || folder_id::text
                WHEN document_id IS NOT NULL THEN 'acta::document::' || document_id::text
                WHEN board_id IS NOT NULL THEN 'acta::board::' || board_id::text
                ELSE 'acta::workspace::' || workspace_id::text
            END
            "#,
        )
        .await?;

        // Step 3: enforce NOT NULL now that every row has a value.
        conn.execute_unprepared(
            "ALTER TABLE permission_grants ALTER COLUMN resource_ref SET NOT NULL",
        )
        .await?;

        // Step 4: replace the target-column unique index with the resource-ref
        // equivalent, matching the verified live column list exactly.
        conn.execute_unprepared("DROP INDEX permission_grants_uq")
            .await?;
        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX permission_grants_uq
            ON permission_grants (workspace_id, user_id, api_key_id, group_id, resource_ref)
            NULLS NOT DISTINCT
            "#,
        )
        .await?;

        // Step 5: replace the four partial target indexes with one resource-ref index.
        conn.execute_unprepared("DROP INDEX permission_grants_project_idx")
            .await?;
        conn.execute_unprepared("DROP INDEX permission_grants_folder_idx")
            .await?;
        conn.execute_unprepared("DROP INDEX permission_grants_document_idx")
            .await?;
        conn.execute_unprepared("DROP INDEX permission_grants_board_idx")
            .await?;
        conn.execute_unprepared(
            "CREATE INDEX permission_grants_resource_idx ON permission_grants (workspace_id, resource_ref)",
        )
        .await?;

        // Step 6: the at-most-one-target CHECK no longer applies to a single
        // opaque resource_ref column.
        conn.execute_unprepared(
            "ALTER TABLE permission_grants DROP CONSTRAINT permission_grants_target_at_most_one",
        )
        .await?;

        // Step 7: drop the nine Custos-outbound FK constraints from D1.
        conn.execute_unprepared(
            "ALTER TABLE permission_grants DROP CONSTRAINT permission_grants_workspace_id_fkey",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants DROP CONSTRAINT permission_grants_project_id_fkey",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants DROP CONSTRAINT permission_grants_folder_id_fkey",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants DROP CONSTRAINT permission_grants_document_id_fkey",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants DROP CONSTRAINT permission_grants_board_id_fkey",
        )
        .await?;
        conn.execute_unprepared("ALTER TABLE groups DROP CONSTRAINT groups_workspace_id_fkey")
            .await?;
        conn.execute_unprepared("ALTER TABLE api_keys DROP CONSTRAINT api_keys_workspace_id_fkey")
            .await?;
        conn.execute_unprepared(
            "ALTER TABLE security_audit_log DROP CONSTRAINT security_audit_log_workspace_id_fkey",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE purge_operations DROP CONSTRAINT purge_operations_commit_audit_id_fkey",
        )
        .await?;

        // Step 8: drop the now-redundant target columns.
        conn.execute_unprepared("ALTER TABLE permission_grants DROP COLUMN project_id")
            .await?;
        conn.execute_unprepared("ALTER TABLE permission_grants DROP COLUMN folder_id")
            .await?;
        conn.execute_unprepared("ALTER TABLE permission_grants DROP COLUMN document_id")
            .await?;
        conn.execute_unprepared("ALTER TABLE permission_grants DROP COLUMN board_id")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Restore the four target columns and re-derive them from resource_ref.
        // A row whose resource_ref decodes to a non-workspace kind gets exactly
        // one of these columns populated; a workspace-scope grant leaves all
        // four NULL, matching the pre-migration shape. Rows a now-removed
        // cascade would have deleted while this migration was forward cannot be
        // restored — only currently-existing rows are backfilled.
        conn.execute_unprepared("ALTER TABLE permission_grants ADD COLUMN project_id UUID")
            .await?;
        conn.execute_unprepared("ALTER TABLE permission_grants ADD COLUMN folder_id UUID")
            .await?;
        conn.execute_unprepared("ALTER TABLE permission_grants ADD COLUMN document_id UUID")
            .await?;
        conn.execute_unprepared("ALTER TABLE permission_grants ADD COLUMN board_id UUID")
            .await?;

        conn.execute_unprepared(
            r#"
            UPDATE permission_grants
            SET project_id = CASE WHEN resource_ref LIKE 'acta::project::%'
                THEN substring(resource_ref FROM 16)::uuid ELSE NULL END,
                folder_id = CASE WHEN resource_ref LIKE 'acta::folder::%'
                THEN substring(resource_ref FROM 15)::uuid ELSE NULL END,
                document_id = CASE WHEN resource_ref LIKE 'acta::document::%'
                THEN substring(resource_ref FROM 17)::uuid ELSE NULL END,
                board_id = CASE WHEN resource_ref LIKE 'acta::board::%'
                THEN substring(resource_ref FROM 14)::uuid ELSE NULL END
            "#,
        )
        .await?;

        // A forward-schema delete has no FK to cascade through, so a
        // re-derived column above can be orphaned; drop those rows first.
        conn.execute_unprepared(
            "DELETE FROM permission_grants WHERE project_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM projects WHERE projects.id = permission_grants.project_id)",
        )
        .await?;
        conn.execute_unprepared(
            "DELETE FROM permission_grants WHERE folder_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM folders WHERE folders.id = permission_grants.folder_id)",
        )
        .await?;
        conn.execute_unprepared(
            "DELETE FROM permission_grants WHERE document_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM documents WHERE documents.id = permission_grants.document_id)",
        )
        .await?;
        conn.execute_unprepared(
            "DELETE FROM permission_grants WHERE board_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM boards WHERE boards.id = permission_grants.board_id)",
        )
        .await?;

        // The same applies to every other re-added constraint: rows whose
        // parent vanished while the forward schema was live would abort the
        // ALTER TABLE revalidation below. Each guard mirrors the restored
        // FK's own delete semantics: permission_grants/groups were ON DELETE
        // CASCADE (groups' own dependents, group_members and
        // permission_grants.group_id, are CASCADE too, so the delete chains
        // cleanly); security_audit_log was ON DELETE SET NULL. The two
        // blocking-style FKs cannot block retroactively on rollback, so the
        // nullable api_keys.workspace_id is nulled and the NOT NULL
        // purge_operations.commit_audit_id row is dropped — both orphan
        // shapes are unreachable today (workspaces are only soft-deleted and
        // nothing hard-deletes audit rows), so these are abort-prevention
        // guards, not expected data paths.
        conn.execute_unprepared(
            "DELETE FROM permission_grants \
             WHERE NOT EXISTS (SELECT 1 FROM workspaces WHERE workspaces.id = permission_grants.workspace_id)",
        )
        .await?;
        conn.execute_unprepared(
            "DELETE FROM groups \
             WHERE NOT EXISTS (SELECT 1 FROM workspaces WHERE workspaces.id = groups.workspace_id)",
        )
        .await?;
        conn.execute_unprepared(
            "UPDATE api_keys SET workspace_id = NULL WHERE workspace_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM workspaces WHERE workspaces.id = api_keys.workspace_id)",
        )
        .await?;
        conn.execute_unprepared(
            "UPDATE security_audit_log SET workspace_id = NULL WHERE workspace_id IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM workspaces WHERE workspaces.id = security_audit_log.workspace_id)",
        )
        .await?;
        conn.execute_unprepared(
            "DELETE FROM purge_operations \
             WHERE NOT EXISTS (SELECT 1 FROM security_audit_log WHERE security_audit_log.id = purge_operations.commit_audit_id)",
        )
        .await?;

        // Restore the nine FK constraints.
        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_workspace_id_fkey \
             FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_project_id_fkey \
             FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_folder_id_fkey \
             FOREIGN KEY (folder_id) REFERENCES folders(id) ON DELETE CASCADE",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_document_id_fkey \
             FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE CASCADE",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_board_id_fkey \
             FOREIGN KEY (board_id) REFERENCES boards(id) ON DELETE CASCADE",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE groups ADD CONSTRAINT groups_workspace_id_fkey \
             FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE api_keys ADD CONSTRAINT api_keys_workspace_id_fkey \
             FOREIGN KEY (workspace_id) REFERENCES workspaces(id)",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE security_audit_log ADD CONSTRAINT security_audit_log_workspace_id_fkey \
             FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE purge_operations ADD CONSTRAINT purge_operations_commit_audit_id_fkey \
             FOREIGN KEY (commit_audit_id) REFERENCES security_audit_log(id) ON DELETE RESTRICT",
        )
        .await?;

        // Restore the at-most-one-target CHECK.
        conn.execute_unprepared(
            "ALTER TABLE permission_grants ADD CONSTRAINT permission_grants_target_at_most_one \
             CHECK (num_nonnulls(project_id, folder_id, document_id, board_id) <= 1)",
        )
        .await?;

        // Restore the four partial target indexes; drop the resource-ref index.
        conn.execute_unprepared("DROP INDEX IF EXISTS permission_grants_resource_idx")
            .await?;
        conn.execute_unprepared(
            "CREATE INDEX permission_grants_project_idx ON permission_grants (project_id) \
             WHERE project_id IS NOT NULL",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE INDEX permission_grants_folder_idx ON permission_grants (folder_id) \
             WHERE folder_id IS NOT NULL",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE INDEX permission_grants_document_idx ON permission_grants (document_id) \
             WHERE document_id IS NOT NULL",
        )
        .await?;
        conn.execute_unprepared(
            "CREATE INDEX permission_grants_board_idx ON permission_grants (board_id) \
             WHERE board_id IS NOT NULL",
        )
        .await?;

        // Restore the original 8-column unique index (the verified live shape).
        conn.execute_unprepared("DROP INDEX permission_grants_uq")
            .await?;
        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX permission_grants_uq
            ON permission_grants (workspace_id, user_id, api_key_id, group_id, project_id, folder_id, document_id, board_id)
            NULLS NOT DISTINCT
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "ALTER TABLE permission_grants ALTER COLUMN resource_ref DROP NOT NULL",
        )
        .await?;
        conn.execute_unprepared("ALTER TABLE permission_grants DROP COLUMN resource_ref")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260830_000050_grant_resource_ref");
    }
}
