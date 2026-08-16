use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260816_000048_board_archive"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Distinct from `deleted_at`: an archived board is still readable and
        // still listed, it only refuses writes. A deleted one is gone from every
        // listing and waiting on the Trash purge.
        conn.execute_unprepared(r#"ALTER TABLE boards ADD COLUMN archived_at TIMESTAMPTZ"#)
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("ALTER TABLE boards DROP COLUMN IF EXISTS archived_at")
            .await?;

        Ok(())
    }
}
