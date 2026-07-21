use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260721_000043_recoverable_deletion"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE purge_operations (
                    id UUID PRIMARY KEY,
                    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE RESTRICT,
                    target_kind TEXT NOT NULL CHECK (target_kind IN ('project', 'folder', 'document', 'comment', 'attachment')),
                    target_id UUID NOT NULL,
                    original_actor_user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
                    commit_audit_id UUID NOT NULL REFERENCES security_audit_log(id) ON DELETE RESTRICT,
                    status TEXT NOT NULL CHECK (status IN ('db_committed', 'cleanup_pending', 'cleanup_failed', 'complete')),
                    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
                    last_action TEXT NOT NULL CHECK (
                        (status = 'db_committed' AND last_action = 'resource.purge_committed') OR
                        (status = 'cleanup_pending' AND last_action = 'resource.purge_cleanup_pending') OR
                        (status = 'cleanup_failed' AND last_action = 'resource.purge_cleanup_failed') OR
                        (status = 'complete' AND last_action = 'resource.purge_completed')
                    ),
                    last_executor_type TEXT NOT NULL CHECK (last_executor_type IN ('user', 'system')),
                    last_executor_id UUID,
                    last_error TEXT,
                    last_attempt_at TIMESTAMPTZ,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                    UNIQUE (workspace_id, target_kind, target_id)
                );
                CREATE INDEX purge_operations_workspace_deleted_idx
                    ON purge_operations (workspace_id, created_at DESC, id DESC);

                CREATE TABLE purge_operation_digests (
                    operation_id UUID NOT NULL REFERENCES purge_operations(id) ON DELETE RESTRICT,
                    digest TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('db_committed', 'cleanup_pending', 'cleanup_failed', 'complete')),
                    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
                    last_error TEXT,
                    last_attempt_at TIMESTAMPTZ,
                    PRIMARY KEY (operation_id, digest)
                );

                CREATE INDEX projects_workspace_deleted_idx
                    ON projects (workspace_id, deleted_at DESC, id DESC)
                    WHERE deleted_at IS NOT NULL;
                CREATE INDEX folders_workspace_deleted_idx
                    ON folders (workspace_id, deleted_at DESC, id DESC)
                    WHERE deleted_at IS NOT NULL;
                CREATE INDEX documents_workspace_deleted_idx
                    ON documents (workspace_id, deleted_at DESC, id DESC)
                    WHERE deleted_at IS NOT NULL;
                CREATE INDEX comments_workspace_deleted_idx
                    ON comments (workspace_id, deleted_at DESC, id DESC)
                    WHERE deleted_at IS NOT NULL;
                CREATE INDEX attachments_workspace_deleted_idx
                    ON attachments (workspace_id, deleted_at DESC, id DESC)
                    WHERE deleted_at IS NOT NULL;
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DO $$
                BEGIN
                    IF EXISTS (SELECT 1 FROM purge_operations)
                       OR EXISTS (SELECT 1 FROM purge_operation_digests) THEN
                        RAISE EXCEPTION 'cannot roll back recoverable deletion while purge operations exist';
                    END IF;
                END $$;

                DROP INDEX IF EXISTS attachments_workspace_deleted_idx;
                DROP INDEX IF EXISTS comments_workspace_deleted_idx;
                DROP INDEX IF EXISTS documents_workspace_deleted_idx;
                DROP INDEX IF EXISTS folders_workspace_deleted_idx;
                DROP INDEX IF EXISTS projects_workspace_deleted_idx;
                DROP TABLE purge_operation_digests;
                DROP TABLE purge_operations;
                "#,
            )
            .await
            .map(|_| ())
    }
}
