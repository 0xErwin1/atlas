//! E4-S1: introduces `custos.principals` additively, with no re-key —
//! `User` and `Agent` principals share one stable identity row carrying
//! `kind`, `display_name` and `deactivated_at`, while the outbound FKs on
//! `permission_grants`, `sessions`, `security_audit_log`, `group_members`
//! and `user_activation_tokens` keep pointing at `users`/`api_keys`
//! untouched.
//!
//! Back-fill, inside this one migration:
//! - one `user` principal per `custos.users` row (`id = users.id`,
//!   `display_name = users.display_name`, `deactivated_at =
//!   users.disabled_at`);
//! - one `agent` principal per `custos.api_keys` row with a fresh UUIDv7
//!   id (`display_name = api_keys.name`, `deactivated_at =
//!   api_keys.revoked_at`). The ids are generated application-side: PG17
//!   has no `uuidv7()` SQL function, and the workspace id convention is
//!   app-generated UUIDv7.
//!
//! Every V1 `ApiKeyType` — `agent|cli|bot|integration` — collapses to the
//! single `agent` kind: the principals spec defines exactly two kinds, and
//! every non-human credential is an agent. `PersonalApiKey` (a
//! user-attributed credential that owns no principal) is a later slice, so
//! every existing `api_keys` row gets an agent principal.
//!
//! `users.principal_id` and `api_keys.principal_id` are added nullable with
//! an FK to `custos.principals(id)`, and back-filled here (`users
//! .principal_id = users.id`; each key points at the agent principal created
//! for it). The `NOT NULL` constraint is deliberately NOT applied in this
//! slice: it lands in the follow-up slice together with every principal
//! mirror writer, so this migration stays purely additive and no existing
//! write path can violate a constraint it does not know about yet.
//!
//! Runs after `m20260906_000052_grant_principal_idx`, so the `custos`
//! schema already exists. `crates/migration` stays byte-frozen
//! (INV-MIGRATION-SCOPE): this migration is spliced into
//! `ComposedMigrator`/`ComposedTestMigrator` by `custos_new()`.

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260917_000053_custos_principals"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "CREATE TABLE custos.principals ( \
                 id             UUID PRIMARY KEY, \
                 kind           TEXT NOT NULL CHECK (kind IN ('user','agent')), \
                 display_name   TEXT NOT NULL, \
                 deactivated_at TIMESTAMPTZ NULL, \
                 created_at     TIMESTAMPTZ NOT NULL DEFAULT now(), \
                 updated_at     TIMESTAMPTZ NOT NULL DEFAULT now() \
             )",
        )
        .await?;

        conn.execute_unprepared(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at) \
             SELECT id, 'user', display_name, disabled_at FROM custos.users",
        )
        .await?;

        conn.execute_unprepared(
            "ALTER TABLE custos.users \
                 ADD COLUMN principal_id UUID REFERENCES custos.principals (id)",
        )
        .await?;
        conn.execute_unprepared(
            "ALTER TABLE custos.api_keys \
                 ADD COLUMN principal_id UUID REFERENCES custos.principals (id)",
        )
        .await?;

        conn.execute_unprepared("UPDATE custos.users SET principal_id = id")
            .await?;

        // Fresh UUIDv7 per key, generated application-side because PG17 has
        // no `uuidv7()` function (see module docs).
        let keys = conn
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT id, name, revoked_at FROM custos.api_keys".to_owned(),
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

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("ALTER TABLE custos.api_keys DROP COLUMN principal_id")
            .await?;
        conn.execute_unprepared("ALTER TABLE custos.users DROP COLUMN principal_id")
            .await?;
        conn.execute_unprepared("DROP TABLE custos.principals")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260917_000053_custos_principals");
    }
}
