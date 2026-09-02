//! `platform.idempotency_keys` store (design §D2/D3/D4/D5/D7/D9).
//!
//! This is the persistence half of the `Idempotency-Key` mechanism only — no
//! middleware, no route wiring. PR4 builds the `axum` middleware on top of
//! [`PgIdempotencyRepo`].
//!
//! The concurrency control (D2) is a unique constraint on
//! `(principal_id, method, path, key)`: [`PgIdempotencyRepo::insert_in_flight`]
//! attempts an `INSERT ... ON CONFLICT DO NOTHING`, then branches on a
//! follow-up `SELECT` when the insert loses the race. No
//! `pg_advisory_xact_lock`, no `SELECT ... FOR UPDATE` — both were rejected
//! in the design because they require a transaction spanning the entire
//! handler execution, which no repo in this codebase does today.
//!
//! Every timestamp-sensitive decision (in-flight staleness, response TTL,
//! expired-row cleanup) takes `now` as an explicit parameter rather than
//! reading the wall clock internally, so tests can prove staleness/TTL
//! behavior without sleeping.

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};
use uuid::Uuid;

use atlas_core::error::DomainError;
use atlas_postgres::db_err;

/// The fixed, non-configurable in-flight staleness window (D2). A row left
/// `in_flight` past this window is treated as abandoned (a crashed request)
/// and reclaimed by the next `insert_in_flight` call for the same scope.
pub const IN_FLIGHT_TTL: Duration = Duration::seconds(30);

/// Bounded opportunistic cleanup batch size (ORCHESTRATOR DECISION,
/// 2026-09-02, T3.15): every `insert_in_flight` call additionally deletes up
/// to this many expired rows scoped to the same `principal_id`, piggybacking
/// on a write path that already touches the table. This bounds unbounded
/// growth from keys that are never looked up again, with zero
/// scheduled-job machinery — delete-on-read (D7) remains the fast path for
/// rows that *are* looked up.
pub const CLEANUP_BATCH_LIMIT: i64 = 100;

/// Bound on the `branch_on_existing <-> reclaim` recursion (F1/F3 defense in
/// depth). Under real, distinct clock readings each successful reclaim mints
/// a fresh occupancy generation, so a losing racer's next round always sees
/// a non-stale row and returns `InFlight` within one or two rounds. This cap
/// turns any future boundary/predicate mismatch, or a pathological
/// thundering herd, into a safe `InFlight` answer instead of an unbounded
/// loop.
const MAX_RECLAIM_ROUNDS: u32 = 3;

/// The dedup scope (D3): concrete path, never a path template, to make
/// cross-tenant replay leakage structurally impossible.
#[derive(Debug, Clone)]
pub struct IdempotencyScope {
    pub principal_id: Uuid,
    pub method: String,
    pub path: String,
    pub key: String,
}

/// A stored, completed response (D5): status, verbatim body, and the header
/// allowlist only — never a full-header replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    pub status: i16,
    pub body: Vec<u8>,
    pub headers: Option<serde_json::Value>,
}

/// The outcome of [`PgIdempotencyRepo::insert_in_flight`], mapping exactly
/// onto D2's branch table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// No row existed for this scope, or an existing `in_flight`/expired
    /// `completed` row was reclaimed. The caller runs the handler and later
    /// calls [`PgIdempotencyRepo::complete`] with this `id` and
    /// `generation` — a store-minted occupancy token, independent of any
    /// caller-supplied `now` — which guards against a handler that outlives
    /// [`IN_FLIGHT_TTL`] stamping a later reclaimer's row.
    Fresh { id: Uuid, generation: Uuid },
    /// A non-stale `in_flight` row already exists for this scope (D2: 409,
    /// regardless of fingerprint — the concurrent-duplicate branch never
    /// compares fingerprints).
    InFlight,
    /// A `completed`, non-expired row exists with a matching fingerprint.
    Replay(StoredResponse),
    /// A `completed`, non-expired row exists with a different fingerprint
    /// (D1: renders as `ApiError::IdempotencyKeyConflict` upstream).
    Mismatch { existing_fingerprint: Vec<u8> },
}

