use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260702_000034_comments"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE comments (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                task_id UUID REFERENCES tasks(id) ON DELETE CASCADE,
                document_id UUID REFERENCES documents(id) ON DELETE CASCADE,
                body TEXT NOT NULL,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT comments_owner_check
                    CHECK (num_nonnulls(task_id, document_id) = 1),
                CONSTRAINT comments_actor_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX comments_task_id_idx
               ON comments (task_id, id ASC)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS comments CASCADE")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260702_000034_comments");
    }
}
