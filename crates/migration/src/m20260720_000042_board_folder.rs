use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260720_000042_board_folder"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"ALTER TABLE boards ADD COLUMN folder_id UUID REFERENCES folders(id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX boards_workspace_folder_idx ON boards (workspace_id, folder_id)"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP INDEX IF EXISTS boards_workspace_folder_idx")
            .await?;
        conn.execute_unprepared("ALTER TABLE boards DROP COLUMN IF EXISTS folder_id")
            .await?;

        Ok(())
    }
}