/// The outcome of [`PgIdempotencyRepo::complete`]'s generation-guarded
/// update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteOutcome {
    /// The row was still `in_flight` at the claimed `generation`; the
    /// response was stored for replay.
    Stored,
    /// No row matched `id` and `generation`: reclaimed or deleted by a
    /// later caller, which now owns the row. Never treat this response as
    /// authoritative.
    Superseded,
}

#[derive(Debug, FromQueryResult)]
struct ExistingRow {
    state: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    request_fingerprint: Vec<u8>,
    response_status: Option<i16>,
    response_body: Option<Vec<u8>>,
    response_headers: Option<serde_json::Value>,
}

pub struct PgIdempotencyRepo {
    pub conn: DatabaseConnection,
}

impl PgIdempotencyRepo {
    /// D2's insert-then-branch entry point. Also performs the bounded
    /// opportunistic cleanup (T3.15) for `scope.principal_id` before
    /// attempting the insert, so a principal's abandoned rows shrink over
    /// successive calls without a scheduled job. Cleanup is best-effort: a
    /// failure there must never block the request the caller actually
    /// wants (D2/D7 do not depend on it running on every call).
    ///
    /// `retention` is the config-driven response TTL (D7, default 24h);
    /// `now` is the caller-supplied clock reading.
    pub async fn insert_in_flight(
        &self,
        scope: &IdempotencyScope,
        fingerprint: &[u8],
        now: DateTime<Utc>,
        retention: Duration,
    ) -> Result<InsertOutcome, DomainError> {
        if let Err(err) = self
            .delete_expired_for_principal(scope.principal_id, now)
            .await
        {
            tracing::warn!(
                ?err,
                principal_id = %scope.principal_id,
                "opportunistic idempotency cleanup failed; continuing"
            );
        }

        let id = Uuid::now_v7();
        let generation = Uuid::now_v7();
        let expires_at = now + retention;

        if self
            .try_insert(id, generation, scope, fingerprint, now, expires_at)
            .await?
        {
            return Ok(InsertOutcome::Fresh { id, generation });
        }

        self.branch_on_existing(scope, fingerprint, now, retention, 0)
            .await
    }

    /// The raw `INSERT ... ON CONFLICT DO NOTHING` attempt shared by
    /// [`Self::insert_in_flight`] and the missing-row retry in
    /// [`Self::reclaim`]. Returns `true` when this call's row won.
    async fn try_insert(
        &self,
        id: Uuid,
        generation: Uuid,
        scope: &IdempotencyScope,
        fingerprint: &[u8],
        now: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<bool, DomainError> {
        let insert = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                INSERT INTO platform.idempotency_keys
                    (id, generation, principal_id, method, path, key, request_fingerprint,
                     state, created_at, expires_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, 'in_flight', $8, $9)
                ON CONFLICT (principal_id, method, path, key) DO NOTHING
                "#,
                [
                    id.into(),
                    generation.into(),
                    scope.principal_id.into(),
                    scope.method.clone().into(),
                    scope.path.clone().into(),
                    scope.key.clone().into(),
                    fingerprint.to_vec().into(),
                    now.into(),
                    expires_at.into(),
                ],
            ))
            .await
            .map_err(db_err)?;

