//! Reciprocal Rank Fusion of the lexical and vector search arms.
//!
//! The two arms fail in different directions: `ts_rank_cd` cannot match "how do
//! we authenticate" against a document that says "OAuth flow", and the vector
//! arm cannot match a literal `ATL-1247`. RRF combines them by rank instead of
//! by score, which is what makes the combination possible at all — a `ts_rank_cd`
//! value and a cosine distance are not comparable quantities, and any attempt to
//! normalize one into the other encodes an arbitrary weighting.

use atlas_domain::{
    DomainError, WorkspaceCtx,
    ids::WorkspaceId,
    permissions::Principal,
    search::{SearchHit, SearchQuery},
    semantic_search::{
        SemanticSearchHit, SemanticSearchQuery, SemanticSearchRepo, SemanticSearchTypeFilter,
    },
};
use chrono::{DateTime, Utc};
use sea_orm::{DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};
use std::collections::HashMap;
use uuid::Uuid;

use crate::persistence::repos::PgSemanticSearchRepo;

/// The kinds both arms agree on, used to key a resource across them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HybridKind {
    Document,
    Task,
}

/// One fused result, ranked by the sum of its reciprocal ranks.
#[derive(Debug, Clone, PartialEq)]
pub struct HybridHit {
    pub id: Uuid,
    pub kind: HybridKind,
    pub readable_id: Option<String>,
    pub title: String,
    pub project_slug: Option<String>,
    pub column_name: Option<String>,
    /// Highlighted lexical snippet, or the semantic excerpt for a hit only the
    /// vector arm found.
    pub snippet: Option<String>,
    pub score: f32,
}

/// Fuses both arms into one ranking.
///
/// `k` damps the contribution of the top ranks: a small `k` lets a single arm's
/// first result dominate, a large one flattens the two arms toward equal weight.
/// 60 is the value the original RRF paper used and the usual starting point, but
/// it is worth measuring against a real corpus, which is why it is configurable.
///
/// Each input must already be ordered best-first — the position in the slice is
/// the rank. A resource found by both arms sums both contributions, which is
/// what makes agreement between two independent retrievers outrank a strong
/// showing in either one alone.
pub fn fuse_ranks(lexical: &[SearchHit], semantic: &[SemanticSearchHit], k: f32) -> Vec<HybridHit> {
    let k = k.max(1.0);
    let mut fused: Vec<HybridHit> = Vec::new();
    let mut position: HashMap<(HybridKind, Uuid), usize> = HashMap::new();

    for (rank, hit) in lexical.iter().enumerate() {
        let kind = lexical_kind(hit);
        let key = (kind, hit.id);
        position.insert(key, fused.len());
        fused.push(HybridHit {
            id: hit.id,
            kind,
            readable_id: hit.readable_id.clone(),
            title: hit.title.clone(),
            project_slug: hit.project_slug.clone(),
            column_name: hit.column_name.clone(),
            snippet: hit.snippet.clone(),
            score: reciprocal_rank(rank, k),
        });
    }

    for (rank, hit) in semantic.iter().enumerate() {
        let kind = semantic_kind(hit);
        let contribution = reciprocal_rank(rank, k);
        match position.get(&(kind, hit.id)) {
            Some(index) => {
                if let Some(existing) = fused.get_mut(*index) {
                    existing.score += contribution;
                    if existing.snippet.is_none() {
                        existing.snippet = Some(hit.excerpt.clone());
                    }
                }
            }
            None => {
                position.insert((kind, hit.id), fused.len());
                fused.push(HybridHit {
                    id: hit.id,
                    kind,
                    readable_id: hit.readable_id.clone(),
                    title: hit.title.clone(),
                    project_slug: hit.project_slug.clone(),
                    column_name: hit.column_name.clone(),
                    snippet: Some(hit.excerpt.clone()),
                    score: contribution,
                });
            }
        }
    }

    // Ties are broken by id so a page boundary lands in the same place on every
    // request; float ordering is total here because every score is a finite sum
    // of positive reciprocals.
    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.id.cmp(&a.id))
    });
    fused
}

