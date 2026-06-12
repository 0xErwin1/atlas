use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260612_000004_boards_tasks"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE boards (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                project_id UUID NOT NULL REFERENCES projects(id),
                name TEXT NOT NULL,
                created_by_user_id UUID REFERENCES users(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX boards_workspace_project_idx ON boards (workspace_id, project_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE board_columns (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                board_id UUID NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                position_key TEXT NOT NULL,
                created_by_user_id UUID REFERENCES users(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX board_columns_board_idx ON board_columns (board_id, position_key)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE tasks (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                project_id UUID NOT NULL REFERENCES projects(id),
                board_id UUID NOT NULL REFERENCES boards(id),
                column_id UUID NOT NULL REFERENCES board_columns(id),
                readable_id TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                properties JSONB,
                position_key TEXT NOT NULL,
                search_vector TSVECTOR GENERATED ALWAYS AS (
                    setweight(to_tsvector('simple', coalesce(title, '')), 'A') ||
                    setweight(to_tsvector('simple', coalesce(description, '')), 'B')
                ) STORED,
                created_by_user_id UUID REFERENCES users(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT tasks_readable_id_format CHECK (readable_id ~ '^[A-Z][A-Z0-9]*-[0-9]+$')
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX tasks_project_readable_id_uidx
               ON tasks (project_id, readable_id)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX tasks_column_position_idx ON tasks (column_id, position_key)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX tasks_search_vector_gin ON tasks USING gin (search_vector)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX tasks_properties_gin ON tasks USING gin (properties jsonb_path_ops)
               WHERE properties IS NOT NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE task_references (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                source_task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE RESTRICT,
                kind TEXT NOT NULL,
                target_task_id UUID REFERENCES tasks(id) ON DELETE CASCADE,
                target_document_id UUID REFERENCES documents(id) ON DELETE CASCADE,
                created_by_user_id UUID REFERENCES users(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT task_references_kind_check
                    CHECK (kind IN ('relates', 'blocks', 'parent', 'spec')),
                CONSTRAINT task_references_target_check
                    CHECK (num_nonnulls(target_task_id, target_document_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX task_references_dedup_uidx
               ON task_references (source_task_id, kind, target_task_id, target_document_id)
               NULLS NOT DISTINCT"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_references_source_idx ON task_references (source_task_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_references_target_task_idx ON task_references (target_task_id)"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS task_references CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS tasks CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS board_columns CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS boards CASCADE")
            .await?;

        Ok(())
    }
}
