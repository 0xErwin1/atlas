use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260620_000013_tags"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE tags (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                name TEXT NOT NULL,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT tags_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX tags_ws_lower_name_idx
               ON tags (workspace_id, lower(name))
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX tags_workspace_idx
               ON tags (workspace_id)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS tags CASCADE")
            .await?;

        Ok(())
    }
}
