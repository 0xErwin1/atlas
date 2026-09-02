use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::ports::search::SearchAfter;
use atlas_acta::ports::search::SearchRepo;
use atlas_acta::ports::search::SortKey as DomainSortKey;
use atlas_acta::search::SearchKind;
use atlas_acta::search::SearchQuery;
use atlas_acta::search::SearchSort;
use atlas_acta::search::SearchWarning;
use atlas_acta::search::TypeSet;
use atlas_acta::search::parse_query;
use atlas_acta::search::task_filter_on_notes;
use atlas_acta_postgres::repos::search::PgSearchRepo;
use atlas_api::{
    dtos::search::{SearchHitDto, SearchKindDto},
    pagination::{Page, SearchCursor, SortKey as ApiSortKey},
};
use atlas_custos::capability::CapabilityFamily;

use crate::{authz::WorkspaceAccess, error::ApiError, hybrid_search, state::AppState};

/// Query parameters for `GET /api/workspaces/{ws}/search`.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchQueryParams {
    /// Free-text query with optional `key:value` filter tokens. Required.
    pub q: Option<String>,
    /// Comma-separated content kinds: `note` (documents), `task`. `all` or empty = no restriction (default). Unknown values are ignored.
    #[serde(rename = "type")]
    pub type_filter: Option<String>,
    /// Sort order: `relevance` (default) or `updated`.
    pub sort: Option<String>,
    /// Opaque pagination cursor returned by the previous response.
    pub cursor: Option<String>,
    /// Maximum results per page. Default 50, clamped to [1, 200].
    pub limit: Option<u32>,
    /// When true, match each query word as a prefix (typeahead). Default false.
    pub prefix: Option<bool>,
    /// Retrieval mode: `lexical` (default), `semantic`, or `hybrid`.
    pub mode: Option<String>,
}

/// How a search request retrieves its candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchMode {
    Lexical,
    Semantic,
    Hybrid,
}

