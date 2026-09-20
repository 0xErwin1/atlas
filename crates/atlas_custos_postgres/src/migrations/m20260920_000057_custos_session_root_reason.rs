//! E4-S3C (`v2-e4-s3c-root-reason`): root becomes break-glass that must
//! justify itself — `custos.sessions.root_reason`, a nullable TEXT column
//! carrying the mandatory justification recorded at root login.
//!
//! Two back-fills close the standing-credential holes that would have made
//! the justification requirement avoidable, and both are one-way:
//!
//! 1. **Every existing session belonging to a root user is revoked.** A root
//!    session minted before this migration carries no stated reason (the
//!    column does not exist yet), so letting it survive would keep an
//!    unjustified break-glass credential alive indefinitely. The login
//!    handler now requires a non-empty reason for root, so a legitimate root
//!    can simply log in again and state one; a non-root session is untouched.
//!
//! 2. **Every existing personal API key whose creator is root is revoked,
//!    with the count logged.** A personal key acts as its user without any
//!    session, so it has no place to carry a reason: a standing root personal
//!    key would defeat "every root action carries a mandatory justification"
//!    entirely. Agent keys owned by root are deliberately kept — they act as
//!    the agent principal, are capability-scoped and capped at editor, and
//!    are not root credentials. The revoked-key count is reported via the
//!    `tracing` log (`root_reason.backfill` event) so operators see the
//!    credential-surface change in the startup output; the migration cannot
//!    fail the deploy over a data-hygiene report, so it is informational.
//!
//! `down()` only drops the column: the revocations are irreversible by
//! design — a revoked credential must be re-minted through the audited
//! creation paths, not resurrected by a rollback. `crates/migration` stays
//! byte-frozen (INV-MIGRATION-SCOPE): this migration is spliced into
//! `ComposedMigrator`/`ComposedTestMigrator` by `custos_new()`.

use sea_orm::{ConnectionTrait, FromQueryResult, Statement};
use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260920_000057_custos_session_root_reason"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("ALTER TABLE custos.sessions ADD COLUMN root_reason TEXT NULL")
            .await?;

        conn.execute_unprepared(
            "UPDATE custos.sessions SET revoked_at = now() \
             WHERE user_id IN (SELECT id FROM custos.users WHERE is_root) \
               AND revoked_at IS NULL",
        )
        .await?;

        let root_personal_keys = count_root_personal_keys(conn).await?;
        conn.execute_unprepared(
            "UPDATE custos.api_keys SET revoked_at = now() \
             WHERE created_by_user_id IN (SELECT id FROM custos.users WHERE is_root) \
               AND principal_id IN (SELECT id FROM custos.principals WHERE kind = 'user') \
               AND revoked_at IS NULL",
        )
        .await?;

        tracing::info!(
            target: "root_reason.backfill",
            event = "root_credentials_revoked",
            root_personal_keys_revoked = root_personal_keys,
            "revoked pre-migration root sessions and root personal API keys: \
             a root credential without a stated reason must not survive the \
             justification requirement"
        );

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("ALTER TABLE custos.sessions DROP COLUMN root_reason")
            .await?;

        Ok(())
    }
}

/// Counts the root-owned personal API keys the migration is about to revoke,
/// so the log line reports the exact credential-surface change.
async fn count_root_personal_keys<C: ConnectionTrait>(conn: &C) -> Result<u64, DbErr> {
    #[derive(Debug, FromQueryResult)]
    struct CountRow {
        count: i64,
    }

    let row = CountRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT count(*) AS count FROM custos.api_keys \
         WHERE created_by_user_id IN (SELECT id FROM custos.users WHERE is_root) \
           AND principal_id IN (SELECT id FROM custos.principals WHERE kind = 'user') \
           AND revoked_at IS NULL"
            .to_owned(),
    ))
    .one(conn)
    .await?
    .ok_or(DbErr::Custom("count query returned no row".to_string()))?;

    Ok(std::cmp::max(row.count, 0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(
            Migration.name(),
            "m20260920_000057_custos_session_root_reason"
        );
    }
}
