#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sea_orm::DatabaseConnection;
use sha2::Sha256;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::DispatcherConfig;
use crate::crypto::WebhookCrypto;
use crate::persistence::entities::events_outbox::event_outbox;
use crate::persistence::entities::webhook_subscription::webhook_subscriptions;
use crate::persistence::repos::{PgOutboxRepo, PgWebhookDeliveryRepo};

type HmacSha256 = Hmac<Sha256>;

/// Polls the transactional outbox and delivers pending events to matching
/// webhook subscriptions.
///
/// The dispatcher runs as a background tokio task spawned in `main.rs`. Shutdown
/// is signalled via a `watch::Receiver<bool>` (set to `true`); the loop completes
/// any in-progress batch before exiting, so no deliveries are abandoned on graceful
/// shutdown.
pub struct WebhookDispatcher {
    db: DatabaseConnection,
    crypto: Arc<WebhookCrypto>,
    config: DispatcherConfig,
    client: reqwest::Client,
}

impl WebhookDispatcher {
    /// Constructs a dispatcher from an existing database connection, crypto
    /// instance, and config.
    ///
    /// The reqwest client is built once here with the configured delivery timeout.
    pub fn new(
        db: DatabaseConnection,
        crypto: Arc<WebhookCrypto>,
        config: DispatcherConfig,
    ) -> Self {
        let timeout = Duration::from_millis(config.delivery_timeout_ms);

        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();

        Self {
            db,
            crypto,
            config,
            client,
        }
    }

    /// Runs the polling loop until `shutdown` receives `true`.
    ///
    /// One full `poll_and_dispatch` cycle runs before each sleep, and any
    /// in-flight batch completes before the loop exits. The loop itself never
    /// panics: errors from individual cycles are logged and do not terminate
    /// the loop.
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        let poll_duration = Duration::from_millis(self.config.poll_interval_ms);

        loop {
            if let Err(e) = self.poll_and_dispatch().await {
                error!(error = %e, "dispatcher poll error");
            }

            if *shutdown.borrow() {
                break;
            }

            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                _ = tokio::time::sleep(poll_duration) => {}
            }
        }

        info!("webhook dispatcher shutting down");
    }

    /// Claims a batch of pending outbox rows and delivers them to matching
    /// subscriptions.
    ///
    /// Concurrent deliveries are bounded by the configured semaphore. Each
    /// delivery runs in its own `JoinSet` task so a panicking delivery does
    /// not kill the loop. The method awaits all tasks before returning, ensuring
    /// complete drain before the caller proceeds.
    pub async fn poll_and_dispatch(&self) -> Result<(), anyhow::Error> {
        let recovered = PgOutboxRepo::recovery_sweep(&self.db)
            .await
            .map_err(|e| anyhow::anyhow!("recovery sweep: {e}"))?;

        if recovered > 0 {
            info!(count = recovered, "recovered stale delivering outbox rows");
        }

        let rows =
            PgOutboxRepo::claim_batch(&self.db, self.config.batch_size, self.config.lease_secs)
                .await
                .map_err(|e| anyhow::anyhow!("claim batch: {e}"))?;

        if rows.is_empty() {
            return Ok(());
        }

        debug!(count = rows.len(), "claimed outbox rows for delivery");

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent));
        let mut join_set: JoinSet<()> = JoinSet::new();

        for row in rows {
            let ctx = DeliveryContext {
                db: self.db.clone(),
                crypto: Arc::clone(&self.crypto),
                client: self.client.clone(),
                max_attempts: self.config.max_attempts,
            };

            let permit = semaphore.clone().acquire_owned().await?;

            join_set.spawn(async move {
                let _permit = permit;
                process_event(ctx, row).await;
            });
        }

        while let Some(result) = join_set.join_next().await {
            if let Err(e) = result {
                error!(error = %e, "delivery task panicked");
            }
        }

        Ok(())
    }
}

