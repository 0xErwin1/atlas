use atlas_api::{
    dtos::lifecycle::{
        PurgeStatusDto, PurgeStatusDtoResponse, PurgeTrashItemRequest, RestoreTrashItemRequest,
        TrashItemDto, TrashKindDto,
    },
    pagination::{Page, SearchCursor, SortKey},
};
use atlas_domain::{entities::lifecycle::TrashKind, ids::WorkspaceId};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::{
    authz::RequireUserAdmin, error::ApiError, persistence::repos::PgPurgeOperationRepo,
    services::TrashService, state::AppState,
};

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

#[utoipa::path(post, path = "/api/admin/trash/purge", tag = "trash", security(("bearer_auth" = [])), request_body = PurgeTrashItemRequest, responses((status = 202, body = PurgeStatusDtoResponse, description = "Database purge committed; cleanup is pending"), (status = 204, description = "Purge cleanup already complete"), (status = 400), (status = 401), (status = 403), (status = 404)))]
pub(crate) async fn purge_trash(
    admin: RequireUserAdmin,
    State(state): State<AppState>,
    Json(request): Json<PurgeTrashItemRequest>,
) -> Result<Response, ApiError> {
    if !request.confirm {
        return Err(ApiError::BadRequest {
            message: "purge requires confirm: true".into(),
        });
    }

    let service = TrashService::new((*state.db).clone());
    let operation = service
        .purge(admin.user.id, from_dto(request.kind), request.target_id)
        .await
        .map_err(ApiError::Domain)?;
    let operation = service
        .cleanup(operation.id, state.attachments.as_ref())
        .await
        .map_err(ApiError::Domain)?;

    if operation.status == atlas_domain::entities::lifecycle::PurgeStatus::Complete {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    Ok((StatusCode::ACCEPTED, Json(to_purge_dto(operation))).into_response())
}

#[utoipa::path(get, path = "/api/admin/trash/purges/{operation_id}", tag = "trash", security(("bearer_auth" = [])), params(("operation_id" = uuid::Uuid, Path)), responses((status = 200, body = PurgeStatusDtoResponse), (status = 401), (status = 403), (status = 404)))]
pub(crate) async fn get_purge_status(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Path(operation_id): Path<uuid::Uuid>,
) -> Result<Json<PurgeStatusDtoResponse>, ApiError> {
    let operation = PgPurgeOperationRepo
        .find_by_id_in(
            state.db.as_ref(),
            atlas_domain::ids::PurgeOperationId(operation_id),
        )
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::Domain(atlas_domain::DomainError::NotFound {
            entity: "purge_operation",
            id: operation_id,
        }))?;
    Ok(Json(to_purge_dto(operation)))
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

fn to_purge_dto(
    operation: atlas_domain::entities::lifecycle::PurgeOperation,
) -> PurgeStatusDtoResponse {
    PurgeStatusDtoResponse {
        operation_id: operation.id.0,
        kind: match operation.target.kind {
            TrashKind::Project => TrashKindDto::Project,
            TrashKind::Folder => TrashKindDto::Folder,
            TrashKind::Document => TrashKindDto::Document,
            TrashKind::Comment => TrashKindDto::Comment,
            TrashKind::Attachment => TrashKindDto::Attachment,
        },
        target_id: operation.target.target_id,
        status: match operation.status {
            atlas_domain::entities::lifecycle::PurgeStatus::DbCommitted => {
                PurgeStatusDto::DbCommitted
            }
            atlas_domain::entities::lifecycle::PurgeStatus::CleanupPending => {
                PurgeStatusDto::CleanupPending
            }
            atlas_domain::entities::lifecycle::PurgeStatus::CleanupFailed => {
                PurgeStatusDto::CleanupFailed
            }
            atlas_domain::entities::lifecycle::PurgeStatus::Complete => PurgeStatusDto::Complete,
        },
        attempts: operation.attempts,
    }
}