impl SearchMode {
    fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        match raw.map(str::trim).unwrap_or("lexical") {
            "" | "lexical" => Ok(Self::Lexical),
            "semantic" => Ok(Self::Semantic),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(ApiError::InvalidInput {
                message: format!("unknown search mode '{other}'; use lexical, semantic or hybrid"),
            }),
        }
    }

    fn uses_vectors(self) -> bool {
        matches!(self, Self::Semantic | Self::Hybrid)
    }
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/search
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/search",
    tag = "search",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("q" = String, Query, description = "Search query (required). Supports key:value filter tokens."),
        ("type" = Option<String>, Query, description = "Comma-separated kinds: note, task (e.g. note,task). 'all' or empty = no restriction (default). Unknown values are ignored."),
        ("sort" = Option<String>, Query, description = "Sort order: relevance (default) | updated"),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor; must match the sort of the issuing request"),
        ("limit" = Option<u32>, Query, description = "Page size, default 50, clamped to [1,200]"),
        ("prefix" = Option<bool>, Query, description = "When true, match each query word as a prefix (typeahead). Default false."),
        ("mode" = Option<String>, Query, description = "Retrieval mode: lexical (default) | semantic | hybrid. semantic and hybrid rank by fused relevance and require sort=relevance; hybrid falls back to lexical results when embeddings are unavailable."),
    ),
    responses(
        (status = 200, description = "Search results page", body = inline(Page<SearchHitDto>)),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or principal has no access"),
        (status = 422, description = "Invalid input: absent q, unknown mode, malformed cursor, or cursor/sort mismatch"),
        (status = 503, description = "mode=semantic while embeddings are disabled or their schema is unavailable"),
    )
)]
pub(crate) async fn search(
    auth: WorkspaceAccess,
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200) as u64;

    let raw_q = params.q.as_deref().ok_or_else(|| ApiError::InvalidInput {
        message: "query parameter 'q' is required".into(),
    })?;

    let mut query = parse_query(raw_q);
    apply_param_overrides(
        &mut query,
        params.type_filter.as_deref(),
        params.sort.as_deref(),
    );
    query.prefix = params.prefix.unwrap_or(false);

    let mode = SearchMode::parse(params.mode.as_deref())?;
    if mode.uses_vectors() && query.sort != SearchSort::Relevance {
        return Err(ApiError::InvalidInput {
            message: "sort=updated is only available in lexical mode; a fused ranking has no \
                      recency order to page by"
                .into(),
        });
    }

    let after = resolve_cursor(params.cursor.as_deref(), &query)?;

    if query.warnings.contains(&SearchWarning::TaskFilterOnNotes) {
        return Ok(Json(Page::<SearchHitDto>::empty()).into_response());
    }

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    // Scope-gate cross-family read: an API key only sees hits for families it holds
    // `{family}:read` on. Humans (and root/bypass) carry `read_scopes == None` and
    // read every family. The two booleans are ANDed into the repo's per-arm emit
    // toggles, dropping an entire family arm BEFORE the LIMIT/cursor logic so page
    // size, `has_more`, and `next_cursor` stay exactly correct.
    let (may_read_docs, may_read_tasks) = match &auth.read_scopes {
        Some(scopes) => (
            scopes.allows(CapabilityFamily::Docs),
            scopes.allows(CapabilityFamily::Tasks),
        ),
        None => (true, true),
    };

    let principal = auth.principal;
    let bypass = auth.bypass;

    if mode.uses_vectors() {
        return fused_search(
            &state,
            &ctx,
            &principal,
            &query,
            mode,
            limit,
            params.cursor.as_deref(),
            bypass,
            may_read_docs,
            may_read_tasks,
        )
        .await;
    }

    let repo = PgSearchRepo::new((*state.db).clone());
    let hits = repo
        .search(
            &ctx,
            &principal,
            &query,
            limit + 1,
            after,
            bypass,
            may_read_docs,
            may_read_tasks,
        )
        .await
        .map_err(ApiError::Domain)?;

    let has_more = hits.len() as u64 > limit;
    let mut hits = hits;
    if has_more {
        hits.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        hits.last().map(|h| {
            let key = match query.sort {
                SearchSort::Relevance => ApiSortKey::Relevance(h.score),
                SearchSort::UpdatedDesc => ApiSortKey::Updated(h.updated_at.timestamp_micros()),
            };
            SearchCursor { key, id: h.id }
        })
    } else {
        None
    };

    let dtos: Vec<SearchHitDto> = hits.into_iter().map(hit_to_dto).collect();
    let page = Page::new_search(dtos, next_cursor, has_more);

    Ok(Json(page).into_response())
}

// ---------------------------------------------------------------------------
// Fused retrieval (mode=semantic | hybrid)
// ---------------------------------------------------------------------------