/// Bundles the read-only state shared across delivery tasks within one poll cycle.
struct DeliveryContext {
    db: DatabaseConnection,
    crypto: Arc<WebhookCrypto>,
    client: reqwest::Client,
    max_attempts: i32,
}

/// Processes one outbox row: fans out to matching subscriptions and finalizes.
async fn process_event(ctx: DeliveryContext, row: event_outbox::Model) {
    let subscriptions = match PgOutboxRepo::match_active_subscriptions(
        &ctx.db,
        row.workspace_id,
        &row.event_type,
        row.project_id,
        row.board_id,
    )
    .await
    {
        Ok(subs) => subs,
        Err(e) => {
            error!(event_id = %row.id, error = %e, "match_active_subscriptions failed");
            return;
        }
    };

    let already_succeeded = match PgWebhookDeliveryRepo::succeeded_subscription_ids_for_event(
        &ctx.db, row.id,
    )
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            error!(event_id = %row.id, error = %e, "succeeded_subscription_ids_for_event failed");
            return;
        }
    };

    let pending_subs: Vec<_> = subscriptions
        .into_iter()
        .filter(|s| !already_succeeded.contains(&s.id))
        .collect();

    if pending_subs.is_empty() {
        finalize(&ctx.db, row.id, 0, ctx.max_attempts).await;
        return;
    }

    let body_bytes = match serde_json::to_vec(&row.payload) {
        Ok(b) => b,
        Err(e) => {
            error!(event_id = %row.id, error = %e, "failed to serialize outbox payload");
            return;
        }
    };

    let mut failures: i32 = 0;

    for sub in &pending_subs {
        let success = deliver_to_subscription(
            &ctx,
            sub,
            row.id,
            row.workspace_id,
            &body_bytes,
            row.attempt_count,
        )
        .await;

        if !success {
            failures += 1;
        }
    }

    finalize(&ctx.db, row.id, failures, ctx.max_attempts).await;
}

/// Delivers the POST for one subscription and records the attempt in the log.
///
/// Returns `true` on a 2xx response, `false` on any network or HTTP error.
async fn deliver_to_subscription(
    ctx: &DeliveryContext,
    sub: &webhook_subscriptions::Model,
    event_id: Uuid,
    workspace_id: Uuid,
    body_bytes: &[u8],
    attempt_no: i32,
) -> bool {
    let start = std::time::Instant::now();

    let secret = match ctx.crypto.decrypt(&sub.encrypted_secret, &sub.secret_nonce) {
        Ok(s) => s,
        Err(e) => {
            error!(sub_id = %sub.id, event_id = %event_id, error = %e, "decrypt secret failed");
            record_log(
                &ctx.db,
                workspace_id,
                sub.id,
                event_id,
                attempt_no,
                "failure",
                None,
                None,
                Some(e),
                elapsed_ms(start),
            )
            .await;
            return false;
        }
    };

    let signature = match compute_signature(&secret, body_bytes) {
        Ok(s) => s,
        Err(e) => {
            error!(sub_id = %sub.id, event_id = %event_id, error = %e, "HMAC computation failed");
            record_log(
                &ctx.db,
                workspace_id,
                sub.id,
                event_id,
                attempt_no,
                "failure",
                None,
                None,
                Some(e),
                elapsed_ms(start),
            )
            .await;
            return false;
        }
    };

    let response = ctx
        .client
        .post(&sub.target_url)
        .header("Content-Type", "application/json")
        .header("X-Atlas-Signature", &signature)
        .body(body_bytes.to_vec())
        .send()
        .await;

    let duration = elapsed_ms(start);

    match response {
        Err(e) => {
            warn!(sub_id = %sub.id, event_id = %event_id, error = %e, "delivery network error");
            record_log(
                &ctx.db,
                workspace_id,
                sub.id,
                event_id,
                attempt_no,
                "failure",
                None,
                None,
                Some(e.to_string()),
                duration,
            )
            .await;
            false
        }
        Ok(resp) => {
            let status = resp.status();
            let status_code = status.as_u16() as i32;
            let success = status.is_success();

            let snippet = resp
                .text()
                .await
                .ok()
                .map(|t| t.chars().take(256).collect::<String>());

            let outcome = if success { "success" } else { "failure" };

            if !success {
                warn!(
                    sub_id = %sub.id,
                    event_id = %event_id,
                    status = status_code,
                    "delivery returned non-2xx response"
                );
            }

            record_log(
                &ctx.db,
                workspace_id,
                sub.id,
                event_id,
                attempt_no,
                outcome,
                Some(status_code),
                snippet,
                None,
                duration,
            )
            .await;

            success
        }
    }
}

