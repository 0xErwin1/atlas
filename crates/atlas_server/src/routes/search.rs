use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::{
    dtos::search::{SearchHitDto, SearchKindDto},
    pagination::{Page, SearchCursor, SortKey as ApiSortKey},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    ports::search::{SearchAfter, SearchRepo, SortKey as DomainSortKey},
    search::{SearchKind, SearchQuery, SearchSort, TypeFilter, SearchWarning, parse_query},
};

use crate::{
    authz::{Authorized, ViewerMin, authorized::WorkspaceRes},
    error::ApiError,
    persistence::repos::PgSearchRepo,
    state::AppState,
};

/// Query parameters for `GET /v1/workspaces/{ws}/search`.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchQueryParams {
    /// Free-text query with optional `key:value` filter tokens. Required.
    pub q: Option<String>,
    /// Restricts results by kind: `all` (default), `note` (documents), `task`.
    #[serde(rename = "type")]
    pub type_filter: Option<String>,
    /// Sort order: `relevance` (default) or `updated`.
    pub sort: Option<String>,
    /// Opaque pagination cursor returned by the previous response.
    pub cursor: Option<String>,
    /// Maximum results per page. Default 50, clamped to [1, 200].
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/search
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/search",
    tag = "search",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("q" = String, Query, description = "Search query (required). Supports key:value filter tokens."),
        ("type" = Option<String>, Query, description = "Kind filter: all (default) | note | task"),
        ("sort" = Option<String>, Query, description = "Sort order: relevance (default) | updated"),
        ("cursor" = Option<String>, Query, description = "Opaque pagination cursor; must match the sort of the issuing request"),
        ("limit" = Option<u32>, Query, description = "Page size, default 50, clamped to [1,200]"),
    ),
    responses(
        (status = 200, description = "Search results page", body = inline(Page<SearchHitDto>)),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or not a member"),
        (status = 422, description = "Invalid input: absent q, malformed cursor, or cursor/sort mismatch"),
    )
)]
pub(crate) async fn search(
    auth: Authorized<WorkspaceRes, ViewerMin>,
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200) as u64;

    let raw_q = params.q.as_deref().ok_or_else(|| ApiError::InvalidInput {
        message: "query parameter 'q' is required".into(),
    })?;

    let mut query = parse_query(raw_q);
    apply_param_overrides(&mut query, params.type_filter.as_deref(), params.sort.as_deref());

    let after = resolve_cursor(params.cursor.as_deref(), &query)?;

    if query.warnings.contains(&SearchWarning::TaskFilterOnNotes) {
        return Ok(Json(Page::<SearchHitDto>::empty()).into_response());
    }

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let principal = auth.principal;

    let repo = PgSearchRepo::new((*state.db).clone());
    let hits = repo
        .search(&ctx, &principal, &query, limit + 1, after)
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
                SearchSort::UpdatedDesc => {
                    ApiSortKey::Updated(h.updated_at.timestamp_micros())
                }
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
// Internal helpers
// ---------------------------------------------------------------------------

fn apply_param_overrides(query: &mut SearchQuery, type_param: Option<&str>, sort_param: Option<&str>) {
    if let Some(t) = type_param {
        query.type_filter = match t.to_ascii_lowercase().as_str() {
            "note" | "notes" | "document" | "documents" => TypeFilter::Documents,
            "task" | "tasks" => TypeFilter::Tasks,
            _ => TypeFilter::All,
        };
    }

    if let Some(s) = sort_param {
        query.sort = match s.to_ascii_lowercase().as_str() {
            "updated" => SearchSort::UpdatedDesc,
            _ => SearchSort::Relevance,
        };
    }

    let task_only_present = query.filters.iter().any(|f| {
        matches!(
            f,
            atlas_domain::search::SearchFilter::Status(_)
                | atlas_domain::search::SearchFilter::Priority(_)
                | atlas_domain::search::SearchFilter::Assignee(_)
        )
    });
    if task_only_present
        && query.type_filter == TypeFilter::Documents
        && !query.warnings.contains(&SearchWarning::TaskFilterOnNotes)
    {
        query.warnings.push(SearchWarning::TaskFilterOnNotes);
    }
}

fn resolve_cursor(
    raw: Option<&str>,
    query: &SearchQuery,
) -> Result<Option<SearchAfter>, ApiError> {
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

fn hit_to_dto(hit: atlas_domain::search::SearchHit) -> SearchHitDto {
    SearchHitDto {
        id: hit.id,
        kind: match hit.kind {
            SearchKind::Document => SearchKindDto::Document,
            SearchKind::Task => SearchKindDto::Task,
        },
        readable_id: hit.readable_id,
        title: hit.title,
        snippet: hit.snippet,
        score: hit.score,
        updated_at: hit.updated_at,
        project_slug: hit.project_slug,
    }
}

fn principal_to_actor(principal: &atlas_domain::permissions::Principal) -> Actor {
    match principal {
        atlas_domain::permissions::Principal::User(uid) => Actor::User(*uid),
        atlas_domain::permissions::Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    }
}
