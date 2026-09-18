//! E4-S1 PR2: applies `NOT NULL` to `custos.users.principal_id` and
//! `custos.api_keys.principal_id`, closing the mirror introduction started
//! by `m20260917_000053_custos_principals`.
//!
//! This is a separate migration from `000053` because of the nullable
//! window: `000053` added both columns nullable and back-filled only the
//! rows that existed at that moment, and it deliberately touched no writer
//! (no existing write path could violate a constraint it does not know
//! about yet). Between `000053` and this migration, the running app could
//! therefore create users and api keys whose `principal_id` stayed NULL.
//! A bare `SET NOT NULL` passes on fresh databases and fails on exactly
//! those rows, so the back-fill below must run in the same migration that
//! applies the constraint:
//!
//! - users: one `user` principal per still-unlinked row (`id = users.id`,
//!   `display_name = users.display_name`, `deactivated_at =
//!   users.disabled_at`), then `users.principal_id = users.id`;
//! - api keys: one `agent` principal per still-unlinked row with a fresh
//!   application-side UUIDv7 (`display_name = api_keys.name`,
//!   `deactivated_at = api_keys.revoked_at`), then the key is linked to it.
//!   The ids are generated application-side: PG17 has no `uuidv7()` SQL
//!   function, and a fresh id per row cannot be correlated back in pure
//!   SQL (same row-loop pattern as `000053`).
//!
//! `down()` only drops the two `NOT NULL` constraints: the columns and
//! `custos.principals` itself are owned by `000053`.
//!
//! Runs after `m20260917_000053_custos_principals`, so the mirror table and
//! both nullable columns already exist. `crates/migration` stays
//! byte-frozen (INV-MIGRATION-SCOPE): this migration is spliced into
//! `ComposedMigrator`/`ComposedTestMigrator` by `custos_new()`.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260918_000054_custos_principals_not_null"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Back-fill the nullable window before constraining: rows created
        // after `000053` applied may have no principal yet.
        conn.execute_unprepared(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at) \
             SELECT id, 'user', display_name, disabled_at FROM custos.users \
             WHERE principal_id IS NULL",
        )
        .await?;

        conn.execute_unprepared(
            "UPDATE custos.users SET principal_id = id WHERE principal_id IS NULL",
        )
        .await?;

        // Fresh UUIDv7 per unlinked key, generated application-side because
        // PG17 has no `uuidv7()` function and the id cannot be correlated
        // back in pure SQL (see module docs).
        let keys = conn
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT id, name, revoked_at FROM custos.api_keys \
                 WHERE principal_id IS NULL"
                    .to_owned(),
            ))
            .await?;

        for row in &keys {
            let key_id: uuid::Uuid = row.try_get("", "id")?;
            let name: String = row.try_get("", "name")?;
            let revoked_at: Option<chrono::DateTime<chrono::Utc>> =
                row.try_get("", "revoked_at")?;
            let principal_id = uuid::Uuid::now_v7();

            conn.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO custos.principals (id, kind, display_name, deactivated_at) \
                 VALUES ($1, 'agent', $2, $3)",
                [principal_id.into(), name.into(), revoked_at.into()],
            ))
            .await?;

            conn.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE custos.api_keys SET principal_id = $1 WHERE id = $2",
                [principal_id.into(), key_id.into()],
            ))
            .await?;
        }

        conn.execute_unprepared("ALTER TABLE custos.users ALTER COLUMN principal_id SET NOT NULL")
            .await?;
        conn.execute_unprepared(
            "ALTER TABLE custos.api_keys ALTER COLUMN principal_id SET NOT NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("ALTER TABLE custos.users ALTER COLUMN principal_id DROP NOT NULL")
            .await?;
        conn.execute_unprepared(
            "ALTER TABLE custos.api_keys ALTER COLUMN principal_id DROP NOT NULL",
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(
            Migration.name(),
            "m20260918_000054_custos_principals_not_null"
        );
    }
}
