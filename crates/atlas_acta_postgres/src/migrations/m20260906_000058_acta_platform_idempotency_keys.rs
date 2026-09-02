//! S3 PR3: `platform.idempotency_keys` — persistence for the
//! `Idempotency-Key` HTTP mechanism (design §D2/D3/D4/D5/D7/D9).
//!
//! `platform.idempotency_keys` follows the exact precedent
//! `m20260831_000052_acta_platform_ui_state` established for
//! `platform.ui_state`: state owned by no product component (neither Custos
//! nor Acta), landed inside `acta_new()` anyway because that composition
//! slot already carries platform-schema migrations — no new `platform_new()`
//! slot is invented (D9).
//!
//! The unique index on `(principal_id, method, path, key)` backs D2's
//! insert-then-branch concurrency control: two concurrent inserts for the
//! same scope race on this constraint, and the loser reads the pre-existing
//! row instead of blocking on a lock. The composite index on
//! `(principal_id, expires_at)` backs the bounded opportunistic cleanup
//! delete, the only query that filters on `expires_at` (always scoped to a
//! `principal_id`).
//!
//! `generation` is a store-minted occupancy token (`Uuid::now_v7()`,
//! independent of `created_at`/wall-clock `now`): [`complete`] guards its
//! update on `id` AND `generation` matching, so a reclaim rewriting the row
//! in place always invalidates a stale claimant even if the caller-supplied
//! `now` is reused or non-monotonic. `created_at` remains a timestamp for
//! in-flight staleness only.

use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260906_000058_acta_platform_idempotency_keys"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("CREATE SCHEMA IF NOT EXISTS platform")
            .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS platform.idempotency_keys (
                id                    UUID PRIMARY KEY,
                generation            UUID NOT NULL,
                principal_id          UUID NOT NULL,
                method                TEXT NOT NULL,
                path                  TEXT NOT NULL,
                key                   TEXT NOT NULL,
                request_fingerprint   BYTEA NOT NULL,
                state                 TEXT NOT NULL
                                      CHECK (state IN ('in_flight', 'completed')),
                response_status       SMALLINT NULL,
                response_body         BYTEA NULL,
                response_headers      JSONB NULL,
                created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
                completed_at          TIMESTAMPTZ NULL,
                expires_at            TIMESTAMPTZ NOT NULL
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS idempotency_keys_scope_key_idx \
             ON platform.idempotency_keys (principal_id, method, path, key)",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idempotency_keys_principal_expires_at_idx \
             ON platform.idempotency_keys (principal_id, expires_at)",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS platform.idempotency_keys CASCADE")
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
            "m20260906_000058_acta_platform_idempotency_keys"
        );
    }
}
