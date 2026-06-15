use async_trait::async_trait;
use uuid::Uuid;

use crate::{DomainError, WorkspaceCtx, permissions::Principal, search::{SearchHit, SearchQuery}};

/// Sort discriminant for the search result ordering key.
///
/// Mirrors `atlas_api::pagination::SortKey` 1:1; the route maps between them.
/// This type lives in `atlas_domain` to keep the port pure (no atlas_api dep).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortKey {
    /// `ts_rank_cd` relevance score (`f32`).
    Relevance(f32),
    /// `updated_at` as epoch microseconds (`i64`), lossless for PostgreSQL TIMESTAMPTZ.
    Updated(i64),
}

/// Resume token passed to `SearchRepo::search` to continue from the last seen row.
///
/// `key` MUST match `query.sort` — the route validates this before the repo is called.
#[derive(Debug, Clone, Copy)]
pub struct SearchAfter {
    pub key: SortKey,
    pub id: Uuid,
}

/// Port (domain-level repository interface) for unified full-text search.
///
/// Implementations live in `atlas_server` (via `PgSearchRepo`). The domain
/// layer stays pure; no SQL or HTTP knowledge crosses this boundary.
#[async_trait]
pub trait SearchRepo: Send + Sync {
    /// Returns the next slice of search hits visible to `principal`, ordered by
    /// the active sort key (DESC), then by `id` DESC as a stable tiebreak.
    ///
    /// The caller passes `limit + 1` to detect whether a next page exists.
    /// Permission filtering is pushed entirely into the SQL predicate so that
    /// the LIMIT applies after permission-visible rows only.
    ///
    /// `after.key` is guaranteed to match `query.sort` (validated by the route).
    async fn search(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        query: &SearchQuery,
        limit: u64,
        after: Option<SearchAfter>,
    ) -> Result<Vec<SearchHit>, DomainError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_key_relevance_and_updated_are_distinct() {
        let r = SortKey::Relevance(0.5_f32);
        let u = SortKey::Updated(0_i64);
        // They carry different data and are not equal
        assert_ne!(
            std::mem::discriminant(&r),
            std::mem::discriminant(&u)
        );
    }

    #[test]
    fn sort_key_relevance_stores_value() {
        let SortKey::Relevance(score) = SortKey::Relevance(0.5_f32) else {
            panic!("wrong variant");
        };
        assert_eq!(score, 0.5_f32);
    }

    #[test]
    fn sort_key_updated_stores_value() {
        let SortKey::Updated(micros) = SortKey::Updated(1_000_000_i64) else {
            panic!("wrong variant");
        };
        assert_eq!(micros, 1_000_000_i64);
    }

    #[test]
    fn search_after_fields_accessible() {
        let id = Uuid::now_v7();
        let after = SearchAfter {
            key: SortKey::Relevance(0.9),
            id,
        };
        assert_eq!(after.id, id);
        assert!(matches!(after.key, SortKey::Relevance(_)));
    }
}