/// Answers a `semantic` or `hybrid` request by fusing both arms' rankings.
///
/// Ranking, not score, is what crosses between the arms: a `ts_rank_cd` value
/// and a cosine distance are not comparable, so the fused order is built with
/// RRF over each arm's candidate pool.
#[allow(clippy::too_many_arguments)]
async fn fused_search(
    state: &AppState,
    ctx: &WorkspaceCtx,
    principal: &atlas_core::principal::Principal,
    query: &SearchQuery,
    mode: SearchMode,
    limit: u64,
    cursor: Option<&str>,
    bypass: bool,
    may_read_docs: bool,
    may_read_tasks: bool,
) -> Result<axum::response::Response, ApiError> {
    let provider = state.embedding_provider.clone();
    let vectors_ready = match provider {
        Some(_) => state.semantic_search_enabled_now().await.map_err(|error| {
            ApiError::Domain(atlas_core::error::DomainError::Internal {
                message: format!("semantic search schema readiness check failed: {error}"),
            })
        })?,
        None => false,
    };

    // Hybrid degrades to its lexical half rather than failing: the results it
    // can still produce are correct, just less complete. `semantic` has no such
    // half, so it refuses instead of silently answering from another retriever.
    if !vectors_ready && mode == SearchMode::Semantic {
        return Err(ApiError::ServiceUnavailable {
            message: "semantic search embeddings are disabled".to_owned(),
        });
    }

    let pool = state.search.hybrid_pool.max(limit as usize);
    let lexical = if mode == SearchMode::Hybrid {
        PgSearchRepo::new((*state.db).clone())
            .search(
                ctx,
                principal,
                query,
                pool as u64,
                None,
                bypass,
                may_read_docs,
                may_read_tasks,
            )
            .await
            .map_err(ApiError::Domain)?
    } else {
        Vec::new()
    };

    let semantic = match (vectors_ready, provider) {
        (true, Some(provider)) => hybrid_search::semantic_candidates(
            &state.db,
            provider,
            ctx.workspace_id,
            principal.clone(),
            query,
            pool,
            bypass,
            may_read_docs,
            may_read_tasks,
        )
        .await
        .map_err(ApiError::Domain)?,
        _ => Vec::new(),
    };

    let fused = hybrid_search::fuse_ranks(&lexical, &semantic, state.search.rrf_k);
    let page = page_from_fused(state, ctx, &lexical, fused, limit, cursor).await?;

    Ok(Json(page).into_response())
}

/// Applies the cursor and page size to a fused ranking, then hydrates the DTOs.
async fn page_from_fused(
    state: &AppState,
    ctx: &WorkspaceCtx,
    lexical: &[atlas_acta::search::SearchHit],
    fused: Vec<hybrid_search::HybridHit>,
    limit: u64,
    cursor: Option<&str>,
) -> Result<Page<SearchHitDto>, ApiError> {
    let after = match cursor {
        Some(raw) => Some(
            SearchCursor::decode(raw).ok_or_else(|| ApiError::InvalidInput {
                message: "cursor is malformed or has an invalid format".into(),
            })?,
        ),
        None => None,
    };

    let mut remaining: Vec<hybrid_search::HybridHit> = match after {
        Some(cursor) => {
            let ApiSortKey::Relevance(score) = cursor.key else {
                return Err(ApiError::InvalidInput {
                    message: "cursor does not match the requested sort order".into(),
                });
            };
            fused
                .into_iter()
                .skip_while(|hit| !is_after(hit, score, cursor.id))
                .collect()
        }
        None => fused,
    };

    let has_more = remaining.len() as u64 > limit;
    remaining.truncate(limit as usize);

    let next_cursor = if has_more {
        remaining.last().map(|hit| SearchCursor {
            key: ApiSortKey::Relevance(hit.score),
            id: hit.id,
        })
    } else {
        None
    };

    let mut updated_at: std::collections::HashMap<_, _> = lexical
        .iter()
        .map(|hit| {
            let kind = match hit.kind {
                SearchKind::Document => hybrid_search::HybridKind::Document,
                SearchKind::Task => hybrid_search::HybridKind::Task,
            };
            ((kind, hit.id), hit.updated_at)
        })
        .collect();

    let (documents, tasks): (Vec<_>, Vec<_>) = remaining
        .iter()
        .filter(|hit| !updated_at.contains_key(&(hit.kind, hit.id)))
        .partition(|hit| hit.kind == hybrid_search::HybridKind::Document);
    let looked_up = hybrid_search::load_updated_at(
        &state.db,
        ctx,
        &documents.iter().map(|hit| hit.id).collect::<Vec<_>>(),
        &tasks.iter().map(|hit| hit.id).collect::<Vec<_>>(),
    )
    .await
    .map_err(ApiError::Domain)?;
    updated_at.extend(looked_up);

    let dtos: Vec<SearchHitDto> = remaining
        .into_iter()
        .map(|hit| {
            let timestamp = updated_at
                .get(&(hit.kind, hit.id))
                .copied()
                .unwrap_or_default();
            SearchHitDto {
                id: hit.id,
                kind: match hit.kind {
                    hybrid_search::HybridKind::Document => SearchKindDto::Document,
                    hybrid_search::HybridKind::Task => SearchKindDto::Task,
                },
                readable_id: hit.readable_id,
                document_slug: hit.document_slug,
                title: hit.title,
                snippet: hit.snippet,
                score: hit.score,
                updated_at: timestamp,
                project_slug: hit.project_slug,
                column_name: hit.column_name,
            }
        })
        .collect();

    Ok(Page::new_search(dtos, next_cursor, has_more))
}

