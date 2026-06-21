use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260621_000015_task_views"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE task_views (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                name TEXT NOT NULL,
                filters JSONB NOT NULL DEFAULT '{}',
                owner_user_id UUID REFERENCES users(id),
                owner_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT task_views_owner_check
                    CHECK (num_nonnulls(owner_user_id, owner_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        // Two partial unique indexes are required here instead of one composite index because
        // the uniqueness constraint spans an XOR pair: (workspace_id, owner, lower(name)) among
        // live rows, where owner is either owner_user_id or owner_api_key_id but never both.
        // A single composite unique index cannot express XOR uniqueness: when one side is NULL,
        // Postgres treats the NULL as "not equal to anything" and two rows with different non-null
        // owners but the same name and workspace would not conflict. Each partial index covers
        // exactly one owner type, making the pair together express the full constraint.
        // Do NOT collapse these two indexes into one.
        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX task_views_ws_owner_user_name_idx
               ON task_views (workspace_id, owner_user_id, lower(name))
               WHERE deleted_at IS NULL AND owner_user_id IS NOT NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX task_views_ws_owner_key_name_idx
               ON task_views (workspace_id, owner_api_key_id, lower(name))
               WHERE deleted_at IS NULL AND owner_api_key_id IS NOT NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_views_owner_user_idx
               ON task_views (workspace_id, owner_user_id) WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX task_views_owner_key_idx
               ON task_views (workspace_id, owner_api_key_id) WHERE deleted_at IS NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS task_views CASCADE")
            .await?;

        Ok(())
    }
}
