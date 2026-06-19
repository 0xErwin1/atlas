use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260619_000012_task_parent"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"ALTER TABLE tasks
               ADD COLUMN parent_task_id UUID NULL
               REFERENCES tasks(id) ON DELETE CASCADE"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX tasks_parent_idx
               ON tasks (workspace_id, parent_task_id)
               WHERE parent_task_id IS NOT NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(r#"DROP INDEX IF EXISTS tasks_parent_idx"#)
            .await?;
        conn.execute_unprepared(r#"ALTER TABLE tasks DROP COLUMN IF EXISTS parent_task_id"#)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260619_000012_task_parent");
    }
}
