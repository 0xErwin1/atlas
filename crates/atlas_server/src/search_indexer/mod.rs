use std::sync::Arc;
use std::time::Duration;

use atlas_acta::semantic_search::SemanticIndexer;
use sea_orm::DatabaseConnection;
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::persistence::repos::{PgSearchIndexQueueRepo, QueuedResource};

/// How long a claimed row stays leased to this worker.
///
/// Generous relative to a poll cycle: the lease only needs to outlive one
/// embedding round-trip, and reclaiming early would double-embed rather than
/// lose work.
const LEASE_SECONDS: i64 = 120;

/// Drains `search_index_queue`, re-embedding each dirty resource.
///
/// Runs as a background tokio task alongside the webhook dispatcher. A failing
/// resource is released with backoff rather than dropped, and never stalls the
/// rest of the batch.
pub struct SearchIndexWorker {
    db: DatabaseConnection,
    indexer: Arc<dyn SemanticIndexer>,
    poll_interval: Duration,
    batch_size: i64,
}

impl SearchIndexWorker {
    pub fn new(
        db: DatabaseConnection,
        indexer: Arc<dyn SemanticIndexer>,
        poll_interval: Duration,
        batch_size: i64,
    ) -> Self {
        Self {
            db,
            indexer,
            poll_interval,
            batch_size,
        }
    }

    /// Polls until `shutdown` flips to `true`, finishing the in-flight batch.
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        loop {
            match self.drain_once().await {
                Ok(0) => {}
                Ok(count) => debug!(count, "search indexer embedded resources"),
                Err(error) => error!(%error, "search indexer poll failed"),
            }

            if *shutdown.borrow() {
                break;
            }

            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                _ = tokio::time::sleep(self.poll_interval) => {}
            }
        }
    }

    /// Claims one batch and indexes it. Returns how many rows were completed.
    ///
    /// Sequential on purpose: embedding providers bill and rate-limit per
    /// request, and the queue coalesces, so depth here costs latency on a
    /// background path rather than throughput on a user-facing one.
    pub async fn drain_once(&self) -> Result<usize, atlas_core::error::DomainError> {
        let claimed =
            PgSearchIndexQueueRepo::claim_batch(&self.db, self.batch_size, LEASE_SECONDS).await?;

        let mut completed = 0;
        for row in claimed {
            if self.index_one(&row).await {
                completed += 1;
            }
        }
        Ok(completed)
    }

    async fn index_one(&self, row: &QueuedResource) -> bool {
        match self
            .indexer
            .index_resource(row.workspace_id, row.kind, row.resource_id)
            .await
        {
            Ok(()) => {
                if let Err(error) =
                    PgSearchIndexQueueRepo::complete(&self.db, row.id, row.enqueued_at).await
                {
                    // The resource is embedded; only the bookkeeping failed. The
                    // lease expiry makes the row claimable again and the second
                    // pass is a no-op, so this is safe to leave for the retry.
                    warn!(%error, resource_id = %row.resource_id, "failed to clear indexed queue row");
                    return false;
                }
                true
            }
            Err(error) => {
                warn!(%error, resource_id = %row.resource_id, kind = ?row.kind, "failed to index resource");
                if let Err(fail_error) = PgSearchIndexQueueRepo::fail(
                    &self.db,
                    row.id,
                    row.attempt_count,
                    &error.to_string(),
                )
                .await
                {
                    error!(%fail_error, "failed to record search index failure");
                }
                false
            }
        }
    }
}
