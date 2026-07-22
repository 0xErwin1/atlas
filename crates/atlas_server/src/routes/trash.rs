use atlas_api::{
    dtos::lifecycle::{RestoreTrashItemRequest, TrashItemDto, TrashKindDto},
    pagination::{Page, SearchCursor, SortKey},
};
use atlas_domain::{entities::lifecycle::TrashKind, ids::WorkspaceId};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{authz::RequireUserAdmin, error::ApiError, services::TrashService, state::AppState};

#[derive(Deserialize)]
pub(crate) struct TrashQuery {
    workspace_id: Option<uuid::Uuid>,
    kind: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[utoipa::path(get, path = "/api/admin/trash", tag = "trash", security(("bearer_auth" = [])), params(("workspace_id" = Option<uuid::Uuid>, Query), ("kind" = Option<String>, Query), ("cursor" = Option<String>, Query), ("limit" = Option<u32>, Query)), responses((status = 200, body = Page<TrashItemDto>), (status = 400), (status = 401), (status = 403)))]
pub(crate) async fn list_trash(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Query(query): Query<TrashQuery>,
) -> Result<Json<Page<TrashItemDto>>, ApiError> {
    let kind = parse_kind(query.kind.as_deref())?;
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as u64;
    let service = TrashService::new((*state.db).clone());
    let mut items = service
        .list(query.workspace_id.map(WorkspaceId), kind, cursor, limit + 1)
        .await
        .map_err(ApiError::Domain)?;
    let has_more = items.len() > limit as usize;
    if has_more {
        items.truncate(limit as usize);
    }
    let next_cursor = items.last().map(|item| SearchCursor {
        key: SortKey::Updated(item.deleted_at.timestamp_micros()),
        id: item.target_id,
    });
    Ok(Json(Page::new_search(
        items.into_iter().map(to_dto).collect(),
        next_cursor,
        has_more,
    )))
}

#[utoipa::path(post, path = "/api/admin/trash/restore", tag = "trash", security(("bearer_auth" = [])), request_body = RestoreTrashItemRequest, responses((status = 204, description = "Restored or already live"), (status = 401), (status = 403), (status = 404), (status = 409)))]
pub(crate) async fn restore_trash(
    admin: RequireUserAdmin,
    State(state): State<AppState>,
    Json(request): Json<RestoreTrashItemRequest>,
) -> Result<StatusCode, ApiError> {
    let kind = from_dto(request.kind);
    TrashService::new((*state.db).clone())
        .restore(admin.user.id, kind, request.target_id)
        .await
        .map_err(ApiError::Domain)?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_kind(value: Option<&str>) -> Result<Option<TrashKind>, ApiError> {
    value
        .map(|value| {
            value.parse().map_err(|_| ApiError::BadRequest {
                message: "kind must be project, folder, document, comment, or attachment".into(),
            })
        })
        .transpose()
}
fn decode_cursor(value: &str) -> Result<(chrono::DateTime<chrono::Utc>, uuid::Uuid), ApiError> {
    let cursor = SearchCursor::decode(value).ok_or_else(|| ApiError::BadRequest {
        message: "invalid trash cursor".into(),
    })?;
    let SortKey::Updated(micros) = cursor.key else {
        return Err(ApiError::BadRequest {
            message: "cursor is not compatible with trash listing".into(),
        });
    };
    let timestamp =
        chrono::DateTime::from_timestamp_micros(micros).ok_or_else(|| ApiError::BadRequest {
            message: "invalid trash cursor timestamp".into(),
        })?;
    Ok((timestamp, cursor.id))
}
fn from_dto(kind: TrashKindDto) -> TrashKind {
    match kind {
        TrashKindDto::Project => TrashKind::Project,
        TrashKindDto::Folder => TrashKind::Folder,
        TrashKindDto::Document => TrashKind::Document,
        TrashKindDto::Comment => TrashKind::Comment,
        TrashKindDto::Attachment => TrashKind::Attachment,
    }
}
fn to_dto(item: atlas_domain::entities::lifecycle::TrashItem) -> TrashItemDto {
    TrashItemDto {
        workspace_id: item.workspace_id.0,
        kind: match item.kind {
            TrashKind::Project => TrashKindDto::Project,
            TrashKind::Folder => TrashKindDto::Folder,
            TrashKind::Document => TrashKindDto::Document,
            TrashKind::Comment => TrashKindDto::Comment,
            TrashKind::Attachment => TrashKindDto::Attachment,
        },
        target_id: item.target_id,
        deleted_at: item.deleted_at,
    }
}
