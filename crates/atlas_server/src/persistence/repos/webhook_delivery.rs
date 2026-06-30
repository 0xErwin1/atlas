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
    /// `outcome` is one of `"success"`, `"http_error"`, or `"network_error"`.
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
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
