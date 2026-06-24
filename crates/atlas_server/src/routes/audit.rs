use axum::{Json, extract::Query, extract::State};
use serde::Deserialize;
use std::collections::HashMap;

use atlas_api::{
    dtos::{audit::AuditEntryDto, documents::ActorDto},
    pagination::{Page, SearchCursor, SortKey},
};
use atlas_domain::{
    Actor,
    entities::security_audit::{AuditCursor, AuditFilters, SecurityAuditEvent},
    entities::task_views::ActorTypeFilter,
    ids::{ApiKeyId, UserId},
};

use crate::{
    authz::{RequireUserAdmin, WorkspaceOwnerOrAdmin},
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, PgApiKeyRepo, PgSecurityAuditRepo, PgUserRepo, SecurityAuditRepo, UserRepo,
    },
    routes::account_status,
    state::AppState,
};

/// Query parameters shared by both audit endpoints.
#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    pub actor: Option<String>,
    pub action: Option<String>,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/audit
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/audit",
    tag = "audit",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("actor" = Option<String>, Query, description = "Actor type filter: 'user' or 'api_key'"),
        ("action" = Option<String>, Query, description = "Filter by action verb (e.g. 'membership.role_changed')"),
        ("from" = Option<String>, Query, description = "Lower bound on created_at (ISO 8601)"),
        ("to" = Option<String>, Query, description = "Upper bound on created_at (ISO 8601)"),
        ("cursor" = Option<String>, Query, description = "Keyset pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200, default 50)"),
    ),
    responses(
        (status = 200, description = "Workspace security audit log", body = Page<AuditEntryDto>),
        (status = 400, description = "Invalid query parameter"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not workspace owner or admin"),
        (status = 404, description = "Workspace not found"),
    )
)]
pub(crate) async fn list_workspace_audit(
    owner_or_admin: WorkspaceOwnerOrAdmin,
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Page<AuditEntryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;

    let filters = build_filters(&q)?;
    let cursor = decode_cursor(q.cursor.as_deref())?;

    let repo = PgSecurityAuditRepo::new((*state.db).clone());
    let mut rows = repo
        .list_for_workspace(owner_or_admin.workspace.id, &filters, cursor, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        rows.last().map(encode_cursor)
    } else {
        None
    };

    let dtos = enrich_audit_entries(&state, rows).await?;
    Ok(Json(Page::new_search(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /v1/admin/audit
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/admin/audit",
    tag = "audit",
    security(("bearer_auth" = [])),
    params(
        ("actor" = Option<String>, Query, description = "Actor type filter: 'user' or 'api_key'"),
        ("action" = Option<String>, Query, description = "Filter by action verb"),
        ("from" = Option<String>, Query, description = "Lower bound on created_at (ISO 8601)"),
        ("to" = Option<String>, Query, description = "Upper bound on created_at (ISO 8601)"),
        ("cursor" = Option<String>, Query, description = "Keyset pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200, default 50)"),
    ),
    responses(
        (status = 200, description = "Platform security audit log (workspace_id IS NULL)", body = Page<AuditEntryDto>),
        (status = 400, description = "Invalid query parameter"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not root or system_admin"),
    )
)]
pub(crate) async fn list_platform_audit(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Page<AuditEntryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;

    let filters = build_filters(&q)?;
    let cursor = decode_cursor(q.cursor.as_deref())?;

    let repo = PgSecurityAuditRepo::new((*state.db).clone());
    let mut rows = repo
        .list_platform(&filters, cursor, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        rows.last().map(encode_cursor)
    } else {
        None
    };

    let dtos = enrich_audit_entries(&state, rows).await?;
    Ok(Json(Page::new_search(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn build_filters(q: &AuditQuery) -> Result<AuditFilters, ApiError> {
    let actor_type = if let Some(ref actor) = q.actor {
        match actor.as_str() {
            "user" => Some(ActorTypeFilter::User),
            "api_key" => Some(ActorTypeFilter::ApiKey),
            other => {
                return Err(ApiError::InvalidInput {
                    message: format!("invalid actor filter '{other}'; must be 'user' or 'api_key'"),
                });
            }
        }
    } else {
        None
    };

    Ok(AuditFilters {
        actor_user_id: None,
        actor_type,
        action: q.action.clone(),
        from: q.from,
        to: q.to,
    })
}

fn decode_cursor(raw: Option<&str>) -> Result<Option<AuditCursor>, ApiError> {
    let Some(s) = raw else {
        return Ok(None);
    };

    let sc = SearchCursor::decode(s).ok_or_else(|| ApiError::InvalidInput {
        message: "invalid cursor".to_string(),
    })?;

    let micros = match sc.key {
        SortKey::Updated(m) => m,
        SortKey::Relevance(_) => {
            return Err(ApiError::InvalidInput {
                message: "cursor is not compatible with audit listing".to_string(),
            });
        }
    };

    let ts =
        chrono::DateTime::from_timestamp_micros(micros).ok_or_else(|| ApiError::InvalidInput {
            message: "invalid cursor timestamp".to_string(),
        })?;

    Ok(Some(AuditCursor {
        created_at: ts,
        id: atlas_domain::SecurityAuditId(sc.id),
    }))
}

fn encode_cursor(row: &SecurityAuditEvent) -> SearchCursor {
    SearchCursor {
        key: SortKey::Updated(row.created_at.timestamp_micros()),
        id: row.id.0,
    }
}

/// Batch-loads user and api-key display information for a page of audit entries.
///
/// Collects distinct actor ids across the page, loads them in two queries
/// (users + api keys), and builds enriched `AuditEntryDto`s with display_name,
/// key_type, and account_status. `target_label` is resolved from the same loaded
/// maps when target_type is "user" or "api_key"; otherwise left as None.
async fn enrich_audit_entries(
    state: &AppState,
    rows: Vec<SecurityAuditEvent>,
) -> Result<Vec<AuditEntryDto>, ApiError> {
    let mut user_ids: Vec<UserId> = Vec::new();
    let mut key_ids: Vec<ApiKeyId> = Vec::new();

    let mut target_user_ids: Vec<UserId> = Vec::new();
    let mut target_key_ids: Vec<ApiKeyId> = Vec::new();

    for row in &rows {
        match &row.actor {
            Actor::User(uid) => user_ids.push(*uid),
            Actor::ApiKey(kid) => key_ids.push(*kid),
        }

        if let Some(tid) = row.target_id {
            match row.target_type.as_str() {
                "user" => target_user_ids.push(UserId(tid)),
                "api_key" => target_key_ids.push(ApiKeyId(tid)),
                _ => {}
            }
        }
    }

    user_ids.sort_by_key(|u| u.0);
    user_ids.dedup_by_key(|u| u.0);
    key_ids.sort_by_key(|k| k.0);
    key_ids.dedup_by_key(|k| k.0);

    target_user_ids.sort_by_key(|u| u.0);
    target_user_ids.dedup_by_key(|u| u.0);
    target_key_ids.sort_by_key(|k| k.0);
    target_key_ids.dedup_by_key(|k| k.0);

    let all_user_ids: Vec<UserId> = {
        let mut combined = user_ids.clone();
        for uid in &target_user_ids {
            if !combined.iter().any(|u| u.0 == uid.0) {
                combined.push(*uid);
            }
        }
        combined
    };

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let user_map: HashMap<uuid::Uuid, atlas_domain::entities::identity::User> = user_repo
        .list_by_ids(&all_user_ids)
        .await
        .map_err(ApiError::Domain)?
        .into_iter()
        .map(|u| (u.id.0, u))
        .collect();

    let key_repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let key_map: HashMap<uuid::Uuid, atlas_domain::entities::identity::ApiKey> =
        if !key_ids.is_empty() || !target_key_ids.is_empty() {
            let all_key_uuids: Vec<uuid::Uuid> = {
                let mut combined: Vec<uuid::Uuid> = key_ids.iter().map(|k| k.0).collect();
                for kid in &target_key_ids {
                    if !combined.contains(&kid.0) {
                        combined.push(kid.0);
                    }
                }
                combined
            };

            let mut map = HashMap::new();
            for kid_uuid in all_key_uuids {
                if let Ok(Some(k)) = key_repo.get_by_id(ApiKeyId(kid_uuid)).await {
                    map.insert(kid_uuid, k);
                }
            }
            map
        } else {
            HashMap::new()
        };

    let dtos = rows
        .into_iter()
        .map(|row| {
            let actor_dto = match &row.actor {
                Actor::User(uid) => {
                    let user = user_map.get(&uid.0);
                    ActorDto {
                        r#type: "user".into(),
                        id: uid.0,
                        display_name: user.map(|u| u.display_name.clone()),
                        key_type: None,
                        account_status: user
                            .map(|u| account_status(u.disabled_at, u.activated_at).to_string()),
                    }
                }
                Actor::ApiKey(kid) => {
                    let key = key_map.get(&kid.0);
                    ActorDto {
                        r#type: "api_key".into(),
                        id: kid.0,
                        display_name: key.map(|k| k.name.clone()),
                        key_type: key.map(|k| k.type_.as_str().to_string()),
                        account_status: None,
                    }
                }
            };

            let target_label = match row.target_type.as_str() {
                "user" => row
                    .target_id
                    .and_then(|tid| user_map.get(&tid))
                    .map(|u| u.display_name.clone()),
                "api_key" => row
                    .target_id
                    .and_then(|tid| key_map.get(&tid))
                    .map(|k| k.name.clone()),
                _ => None,
            };

            AuditEntryDto {
                id: row.id.0,
                workspace_id: row.workspace_id.map(|w| w.0),
                actor: actor_dto,
                action: row.action,
                target_type: row.target_type,
                target_id: row.target_id,
                target_label,
                metadata: row.metadata,
                created_at: row.created_at,
            }
        })
        .collect();

    Ok(dtos)
}