/// Runs the vector arm for a fused search.
///
/// Takes the same reading of the request the lexical arm gets — type filter,
/// per-family read capabilities, bypass — so a family the caller cannot read
/// contributes nothing here either.
#[allow(clippy::too_many_arguments)]
pub async fn semantic_candidates(
    db: &DatabaseConnection,
    provider: std::sync::Arc<dyn atlas_domain::semantic_search::EmbeddingProvider>,
    workspace_id: WorkspaceId,
    principal: Principal,
    query: &SearchQuery,
    pool: usize,
    bypass: bool,
    may_read_docs: bool,
    may_read_tasks: bool,
) -> Result<Vec<SemanticSearchHit>, DomainError> {
    if query.text.trim().is_empty() {
        return Ok(Vec::new());
    }

    let repo = PgSemanticSearchRepo::new(db.clone(), provider);
    repo.search(&SemanticSearchQuery::new(
        workspace_id,
        principal,
        query.text.clone(),
        SemanticSearchTypeFilter {
            documents: query.type_filter.notes,
            tasks: query.type_filter.tasks,
        },
        pool as u64,
        None,
        bypass,
        may_read_docs,
        may_read_tasks,
    ))
    .await
}

/// Fills in the `updated_at` of hits only the vector arm found.
///
/// The vector arm ranks embedding rows and never reads the resource's own
/// timestamp, but the search contract carries one for every hit. Looking it up
/// afterwards keeps that contract without widening the vector query, and is
/// bounded by the page size.
pub async fn load_updated_at(
    db: &DatabaseConnection,
    ctx: &WorkspaceCtx,
    documents: &[Uuid],
    tasks: &[Uuid],
) -> Result<HashMap<(HybridKind, Uuid), DateTime<Utc>>, DomainError> {
    #[derive(Debug, FromQueryResult)]
    struct TimestampRow {
        kind: String,
        id: Uuid,
        updated_at: DateTime<Utc>,
    }

    if documents.is_empty() && tasks.is_empty() {
        return Ok(HashMap::new());
    }

    // The ids come from rows this workspace's own arms returned, never from the
    // request, so inlining them carries no injection surface; the alternative is
    // a driver-specific array binding for a query that is otherwise trivial.
    let sql = format!(
        r#"SELECT 'document' AS kind, id, updated_at FROM documents
           WHERE workspace_id = $1 AND id IN ({documents_in})
           UNION ALL
           SELECT 'task' AS kind, id, updated_at FROM tasks
           WHERE workspace_id = $1 AND id IN ({tasks_in})"#,
        documents_in = uuid_list(documents),
        tasks_in = uuid_list(tasks),
    );

    let rows = TimestampRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        sql,
        vec![ctx.workspace_id.0.into()],
    ))
    .all(db)
    .await
    .map_err(|error| DomainError::Internal {
        message: error.to_string(),
    })?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let kind = if row.kind == "task" {
                HybridKind::Task
            } else {
                HybridKind::Document
            };
            ((kind, row.id), row.updated_at)
        })
        .collect())
}