/// Whether a fused hit sits strictly past the cursor in `(score DESC, id DESC)`.
fn is_after(hit: &hybrid_search::HybridHit, score: f32, id: uuid::Uuid) -> bool {
    hit.score < score || (hit.score == score && hit.id < id)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn apply_param_overrides(
    query: &mut SearchQuery,
    type_param: Option<&str>,
    sort_param: Option<&str>,
) {
    if let Some(t) = type_param {
        // Param OVERRIDES any type: token from q (unchanged precedence),
        // and uses the SAME parser as the token path.
        query.type_filter = TypeSet::parse(t);
    }

    if let Some(s) = sort_param {
        query.sort = match s.to_ascii_lowercase().as_str() {
            "updated" => SearchSort::UpdatedDesc,
            _ => SearchSort::Relevance,
        };
    }

    // Recompute the warning AFTER the possible type override, via the SAME
    // predicate the domain parser uses. Because the override may have flipped
    // type_filter, a stale warning from parse_query must also be DROPPED when
    // it no longer applies — otherwise `q="type:note status:open"` + `type=task`
    // would keep a TaskFilterOnNotes warning that is now wrong.
    let task_only_present = query.filters.iter().any(|f| {
        matches!(
            f,
            atlas_acta::search::SearchFilter::Status(_)
                | atlas_acta::search::SearchFilter::Priority(_)
                | atlas_acta::search::SearchFilter::Assignee(_)
        )
    });

    let should_warn = task_filter_on_notes(query.type_filter, task_only_present);
    let has_warn = query.warnings.contains(&SearchWarning::TaskFilterOnNotes);

    if should_warn && !has_warn {
        query.warnings.push(SearchWarning::TaskFilterOnNotes);
    } else if !should_warn && has_warn {
        query
            .warnings
            .retain(|w| *w != SearchWarning::TaskFilterOnNotes);
    }
}

fn resolve_cursor(raw: Option<&str>, query: &SearchQuery) -> Result<Option<SearchAfter>, ApiError> {
    let Some(s) = raw else {
        return Ok(None);
    };

    let cursor = SearchCursor::decode(s).ok_or_else(|| ApiError::InvalidInput {
        message: "cursor is malformed or has an invalid format".into(),
    })?;

    let sort_matches = matches!(
        (&query.sort, &cursor.key),
        (SearchSort::Relevance, ApiSortKey::Relevance(_))
            | (SearchSort::UpdatedDesc, ApiSortKey::Updated(_))
    );
    if !sort_matches {
        return Err(ApiError::InvalidInput {
            message: "cursor does not match the requested sort order".into(),
        });
    }

    let domain_key = match cursor.key {
        ApiSortKey::Relevance(score) => DomainSortKey::Relevance(score),
        ApiSortKey::Updated(micros) => DomainSortKey::Updated(micros),
    };

    Ok(Some(SearchAfter {
        key: domain_key,
        id: cursor.id,
    }))
}

fn hit_to_dto(hit: atlas_acta::search::SearchHit) -> SearchHitDto {
    SearchHitDto {
        id: hit.id,
        kind: match hit.kind {
            SearchKind::Document => SearchKindDto::Document,
            SearchKind::Task => SearchKindDto::Task,
        },
        readable_id: hit.readable_id,
        document_slug: hit.document_slug,
        title: hit.title,
        snippet: hit.snippet,
        score: hit.score,
        updated_at: hit.updated_at,
        project_slug: hit.project_slug,
        column_name: hit.column_name,
    }
}

fn principal_to_actor(principal: &atlas_core::principal::Principal) -> Actor {
    match principal {
        atlas_core::principal::Principal::User(uid) => {
            Actor::User(atlas_acta::actor::UserAttributionId(uid.0))
        }
        atlas_core::principal::Principal::ApiKey(kid) => {
            Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(kid.0))
        }
        atlas_core::principal::Principal::Group(_) => {
            Actor::User(atlas_acta::actor::UserAttributionId(uuid::Uuid::nil()))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (T05 — apply_param_overrides unit tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_acta::search::SearchFilter;
    use atlas_acta::search::SearchQuery;
    use atlas_acta::search::SearchSort;
    use atlas_acta::search::SearchWarning;
    use atlas_acta::search::TypeSet;

    fn base_query() -> SearchQuery {
        SearchQuery {
            text: String::new(),
            filters: vec![],
            sort: SearchSort::Relevance,
            type_filter: TypeSet::all(),
            warnings: vec![],
            prefix: false,
        }
    }

    #[test]
    fn override_note_task_multi_value() {
        let mut q = base_query();
        apply_param_overrides(&mut q, Some("note,task"), None);
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: true,
                tasks: true
            }
        );
    }

    #[test]
    fn override_note_backward_compat() {
        let mut q = base_query();
        apply_param_overrides(&mut q, Some("note"), None);
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: true,
                tasks: false
            }
        );
    }

    #[test]
    fn override_task_backward_compat() {
        let mut q = base_query();
        apply_param_overrides(&mut q, Some("task"), None);
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: false,
                tasks: true
            }
        );
    }

    #[test]
    fn override_all_backward_compat() {
        let mut q = base_query();
        apply_param_overrides(&mut q, Some("all"), None);
        assert_eq!(q.type_filter, TypeSet::all());
    }

    #[test]
    fn override_empty_string_collapses_to_all() {
        let mut q = base_query();
        apply_param_overrides(&mut q, Some(""), None);
        assert_eq!(q.type_filter, TypeSet::all());
    }

    #[test]
    fn override_none_leaves_type_unchanged() {
        let mut q = base_query();
        q.type_filter = TypeSet {
            notes: true,
            tasks: false,
        };
        apply_param_overrides(&mut q, None, None);
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: true,
                tasks: false
            }
        );
    }

    #[test]
    fn override_precedence_replaces_token_derived_value() {
        let mut q = base_query();
        q.type_filter = TypeSet {
            notes: true,
            tasks: false,
        };
        apply_param_overrides(&mut q, Some("task"), None);
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: false,
                tasks: true
            }
        );
    }

    #[test]
    fn override_warning_add_path() {
        let mut q = base_query();
        q.filters.push(SearchFilter::Status("open".to_string()));
        apply_param_overrides(&mut q, Some("note"), None);
        assert!(q.warnings.contains(&SearchWarning::TaskFilterOnNotes));
    }

    #[test]
    fn override_warning_drop_path() {
        let mut q = base_query();
        q.filters.push(SearchFilter::Status("open".to_string()));
        q.type_filter = TypeSet {
            notes: true,
            tasks: false,
        };
        q.warnings.push(SearchWarning::TaskFilterOnNotes);

        apply_param_overrides(&mut q, Some("note,task"), None);

        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: true,
                tasks: true
            }
        );
        assert!(
            !q.warnings.contains(&SearchWarning::TaskFilterOnNotes),
            "stale TaskFilterOnNotes warning must be removed when type is widened to include tasks"
        );
    }
}