/// Computes `X-Atlas-Signature: sha256=<hex>` over `body` using `secret` as the HMAC key.
pub fn compute_signature(secret: &[u8], body: &[u8]) -> Result<String, String> {
    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|e| format!("HMAC key setup failed: {e}"))?;
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("sha256={hex}"))
}

/// Writes one row to `webhook_delivery_log`, logging the error if it fails.
#[allow(clippy::too_many_arguments)]
async fn record_log(
    db: &DatabaseConnection,
    workspace_id: Uuid,
    subscription_id: Uuid,
    event_id: Uuid,
    attempt_no: i32,
    outcome: &str,
    status_code: Option<i32>,
    snippet: Option<String>,
    error: Option<String>,
    duration_ms: Option<i32>,
) {
    if let Err(e) = PgWebhookDeliveryRepo::append_log(
        db,
        workspace_id,
        subscription_id,
        event_id,
        attempt_no,
        outcome.to_string(),
        status_code,
        snippet,
        error,
        duration_ms,
    )
    .await
    {
        error!(event_id = %event_id, sub_id = %subscription_id, err = %e, "failed to append delivery log");
    }
}

/// Finalizes the outbox row's status after all deliveries in a cycle.
async fn finalize(db: &DatabaseConnection, event_id: Uuid, failures: i32, max_attempts: i32) {
    if let Err(e) = PgOutboxRepo::finalize_event(db, event_id, failures, max_attempts).await {
        error!(event_id = %event_id, error = %e, "finalize_event failed");
    }
}

/// Returns elapsed milliseconds capped at `i32::MAX`.
fn elapsed_ms(start: std::time::Instant) -> Option<i32> {
    let ms = start.elapsed().as_millis().min(i32::MAX as u128) as i32;
    Some(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    // B3.6-1 — compute_signature produces a correctly-prefixed sha256= string
    #[test]
    fn compute_signature_produces_sha256_prefix() {
        let sig = compute_signature(b"my-secret", b"hello").unwrap();
        assert!(
            sig.starts_with("sha256="),
            "signature must start with 'sha256=': {sig}"
        );
    }

    // B3.6-2 — known HMAC vector: HMAC-SHA256("key", "The quick brown fox...")
    // Reference computed with Python: import hmac, hashlib
    // hmac.new(b"key", b"The quick brown fox jumps over the lazy dog", hashlib.sha256).hexdigest()
    #[test]
    fn known_hmac_vector() {
        let sig =
            compute_signature(b"key", b"The quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(
            sig,
            "sha256=f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    // B3.6-3 — different bodies produce different signatures
    #[test]
    fn different_bodies_produce_different_signatures() {
        let sig1 = compute_signature(b"secret", b"body-one").unwrap();
        let sig2 = compute_signature(b"secret", b"body-two").unwrap();
        assert_ne!(sig1, sig2);
    }

    // B3.6-4 — different secrets produce different signatures for the same body
    #[test]
    fn different_secrets_produce_different_signatures() {
        let sig1 = compute_signature(b"secret-a", b"body").unwrap();
        let sig2 = compute_signature(b"secret-b", b"body").unwrap();
        assert_ne!(sig1, sig2);
    }

    // B3.6-5 — elapsed_ms returns Some value
    #[test]
    fn elapsed_ms_returns_some() {
        let start = std::time::Instant::now();
        let result = elapsed_ms(start);
        assert!(result.is_some());
    }
}