/// Renders a uuid list for an `IN (…)` clause, never empty so the SQL stays valid.
fn uuid_list(ids: &[Uuid]) -> String {
    if ids.is_empty() {
        return "NULL".to_owned();
    }
    ids.iter()
        .map(|id| format!("'{id}'::uuid"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn reciprocal_rank(zero_based_rank: usize, k: f32) -> f32 {
    1.0 / (k + zero_based_rank as f32 + 1.0)
}

fn lexical_kind(hit: &SearchHit) -> HybridKind {
    match hit.kind {
        atlas_domain::search::SearchKind::Document => HybridKind::Document,
        atlas_domain::search::SearchKind::Task => HybridKind::Task,
    }
}

fn semantic_kind(hit: &SemanticSearchHit) -> HybridKind {
    match hit.kind {
        atlas_domain::semantic_search::ResourceKind::Document => HybridKind::Document,
        atlas_domain::semantic_search::ResourceKind::Task => HybridKind::Task,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_domain::{
        search::SearchKind,
        semantic_search::{ResourceKind, SemanticSearchSource},
    };
    use chrono::Utc;

    fn lexical_hit(id: Uuid, title: &str) -> SearchHit {
        SearchHit {
            kind: SearchKind::Document,
            id,
            readable_id: None,
            title: title.to_owned(),
            snippet: Some(format!("<mark>{title}</mark>")),
            score: 0.5,
            updated_at: Utc::now(),
            project_slug: None,
            column_name: None,
        }
    }

    fn semantic_hit(id: Uuid, title: &str) -> SemanticSearchHit {
        SemanticSearchHit {
            kind: ResourceKind::Document,
            id,
            readable_id: None,
            slug: None,
            title: title.to_owned(),
            project_slug: None,
            column_name: None,
            similarity: 0.9,
            source: SemanticSearchSource::Content,
            excerpt: format!("excerpt of {title}"),
        }
    }

    #[test]
    fn agreement_between_arms_outranks_either_arm_alone() {
        let both = Uuid::from_u128(1);
        let lexical_only = Uuid::from_u128(2);
        let semantic_only = Uuid::from_u128(3);

        let fused = fuse_ranks(
            &[
                lexical_hit(lexical_only, "lexical"),
                lexical_hit(both, "both"),
            ],
            &[
                semantic_hit(semantic_only, "semantic"),
                semantic_hit(both, "both"),
            ],
            60.0,
        );

        assert_eq!(fused.len(), 3);
        assert_eq!(
            fused.first().map(|hit| hit.id),
            Some(both),
            "a resource both arms found must rank above either arm's own top hit"
        );
    }

    #[test]
    fn a_resource_only_the_vector_arm_found_still_appears() {
        let semantic_only = Uuid::from_u128(7);

        let fused = fuse_ranks(&[], &[semantic_hit(semantic_only, "concept")], 60.0);

        let hit = fused.first().expect("the vector arm's hit survives fusion");
        assert_eq!(hit.id, semantic_only);
        assert_eq!(hit.snippet.as_deref(), Some("excerpt of concept"));
    }

    #[test]
    fn lexical_snippet_wins_over_the_semantic_excerpt() {
        let both = Uuid::from_u128(9);

        let fused = fuse_ranks(
            &[lexical_hit(both, "both")],
            &[semantic_hit(both, "both")],
            60.0,
        );

        assert_eq!(fused.len(), 1, "the same resource is not emitted twice");
        assert_eq!(
            fused.first().and_then(|hit| hit.snippet.clone()),
            Some("<mark>both</mark>".to_owned()),
            "the highlighted lexical snippet is the more useful of the two"
        );
    }

    #[test]
    fn the_same_id_in_different_families_stays_two_results() {
        let shared = Uuid::from_u128(11);
        let mut task = semantic_hit(shared, "task");
        task.kind = ResourceKind::Task;

        let fused = fuse_ranks(&[lexical_hit(shared, "document")], &[task], 60.0);

        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn k_controls_how_much_a_top_rank_dominates() {
        let first = Uuid::from_u128(21);
        let second = Uuid::from_u128(22);
        let lexical = [lexical_hit(first, "first"), lexical_hit(second, "second")];

        let sharp = fuse_ranks(&lexical, &[], 1.0);
        let flat = fuse_ranks(&lexical, &[], 1_000.0);

        let gap = |fused: &[HybridHit]| {
            let top = fused.first().map_or(0.0, |hit| hit.score);
            let next = fused.get(1).map_or(0.0, |hit| hit.score);
            top - next
        };
        assert!(
            gap(&sharp) > gap(&flat),
            "a smaller k must separate the top ranks more sharply"
        );
    }
}
