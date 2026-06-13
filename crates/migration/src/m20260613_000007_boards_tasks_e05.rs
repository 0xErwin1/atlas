use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260613_000007_boards_tasks_e05"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // -------------------------------------------------------------------
        // B2: Add created_by_api_key_id to boards / board_columns / tasks /
        // task_references and relax the user FK to nullable with XOR CHECK.
        // Existing rows are all user-created, so backfill is not needed.
        // -------------------------------------------------------------------

        conn.execute_unprepared(
            r#"
            ALTER TABLE boards
                ADD COLUMN created_by_api_key_id UUID REFERENCES api_keys(id),
                ALTER COLUMN created_by_user_id DROP NOT NULL,
                ADD CONSTRAINT boards_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE board_columns
                ADD COLUMN created_by_api_key_id UUID REFERENCES api_keys(id),
                ALTER COLUMN created_by_user_id DROP NOT NULL,
                ADD CONSTRAINT board_columns_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE tasks
                ADD COLUMN created_by_api_key_id UUID REFERENCES api_keys(id),
                ALTER COLUMN created_by_user_id DROP NOT NULL,
                ADD CONSTRAINT tasks_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE task_references
                ADD COLUMN created_by_api_key_id UUID REFERENCES api_keys(id),
                ALTER COLUMN created_by_user_id DROP NOT NULL,
                ADD CONSTRAINT task_references_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            "#,
        )
        .await?;

        // -------------------------------------------------------------------
        // Typed task columns: priority, due_date, estimate, labels TEXT[].
        // Keep properties JSONB for custom escape-hatch fields.
        // -------------------------------------------------------------------

        conn.execute_unprepared(
            r#"
            ALTER TABLE tasks
                ADD COLUMN priority TEXT
                    CONSTRAINT tasks_priority_check
                        CHECK (priority IN ('low', 'medium', 'high', 'urgent')),
                ADD COLUMN due_date TIMESTAMPTZ,
                ADD COLUMN estimate INTEGER
                    CONSTRAINT tasks_estimate_check CHECK (estimate >= 0),
                ADD COLUMN labels TEXT[] NOT NULL DEFAULT '{}'
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX tasks_labels_gin ON tasks USING gin (labels)"#,
        )
        .await?;

        // -------------------------------------------------------------------
        // task_assignees: principal XOR (user | api_key), unique per task.
        // -------------------------------------------------------------------

        conn.execute_unprepared(
            r#"
            CREATE TABLE task_assignees (
                task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                assignee_user_id UUID REFERENCES users(id),
                assignee_api_key_id UUID REFERENCES api_keys(id),
                assigned_by_user_id UUID REFERENCES users(id),
                assigned_by_api_key_id UUID REFERENCES api_keys(id),
                assigned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT task_assignees_principal_check
                    CHECK (num_nonnulls(assignee_user_id, assignee_api_key_id) = 1),
                CONSTRAINT task_assignees_actor_check
                    CHECK (num_nonnulls(assigned_by_user_id, assigned_by_api_key_id) = 1),
                UNIQUE NULLS NOT DISTINCT (task_id, assignee_user_id, assignee_api_key_id)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_assignees_task_idx ON task_assignees (task_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_assignees_user_idx ON task_assignees (assignee_user_id) WHERE assignee_user_id IS NOT NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_assignees_api_key_idx ON task_assignees (assignee_api_key_id) WHERE assignee_api_key_id IS NOT NULL"#,
        )
        .await?;

        // -------------------------------------------------------------------
        // task_checklist_items: ordered checklist per task.
        // promoted_task_id is set once and never changed (promotion is permanent).
        // -------------------------------------------------------------------

        conn.execute_unprepared(
            r#"
            CREATE TABLE task_checklist_items (
                id UUID PRIMARY KEY,
                task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                title TEXT NOT NULL,
                checked BOOLEAN NOT NULL DEFAULT false,
                position_key TEXT NOT NULL,
                promoted_task_id UUID REFERENCES tasks(id),
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT task_checklist_items_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_checklist_items_task_idx ON task_checklist_items (task_id, position_key) WHERE deleted_at IS NULL"#,
        )
        .await?;

        // -------------------------------------------------------------------
        // task_activity: append-only activity log per task.
        // kind is one of 12 verbs; payload is typed JSONB per verb.
        // actor is XOR (B2 pattern).
        // -------------------------------------------------------------------

        conn.execute_unprepared(
            r#"
            CREATE TABLE task_activity (
                id UUID PRIMARY KEY,
                task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                kind TEXT NOT NULL,
                payload JSONB NOT NULL,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT task_activity_kind_check CHECK (
                    kind IN (
                        'created', 'moved', 'assigned', 'unassigned',
                        'field_changed', 'reference_added', 'reference_removed',
                        'checklist_added', 'checklist_updated', 'checklist_removed',
                        'checklist_promoted', 'deleted'
                    )
                ),
                CONSTRAINT task_activity_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_activity_task_time_idx ON task_activity (task_id, created_at DESC, id DESC)"#,
        )
        .await?;

        // -------------------------------------------------------------------
        // Q5: Polymorphic document_links.
        // Steps:
        //   1. Add source_task_id (nullable FK).
        //   2. Drop NOT NULL on source_document_id (it becomes optional, XOR enforced by CHECK).
        //   3. Drop the old unnamed UNIQUE constraint
        //      (PostgreSQL auto-named document_links_source_document_id_target_title_key).
        //   4. Add XOR CHECK (exactly one source).
        //   5. Add a functional unique index on (coalesced source, target_title).
        // -------------------------------------------------------------------

        conn.execute_unprepared(
            r#"ALTER TABLE document_links ADD COLUMN source_task_id UUID REFERENCES tasks(id) ON DELETE CASCADE"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links ALTER COLUMN source_document_id DROP NOT NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links DROP CONSTRAINT IF EXISTS document_links_source_document_id_target_title_key"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE document_links
                ADD CONSTRAINT document_links_source_check
                    CHECK (num_nonnulls(source_document_id, source_task_id) = 1)
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX document_links_source_title_uidx
                ON document_links (
                    COALESCE(source_document_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    COALESCE(source_task_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    target_title
                )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX document_links_source_task_idx ON document_links (source_task_id) WHERE source_task_id IS NOT NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Undo document_links polymorphic changes.
        conn.execute_unprepared(
            r#"DROP INDEX IF EXISTS document_links_source_task_idx"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"DROP INDEX IF EXISTS document_links_source_title_uidx"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links DROP CONSTRAINT IF EXISTS document_links_source_check"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links ALTER COLUMN source_document_id SET NOT NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links DROP COLUMN IF EXISTS source_task_id"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links ADD CONSTRAINT document_links_source_document_id_target_title_key UNIQUE (source_document_id, target_title)"#,
        )
        .await?;

        // Drop new tables.
        conn.execute_unprepared("DROP TABLE IF EXISTS task_activity CASCADE")
            .await?;

        conn.execute_unprepared("DROP TABLE IF EXISTS task_checklist_items CASCADE")
            .await?;

        conn.execute_unprepared("DROP TABLE IF EXISTS task_assignees CASCADE")
            .await?;

        // Undo typed task columns.
        conn.execute_unprepared(
            r#"
            ALTER TABLE tasks
                DROP COLUMN IF EXISTS priority,
                DROP COLUMN IF EXISTS due_date,
                DROP COLUMN IF EXISTS estimate,
                DROP COLUMN IF EXISTS labels
            "#,
        )
        .await?;

        // Undo actor XOR changes (best-effort; data with api_key actors will be lost).
        conn.execute_unprepared(
            r#"
            ALTER TABLE task_references
                DROP CONSTRAINT IF EXISTS task_references_actor_check,
                DROP COLUMN IF EXISTS created_by_api_key_id,
                ALTER COLUMN created_by_user_id SET NOT NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE tasks
                DROP CONSTRAINT IF EXISTS tasks_actor_check,
                DROP COLUMN IF EXISTS created_by_api_key_id,
                ALTER COLUMN created_by_user_id SET NOT NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE board_columns
                DROP CONSTRAINT IF EXISTS board_columns_actor_check,
                DROP COLUMN IF EXISTS created_by_api_key_id,
                ALTER COLUMN created_by_user_id SET NOT NULL
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE boards
                DROP CONSTRAINT IF EXISTS boards_actor_check,
                DROP COLUMN IF EXISTS created_by_api_key_id,
                ALTER COLUMN created_by_user_id SET NOT NULL
            "#,
        )
        .await?;

        Ok(())
    }
}
