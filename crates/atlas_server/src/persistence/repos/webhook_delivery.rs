use atlas_domain::DomainError;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use uuid::Uuid;

use crate::persistence::entities::webhook_delivery::webhook_delivery_log;

pub struct PgWebhookDeliveryRepo;

impl PgWebhookDeliveryRepo {
    /// Appends one delivery-attempt log row for a single subscription dispatch.
    ///
    /// `outcome` is one of `"success"` or `"failure"`.
    #[allow(clippy::too_many_arguments)]
    pub async fn append_log(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        subscription_id: Uuid,
        outbox_event_id: Uuid,
        attempt_no: i32,
        outcome: String,
        status_code: Option<i32>,
        response_snippet: Option<String>,
        error: Option<String>,
        duration_ms: Option<i32>,
    ) -> Result<webhook_delivery_log::Model, DomainError> {
        let model = webhook_delivery_log::ActiveModel {
            id: Set(Uuid::now_v7()),
            workspace_id: Set(workspace_id),
            subscription_id: Set(subscription_id),
            outbox_event_id: Set(outbox_event_id),
            attempt_no: Set(attempt_no),
            outcome: Set(outcome),
            status_code: Set(status_code),
            response_snippet: Set(response_snippet),
            error: Set(error),
            duration_ms: Set(duration_ms),
            created_at: Set(Utc::now()),
        };

        model.insert(conn).await.map_err(db_err)
    }

    /// Returns delivery log entries for a subscription, ordered by creation time
    /// descending (newest first), paginated by `after_id` cursor.
    pub async fn list_for_subscription(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        subscription_id: Uuid,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<webhook_delivery_log::Model>, DomainError> {
        let mut q = webhook_delivery_log::Entity::find()
            .filter(webhook_delivery_log::Column::WorkspaceId.eq(workspace_id))
            .filter(webhook_delivery_log::Column::SubscriptionId.eq(subscription_id));

        if let Some(cursor) = after_id {
            q = q.filter(webhook_delivery_log::Column::Id.lt(cursor));
        }

        q.order_by_desc(webhook_delivery_log::Column::CreatedAt)
            .limit(limit)
            .all(conn)
            .await
            .map_err(db_err)
    }

    /// Returns the subscription IDs that have a `success` delivery log row for
    /// the given `outbox_event_id`.
    ///
    /// The dispatcher uses this to skip subscriptions that already received the
    /// event successfully (idempotency on retry after a partial-success batch).
    pub async fn succeeded_subscription_ids_for_event(
        conn: &impl ConnectionTrait,
        outbox_event_id: Uuid,
    ) -> Result<Vec<Uuid>, DomainError> {
        let rows = webhook_delivery_log::Entity::find()
            .filter(webhook_delivery_log::Column::OutboxEventId.eq(outbox_event_id))
            .filter(webhook_delivery_log::Column::Outcome.eq("success"))
            .select_only()
            .column(webhook_delivery_log::Column::SubscriptionId)
            .into_tuple::<Uuid>()
            .all(conn)
            .await
            .map_err(db_err)?;

        Ok(rows)
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
