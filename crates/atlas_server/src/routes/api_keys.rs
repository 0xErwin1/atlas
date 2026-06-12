use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct RevokeKeyPath {
    #[allow(dead_code)]
    pub(crate) ws: String,
    pub(crate) key_id: uuid::Uuid,
}

use atlas_api::{
    dtos::{ApiKeyCreated, ApiKeyDto, CreateApiKeyRequest},
    pagination::{Cursor, Page},
};
use atlas_domain::{Actor, WorkspaceCtx};

use crate::{
    auth::tokens::{generate_api_key, hash_token},
    authz::{AdminMin, Authorized, authorized::WorkspaceRes},
    error::ApiError,
    persistence::repos::{ApiKeyRepo, NewApiKey, PgApiKeyRepo},
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/api-keys",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "API key created (secret shown once)", body = ApiKeyCreated),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn create_api_key(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let secret = generate_api_key();
    let token_hash = hash_token(&secret);

    let actor = match &auth.principal {
        atlas_domain::permissions::Principal::User(uid) => Actor::User(*uid),
        atlas_domain::permissions::Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    };
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let key = repo
        .create(
            &ctx,
            NewApiKey {
                name: body.name,
                token_hash,
            },
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreated {
            id: key.id.0,
            name: key.name,
            secret,
            expires_at: key.expires_at,
            created_at: key.created_at,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/api-keys",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated API key list"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_api_keys(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ApiKeyDto>>, ApiError> {
    let actor = match &auth.principal {
        atlas_domain::permissions::Principal::User(uid) => Actor::User(*uid),
        atlas_domain::permissions::Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    };
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let all_keys = repo.list(&ctx).await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let mut filtered: Vec<_> = all_keys
        .into_iter()
        .filter(|k| after_id.is_none_or(|cursor| k.id.0 > cursor))
        .collect();

    let has_more = filtered.len() > limit as usize;
    if has_more {
        filtered.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        filtered.last().map(|k| Cursor(k.id.0))
    } else {
        None
    };

    let dtos: Vec<ApiKeyDto> = filtered
        .iter()
        .map(|k| ApiKeyDto {
            id: k.id.0,
            name: k.name.clone(),
            expires_at: k.expires_at,
            last_used_at: k.last_used_at,
            revoked_at: k.revoked_at,
            created_at: k.created_at,
        })
        .collect();

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/api-keys/{key_id}/revoke",
    tag = "api-keys",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "API key revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn revoke_api_key(
    auth: Authorized<WorkspaceRes, AdminMin>,
    State(state): State<AppState>,
    Path(params): Path<RevokeKeyPath>,
) -> Result<StatusCode, ApiError> {
    let key_id = atlas_domain::ids::ApiKeyId(params.key_id);

    let actor = match &auth.principal {
        atlas_domain::permissions::Principal::User(uid) => Actor::User(*uid),
        atlas_domain::permissions::Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    };
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    repo.revoke(&ctx, key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
}