        Ok(insert.rows_affected() == 1)
    }

    /// Marks an in-flight row `completed`, storing the response for replay
    /// (D5) and extending `expires_at` to `now + retention` from completion
    /// time (D7). `generation` must be the store-minted token from the
    /// [`InsertOutcome::Fresh`] that started this handler; a mismatch means
    /// a later caller already reclaimed this row. `generation` is
    /// independent of `now`/`created_at`, so it cannot collide even when
    /// the caller-supplied clock is reused or non-monotonic across calls.
    #[allow(clippy::too_many_arguments)]
    pub async fn complete(
        &self,
        id: Uuid,
        generation: Uuid,
        status: i16,
        body: &[u8],
        headers: Option<serde_json::Value>,
        now: DateTime<Utc>,
        retention: Duration,
    ) -> Result<CompleteOutcome, DomainError> {
        let expires_at = now + retention;

        let update = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                UPDATE platform.idempotency_keys
                SET state = 'completed',
                    response_status = $1,
                    response_body = $2,
                    response_headers = $3,
                    completed_at = $4,
                    expires_at = $5
                WHERE id = $6 AND state = 'in_flight' AND generation = $7
                "#,
                [
                    status.into(),
                    body.to_vec().into(),
                    headers.into(),
                    now.into(),
                    expires_at.into(),
                    id.into(),
                    generation.into(),
                ],
            ))
            .await
            .map_err(db_err)?;

        if update.rows_affected() == 1 {
            Ok(CompleteOutcome::Stored)
        } else {
            Ok(CompleteOutcome::Superseded)
        }
    }

    /// Read-only lookup applying lazy expiry (D7): an expired `completed`
    /// row is treated as if it did not exist, without deleting it (the
    /// bounded cleanup in `insert_in_flight` reclaims it on the next write).
    pub async fn lookup(
        &self,
        scope: &IdempotencyScope,
        now: DateTime<Utc>,
    ) -> Result<Option<StoredResponse>, DomainError> {
        let row = self.select_existing(scope).await?;

        match row {
            Some(row) if row.state == "completed" && row.expires_at > now => {
                Ok(Some(StoredResponse {
                    status: row.response_status.unwrap_or_default(),
                    body: row.response_body.unwrap_or_default(),
                    headers: row.response_headers,
                }))
            }
            _ => Ok(None),
        }
    }

    /// T3.15: bounded opportunistic cleanup. Deletes up to
    /// [`CLEANUP_BATCH_LIMIT`] expired rows scoped to `principal_id`. Postgres
    /// `DELETE` has no `LIMIT` clause, so the bound is expressed via a
    /// subquery selecting the candidate ids first.
    pub async fn delete_expired_for_principal(
        &self,
        principal_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<u64, DomainError> {
        let result = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                DELETE FROM platform.idempotency_keys
                WHERE id IN (
                    SELECT id FROM platform.idempotency_keys
                    WHERE principal_id = $1 AND expires_at < $2
                    LIMIT $3
                )
                "#,
                [principal_id.into(), now.into(), CLEANUP_BATCH_LIMIT.into()],
            ))
            .await
            .map_err(db_err)?;

        Ok(result.rows_affected())
    }

    async fn select_existing(
        &self,
        scope: &IdempotencyScope,
    ) -> Result<Option<ExistingRow>, DomainError> {
        ExistingRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            SELECT state, created_at, expires_at, request_fingerprint,
                   response_status, response_body, response_headers
            FROM platform.idempotency_keys
            WHERE principal_id = $1 AND method = $2 AND path = $3 AND key = $4
            "#,
            [
                scope.principal_id.into(),
                scope.method.clone().into(),
                scope.path.clone().into(),
                scope.key.clone().into(),
            ],
        ))
        .one(&self.conn)
        .await
        .map_err(db_err)
    }

    /// Branches on the row the losing `INSERT ... ON CONFLICT DO NOTHING`
    /// found, per D2's table. Called only after the insert attempt lost the
    /// unique-constraint race, so the row is guaranteed to exist at this
    /// point (a concurrent delete between the two statements is the one
    /// window this cannot close without a lock D2 explicitly rejects; a
    /// missing row here re-attempts as fresh, which is safe either way).
    ///
    /// The `age >= IN_FLIGHT_TTL`/`expires_at <= now` staleness checks below
    /// are deliberately inclusive of the exact boundary, and [`Self::reclaim`]'s
    /// `UPDATE ... WHERE` uses the same inclusive `<=` — the two must agree
    /// exactly, otherwise a boundary tick routes here into a reclaim whose
    /// `UPDATE` matches zero rows, which (absent the `round` cap) would
    /// recurse forever (F1).
    ///
    /// `round` bounds that recursion (F1/F3): each pass through
    /// `branch_on_existing -> reclaim -> branch_on_existing` increments it,
    /// and past [`MAX_RECLAIM_ROUNDS`] this answers `InFlight` rather than
    /// looping — a safe, retryable answer for the caller.
    async fn branch_on_existing(
        &self,
        scope: &IdempotencyScope,
        fingerprint: &[u8],
        now: DateTime<Utc>,
        retention: Duration,
        round: u32,
    ) -> Result<InsertOutcome, DomainError> {
        if round >= MAX_RECLAIM_ROUNDS {
            return Ok(InsertOutcome::InFlight);
        }

        let Some(row) = self.select_existing(scope).await? else {
            return self
                .reclaim(scope, fingerprint, now, retention, round)
                .await;
        };

        if row.state == "in_flight" {
            let age = now - row.created_at;
            if age < IN_FLIGHT_TTL {
                return Ok(InsertOutcome::InFlight);
            }
            return self
                .reclaim(scope, fingerprint, now, retention, round)
                .await;
        }

        // state == "completed"
        if row.expires_at <= now {
            return self
                .reclaim(scope, fingerprint, now, retention, round)
                .await;
        }

        if row.request_fingerprint == fingerprint {
            return Ok(InsertOutcome::Replay(StoredResponse {
                status: row.response_status.unwrap_or_default(),
                body: row.response_body.unwrap_or_default(),
                headers: row.response_headers,
            }));
        }

        Ok(InsertOutcome::Mismatch {
            existing_fingerprint: row.request_fingerprint,
        })
    }

    /// Reclaims a stale `in_flight` row or an expired `completed` row back
    /// into a fresh `in_flight` state, in place (same `id`), so the caller
    /// re-runs the handler exactly as if no row had existed. Guarded by
    /// `expires_at <= now` OR `(state = 'in_flight' AND created_at <= cutoff)`
    /// so a concurrent reclaimer can win this race too — the loser falls
    /// back to `branch_on_existing`'s decision table for whatever row now
    /// exists, which is always a safe (if slightly pessimistic) answer.
    /// These predicates are inclusive to match `branch_on_existing`'s
    /// staleness checks exactly (F1): the `UPDATE` must match whenever
    /// `branch_on_existing` decided to route here, including at the exact
    /// boundary tick, or the fallback below would recurse without bound.
    ///
    /// Mints a fresh `generation` (`Uuid::now_v7()`) on every successful
    /// reclaim, independent of `now`, so a stale claimant's
    /// [`Self::complete`] call can never match the reclaimed row even if the
    /// caller-supplied clock is reused across calls (F2).
    async fn reclaim(
        &self,
        scope: &IdempotencyScope,
        fingerprint: &[u8],
        now: DateTime<Utc>,
        retention: Duration,
        round: u32,
    ) -> Result<InsertOutcome, DomainError> {
        let stale_cutoff = now - IN_FLIGHT_TTL;
        let expires_at = now + retention;
        let generation = Uuid::now_v7();

        let reclaimed = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                UPDATE platform.idempotency_keys
                SET request_fingerprint = $1,
                    generation = $2,
                    state = 'in_flight',
                    created_at = $3,
                    completed_at = NULL,
                    response_status = NULL,
                    response_body = NULL,
                    response_headers = NULL,
                    expires_at = $4
                WHERE principal_id = $5 AND method = $6 AND path = $7 AND key = $8
                  AND (
                    expires_at <= $3
                    OR (state = 'in_flight' AND created_at <= $9)
                  )
                RETURNING id, generation
                "#,
                [
                    fingerprint.to_vec().into(),
                    generation.into(),
                    now.into(),
                    expires_at.into(),
                    scope.principal_id.into(),
                    scope.method.clone().into(),
                    scope.path.clone().into(),
                    scope.key.clone().into(),
                    stale_cutoff.into(),
                ],
            ))
            .await
            .map_err(db_err)?;

        if let Some(row) = reclaimed {
            let id: Uuid = row.try_get("", "id").map_err(db_err)?;
            let generation: Uuid = row.try_get("", "generation").map_err(db_err)?;
            return Ok(InsertOutcome::Fresh { id, generation });
        }

        // Either a concurrent reclaimer won this race, or the row vanished
        // (e.g. a concurrent cleanup deleted it). A missing row re-attempts
        // the INSERT once, bounded; either way, fall through to the same
        // decision table `branch_on_existing` uses for a losing insert.
        if self.select_existing(scope).await?.is_none() {
            let retry_id = Uuid::now_v7();
            let retry_generation = Uuid::now_v7();
            if self
                .try_insert(
                    retry_id,
                    retry_generation,
                    scope,
                    fingerprint,
                    now,
                    expires_at,
                )
                .await?
            {
                return Ok(InsertOutcome::Fresh {
                    id: retry_id,
                    generation: retry_generation,
                });
            }
        }

        // `Box::pin` satisfies the compiler's recursive-async-fn check.
        Box::pin(self.branch_on_existing(scope, fingerprint, now, retention, round + 1)).await
    }
}
