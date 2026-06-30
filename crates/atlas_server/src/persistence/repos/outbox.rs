use atlas_domain::{
    DomainError, WorkspaceCtx,
    entities::events::{DomainEvent, EventActor, EventEnvelope},
    ids::{BoardId, ProjectId},
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseBackend, EntityTrait, Statement};
use uuid::Uuid;

use crate::persistence::entities::events_outbox::event_outbox;
use crate::persistence::entities::webhook_subscription::webhook_subscriptions;

/// Persistence operations for the transactional outbox and subscription
/// matching needed by the dispatcher.
pub struct PgOutboxRepo;

impl PgOutboxRepo {
    /// Inserts a fully-formed `EventEnvelope` into `events_outbox` within the
    /// caller's transaction.
    ///
    /// The row is inserted with `status = 'pending'`. The call MUST run on the
    /// SAME `ConnectionTrait` (i.e. the same open `DatabaseTransaction`) as the
    /// domain mutation so a rollback on either side leaves no orphaned row.
    pub async fn insert_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        project_id: Option<ProjectId>,
        board_id: Option<BoardId>,
        event: DomainEvent,
    ) -> Result<(), DomainError> {
        let actor = EventActor::from(ctx.actor);
        let envelope = EventEnvelope::new(ctx.workspace_id, project_id, board_id, actor, event);

        let aggregate_type = envelope.data.aggregate_type().to_string();
        let aggregate_id = envelope.data.aggregate_id();
        let event_type = envelope.event_type.clone();
        let event_version = envelope.version;
        let occurred_at = envelope.occurred_at;

        let payload = serde_json::to_value(&envelope).map_err(|e| DomainError::Internal {
            message: format!("failed to serialize event envelope: {e}"),
        })?;

        let now = Utc::now();

        let model = event_outbox::ActiveModel {
            id: Set(envelope.id),
            workspace_id: Set(ctx.workspace_id.0),
            event_type: Set(event_type),
            event_version: Set(event_version),
            source: Set("internal".to_string()),
            project_id: Set(project_id.map(|id| id.0)),
            board_id: Set(board_id.map(|id| id.0)),
            aggregate_type: Set(aggregate_type),
            aggregate_id: Set(aggregate_id),
            payload: Set(payload),
            occurred_at: Set(occurred_at),
            status: Set("pending".to_string()),
            attempt_count: Set(0),
            next_attempt_at: Set(now),
            locked_until: Set(None),
            last_error: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };

        model.insert(conn).await.map_err(db_err)?;
        Ok(())
    }

    /// Claims up to `batch_size` pending rows for delivery, marking them
    /// `delivering` and incrementing `attempt_count`.
    ///
    /// `FOR UPDATE SKIP LOCKED` ensures each row is claimed by at most one
    /// concurrent dispatcher instance. The row is held until `locked_until`
    /// (NOW + `lease_seconds`), after which `recovery_sweep` reclaims it.
    pub async fn claim_batch(
        conn: &impl ConnectionTrait,
        batch_size: i64,
        lease_seconds: i64,
    ) -> Result<Vec<event_outbox::Model>, DomainError> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            UPDATE events_outbox
            SET status        = 'delivering',
                locked_until  = NOW() + ($2 * INTERVAL '1 second'),
                attempt_count = attempt_count + 1,
                updated_at    = NOW()
            WHERE id IN (
                SELECT id FROM events_outbox
                WHERE  status          = 'pending'
                  AND  next_attempt_at <= NOW()
                ORDER  BY occurred_at
                FOR    UPDATE SKIP LOCKED
                LIMIT  $1
            )
            RETURNING id, workspace_id, event_type, event_version, source,
                      project_id, board_id, aggregate_type, aggregate_id,
                      payload, occurred_at, status, attempt_count,
                      next_attempt_at, locked_until, last_error, created_at, updated_at
            "#,
            [batch_size.into(), lease_seconds.into()],
        );

        event_outbox::Entity::find()
            .from_raw_sql(stmt)
            .all(conn)
            .await
            .map_err(db_err)
    }

    /// Resets any `delivering` rows whose `locked_until` has elapsed back to
    /// `pending`, making them eligible for the next `claim_batch` cycle.
    ///
    /// Returns the number of rows recovered.
    pub async fn recovery_sweep(conn: &impl ConnectionTrait) -> Result<u64, DomainError> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            UPDATE events_outbox
            SET status       = 'pending',
                locked_until = NULL,
                updated_at   = NOW()
            WHERE status       = 'delivering'
              AND locked_until  < NOW()
            "#,
            [],
        );

        let result = conn.execute_raw(stmt).await.map_err(db_err)?;
        Ok(result.rows_affected())
    }

    /// Records the final disposition of an outbox row after a dispatcher
    /// delivery cycle.
    ///
    /// - `subs_remaining == 0` → `delivered` immediately.
    /// - `subs_remaining > 0` and `attempt_count >= max_attempts` → `dead`.
    /// - Otherwise → back to `pending` with exponential backoff on
    ///   `next_attempt_at` (2^attempt_count seconds, capped at 2^10 ≈ 17 min).
    pub async fn finalize_event(
        conn: &impl ConnectionTrait,
        id: Uuid,
        subs_remaining: i32,
        max_attempts: i32,
    ) -> Result<(), DomainError> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            UPDATE events_outbox
            SET status           = CASE
                                       WHEN $2 = 0             THEN 'delivered'
                                       WHEN attempt_count >= $3 THEN 'dead'
                                       ELSE 'pending'
                                   END,
                locked_until     = NULL,
                next_attempt_at  = CASE
                                       WHEN $2 > 0 AND attempt_count < $3
                                       THEN NOW() + (POWER(2, LEAST(attempt_count, 10))::int
                                                     * INTERVAL '1 second')
                                       ELSE next_attempt_at
                                   END,
                updated_at       = NOW()
            WHERE id = $1
            "#,
            [id.into(), subs_remaining.into(), max_attempts.into()],
        );

        conn.execute_raw(stmt).await.map_err(db_err)?;
        Ok(())
    }

    /// Returns all active webhook subscriptions in `workspace_id` whose
    /// `event_types` include `event_type` and whose scope matches the event.
    ///
    /// Scope matching rules:
    /// - `scope_type = 'workspace'` always matches.
    /// - `scope_type = 'project'` matches when `scope_id = project_id`.
    /// - `scope_type = 'board'`   matches when `scope_id = board_id`.
    pub async fn match_active_subscriptions(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        event_type: &str,
        project_id: Option<Uuid>,
        board_id: Option<Uuid>,
    ) -> Result<Vec<webhook_subscriptions::Model>, DomainError> {
        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            SELECT id, workspace_id, target_url, event_types, scope_type, scope_id,
                   encrypted_secret, secret_nonce, is_active, label,
                   created_by_user_id, created_by_api_key_id,
                   created_at, updated_at, deleted_at
            FROM   webhook_subscriptions
            WHERE  workspace_id = $1
              AND  is_active    = true
              AND  deleted_at   IS NULL
              AND  $2           = ANY(event_types)
              AND  (
                       scope_type = 'workspace'
                    OR (scope_type = 'project' AND scope_id = $3)
                    OR (scope_type = 'board'   AND scope_id = $4)
              )
            "#,
            [
                workspace_id.into(),
                event_type.into(),
                project_id.into(),
                board_id.into(),
            ],
        );

        webhook_subscriptions::Entity::find()
            .from_raw_sql(stmt)
            .all(conn)
            .await
            .map_err(db_err)
    }

    /// Inserts an externally-sourced event into `events_outbox`, keyed on
    /// `delivery_id` for idempotent dedup.
    ///
    /// The stored `payload` mirrors the `EventEnvelope` JSON shape so the
    /// existing `WebhookDispatcher` fans it out to subscribers without
    /// modification. `source` is e.g. `"external/github"` and `event_type`
    /// is e.g. `"external.github.workflow_run"`.
    ///
    /// Returns `Ok(true)` when a new row was inserted and `Ok(false)` when
    /// `delivery_id` was already present (`ON CONFLICT (id) DO NOTHING`).
    /// The caller is responsible for committing or rolling back any enclosing
    /// transaction.
    pub async fn insert_external_in(
        conn: &impl ConnectionTrait,
        delivery_id: Uuid,
        workspace_id: Uuid,
        source: &str,
        event_type: &str,
        actor_api_key_id: Uuid,
        data: serde_json::Value,
    ) -> Result<bool, DomainError> {
        let occurred_at = Utc::now();

        let payload = serde_json::json!({
            "id": delivery_id,
            "event_type": event_type,
            "version": 1,
            "source": source,
            "workspace_id": workspace_id,
            "project_id": null,
            "board_id": null,
            "occurred_at": occurred_at,
            "actor": {
                "type": "api_key",
                "id": actor_api_key_id
            },
            "data": data
        });

        let stmt = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            INSERT INTO events_outbox (
                id, workspace_id, event_type, event_version, source,
                project_id, board_id, aggregate_type, aggregate_id,
                payload, occurred_at, status, attempt_count, next_attempt_at,
                locked_until, last_error, created_at, updated_at
            )
            VALUES (
                $1, $2, $3, 1, $4,
                NULL, NULL, 'external', $1,
                $5, $6, 'pending', 0, $6,
                NULL, NULL, $6, $6
            )
            ON CONFLICT (id) DO NOTHING
            "#,
            [
                delivery_id.into(),
                workspace_id.into(),
                event_type.into(),
                source.into(),
                payload.into(),
                occurred_at.into(),
            ],
        );

        let result = conn.execute_raw(stmt).await.map_err(db_err)?;
        Ok(result.rows_affected() == 1)
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
