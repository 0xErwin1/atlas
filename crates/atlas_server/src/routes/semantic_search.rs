use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::{
    dtos::semantic_search::{
        SemanticSearchCursor, SemanticSearchHitDto, SemanticSearchKindDto, SemanticSearchSourceDto,
    },
    pagination::Page,
};
use atlas_domain::{
    permissions::CapabilityFamily,
    semantic_search::{
        ResourceKind, SemanticSearchAfter, SemanticSearchHit, SemanticSearchQuery,
        SemanticSearchRepo, SemanticSearchSource, SemanticSearchTypeFilter,
    },
};

use crate::{
    authz::WorkspaceAccess, error::ApiError, persistence::repos::PgSemanticSearchRepo,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub(crate) struct SemanticSearchQueryParams {
    pub q: Option<String>,
    #[serde(rename = "type")]
    pub type_filter: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/semantic-search",
    tag = "search",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("q" = String, Query, description = "Semantic search query (required)."),
        ("type" = Option<String>, Query, description = "Comma-separated kinds: document/note, task, all."),
        ("cursor" = Option<String>, Query, description = "Opaque semantic-search cursor."),
        ("limit" = Option<u32>, Query, description = "Page size, default 50, clamped to [1,200]"),
    ),
    responses(
        (status = 200, description = "Semantic search results page", body = inline(Page<SemanticSearchHitDto>)),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or principal has no access"),
        (status = 422, description = "Invalid input: absent q or malformed cursor"),
        (status = 503, description = "Semantic search is disabled or embedding provider is unavailable"),
    )
)]
pub(crate) async fn semantic_search(
    auth: WorkspaceAccess,
    State(state): State<AppState>,
    Query(params): Query<SemanticSearchQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200) as u64;
    let raw_q = params.q.as_deref().ok_or_else(|| ApiError::InvalidInput {
        message: "query parameter 'q' is required".into(),
    })?;

    if raw_q.trim().is_empty() {
        return Ok(Json(Page::<SemanticSearchHitDto>::empty()).into_response());
    }

    let provider =
        state
            .embedding_provider
            .clone()
            .ok_or_else(|| ApiError::ServiceUnavailable {
                message: "semantic search embeddings are disabled".to_owned(),
            })?;

    let type_filter = parse_type_filter(params.type_filter.as_deref());
    let after = resolve_cursor(params.cursor.as_deref())?;
    let (may_read_documents, may_read_tasks) = match &auth.read_scopes {
        Some(scopes) => (
            scopes.allows(CapabilityFamily::Docs),
            scopes.allows(CapabilityFamily::Tasks),
        ),
        None => (true, true),
    };

    let query = SemanticSearchQuery::new(
        auth.workspace.id,
        auth.principal,
        raw_q.to_owned(),
        type_filter,
        limit + 1,
        after,
        auth.bypass,
        may_read_documents,
        may_read_tasks,
    );

    let repo = PgSemanticSearchRepo::new((*state.db).clone(), provider);
    let hits = repo.search(&query).await.map_err(ApiError::Domain)?;

    let has_more = hits.len() as u64 > limit;
    let mut hits = hits;
    if has_more {
        hits.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        hits.last().map(|hit| {
            SemanticSearchCursor {
                similarity: hit.similarity,
                kind: kind_to_dto(hit.kind),
                id: hit.id,
            }
            .encode()
        })
    } else {
        None
    };
    let page = Page {
        items: hits.into_iter().map(hit_to_dto).collect(),
        next_cursor,
        has_more,
    };
    Ok(Json(page).into_response())
}

fn parse_type_filter(raw: Option<&str>) -> SemanticSearchTypeFilter {
    let Some(raw) = raw else {
        return SemanticSearchTypeFilter::all();
    };
    let mut filter = SemanticSearchTypeFilter {
        documents: false,
        tasks: false,
    };
    for part in raw.split(',').map(|part| part.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "all" | "" => return SemanticSearchTypeFilter::all(),
            "document" | "documents" | "note" | "notes" => filter.documents = true,
            "task" | "tasks" => filter.tasks = true,
            _ => {}
        }
    }
    if filter.documents || filter.tasks {
        filter
    } else {
        SemanticSearchTypeFilter::all()
    }
}

fn resolve_cursor(raw: Option<&str>) -> Result<Option<SemanticSearchAfter>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let cursor = SemanticSearchCursor::decode(raw).ok_or_else(|| ApiError::InvalidInput {
        message: "cursor is malformed or has an invalid format".into(),
    })?;
    Ok(Some(SemanticSearchAfter::new(
        cursor.similarity,
        match cursor.kind {
            SemanticSearchKindDto::Document => ResourceKind::Document,
            SemanticSearchKindDto::Task => ResourceKind::Task,
        },
        cursor.id,
    )))
}

fn hit_to_dto(hit: SemanticSearchHit) -> SemanticSearchHitDto {
    SemanticSearchHitDto {
        id: hit.id,
        kind: kind_to_dto(hit.kind),
        readable_id: hit.readable_id,
        title: hit.title,
        project_slug: hit.project_slug,
        column_name: hit.column_name,
        similarity: hit.similarity,
        source: source_to_dto(hit.source),
        excerpt: hit.excerpt,
    }
}

fn kind_to_dto(kind: ResourceKind) -> SemanticSearchKindDto {
    match kind {
        ResourceKind::Document => SemanticSearchKindDto::Document,
        ResourceKind::Task => SemanticSearchKindDto::Task,
    }
}

fn source_to_dto(source: SemanticSearchSource) -> SemanticSearchSourceDto {
    match source {
        SemanticSearchSource::Title => SemanticSearchSourceDto::Title,
        SemanticSearchSource::Content => SemanticSearchSourceDto::Content,
        SemanticSearchSource::Comment => SemanticSearchSourceDto::Comment,
        SemanticSearchSource::AttachmentName => SemanticSearchSourceDto::AttachmentName,
        SemanticSearchSource::Checklist => SemanticSearchSourceDto::Checklist,
        SemanticSearchSource::Subtask => SemanticSearchSourceDto::Subtask,
        SemanticSearchSource::Aggregate => SemanticSearchSourceDto::Aggregate,
    }
}
