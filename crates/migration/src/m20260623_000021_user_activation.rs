use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260623_000021_user_activation"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Allow password_hash to be NULL so a pending (uninvited) account can
        // exist without a credential. Back-filling keeps existing users active.
        conn.execute_unprepared("ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL")
            .await?;

        conn.execute_unprepared("ALTER TABLE users ADD COLUMN activated_at TIMESTAMPTZ")
            .await?;

        // Back-fill: every pre-existing user is treated as already activated.
        // Root and all existing users must continue to be able to log in after
        // this migration; setting activated_at = created_at achieves that without
        // requiring a manual step.
        conn.execute_unprepared(
            "UPDATE users SET activated_at = created_at WHERE activated_at IS NULL",
        )
        .await?;

        // Token table mirrors the sessions / api_keys pattern: one row per
        // single-use activation link, hashed at rest, CASCADE on user delete.
        conn.execute_unprepared(
            r#"
            CREATE TABLE user_activation_tokens (
                id          UUID PRIMARY KEY,
                user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash  TEXT NOT NULL UNIQUE,
                expires_at  TIMESTAMPTZ NOT NULL,
                consumed_at TIMESTAMPTZ,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX user_activation_tokens_user_idx ON user_activation_tokens (user_id)",
        )
        .await?;

        Ok(())
    }

    /// Reverses the migration.
    ///
    /// `ALTER TABLE users ALTER COLUMN password_hash SET NOT NULL` will fail if
    /// any pending (password_hash = NULL) rows exist. This is acceptable: the
    /// down migration is rarely run in production with pending accounts outstanding.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP INDEX IF EXISTS user_activation_tokens_user_idx")
            .await?;

        conn.execute_unprepared("DROP TABLE IF EXISTS user_activation_tokens")
            .await?;

        conn.execute_unprepared("ALTER TABLE users DROP COLUMN IF EXISTS activated_at")
            .await?;

        conn.execute_unprepared("ALTER TABLE users ALTER COLUMN password_hash SET NOT NULL")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260623_000021_user_activation");
    }
}
