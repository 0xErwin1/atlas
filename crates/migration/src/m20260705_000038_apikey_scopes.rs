use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260705_000038_apikey_scopes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "ALTER TABLE api_keys ADD COLUMN scopes TEXT[] NOT NULL DEFAULT '{}'",
        )
        .await?;

        // Grandfather every existing key (including revoked ones, harmless) to the
        // full catalog, so behavior is identical before and after this migration:
        // the capability gate this column enables does not exist yet for any key
        // created before scopes were selectable.
        conn.execute_unprepared(
            "UPDATE api_keys SET scopes = ARRAY[
                'tasks:read', 'tasks:create', 'tasks:update', 'tasks:delete',
                'docs:read', 'docs:create', 'docs:update', 'docs:delete',
                'boards:read', 'boards:create', 'boards:update', 'boards:delete',
                'folders:read', 'folders:create', 'folders:update', 'folders:delete',
                'projects:read', 'projects:create', 'projects:update', 'projects:delete'
            ]",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("ALTER TABLE api_keys DROP COLUMN IF EXISTS scopes")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260705_000038_apikey_scopes");
    }
}
