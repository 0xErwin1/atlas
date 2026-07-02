//! Shared helpers for building comment DTOs, used by both the task-owned and
//! document-owned comment routes. Comments are polymorphic (`CommentOwner`), so
//! the owner is threaded through here rather than read off the row, and the DTO
//! carries whichever of `task_id` / `document_id` identifies the owner.

use std::collections::HashMap;

use atlas_api::dtos::{boards_tasks::CommentDto, documents::ActorDto};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::comments::{Comment, CommentOwner},
    ids::{ApiKeyId, UserId},
};

use crate::{
    error::ApiError,
    persistence::repos::{ApiKeyRepo, PgApiKeyRepo, PgUserRepo, UserRepo},
    routes::tasks::resolve_actor_dto,
    state::AppState,
};

/// Splits an owner into the `(task_id, document_id)` pair a `CommentDto` carries.
fn owner_dto_ids(owner: CommentOwner) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match owner {
        CommentOwner::Task(id) => (Some(id.0), None),
        CommentOwner::Document(id) => (None, Some(id.0)),
    }
}

/// Builds a `CommentDto` from a single `Comment`, resolving the author's display
/// name. The `owner` is supplied by the caller (from the already-authorized route
/// resource) rather than read off the row, since `Comment` is polymorphic.
pub(crate) async fn comment_to_dto(
    state: &AppState,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    c: Comment,
) -> CommentDto {
    let (task_id, document_id) = owner_dto_ids(owner);

    CommentDto {
        id: c.id.0,
        task_id,
        document_id,
        body: c.body,
        author: resolve_actor_dto(state, ctx, &c.created_by).await,
        created_at: c.created_at,
        updated_at: c.updated_at,
    }
}

/// Batch-resolves the distinct authors of a page of comments and builds their
/// DTOs in a fixed number of queries (the referenced users, and — only when a
/// comment was posted by an api key — the referenced api keys), so a comment page
/// never issues one query per row.
///
/// Api-key authors are resolved by id (unscoped), matching the create path's
/// `resolve_actor_dto`/`get_by_id` semantics. A workspace-grant/revocation filter
/// would drop the name of a global key (admitted via `is_global`, no grant row) or
/// a later-revoked key, so those authors would render as the generic fallback and
/// diverge from the create response.
pub(crate) async fn enrich_comment_entries(
    state: &AppState,
    owner: CommentOwner,
    comments: Vec<Comment>,
) -> Result<Vec<CommentDto>, ApiError> {
    let (task_id, document_id) = owner_dto_ids(owner);

    let mut user_ids: Vec<UserId> = Vec::new();
    let mut key_ids: Vec<ApiKeyId> = Vec::new();

    for c in &comments {
        match &c.created_by {
            Actor::User(uid) => user_ids.push(*uid),
            Actor::ApiKey(kid) => key_ids.push(*kid),
        }
    }

    user_ids.sort_by_key(|u| u.0);
    user_ids.dedup_by_key(|u| u.0);

    key_ids.sort_by_key(|k| k.0);
    key_ids.dedup_by_key(|k| k.0);

    let user_names: HashMap<uuid::Uuid, String> = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&user_ids)
    .await
    .map_err(ApiError::Domain)?
    .into_iter()
    .map(|u| (u.id.0, u.display_name))
    .collect();

    let key_info: HashMap<uuid::Uuid, (String, String)> = if key_ids.is_empty() {
        HashMap::new()
    } else {
        PgApiKeyRepo {
            conn: (*state.db).clone(),
        }
        .list_by_ids(&key_ids)
        .await
        .map_err(ApiError::Domain)?
        .into_iter()
        .map(|k| (k.id.0, (k.name, k.type_.as_str().to_string())))
        .collect()
    };

    let dtos = comments
        .into_iter()
        .map(|c| {
            let author = match &c.created_by {
                Actor::User(uid) => ActorDto {
                    r#type: "user".into(),
                    id: uid.0,
                    display_name: user_names.get(&uid.0).cloned(),
                    key_type: None,
                    account_status: None,
                },
                Actor::ApiKey(kid) => {
                    let info = key_info.get(&kid.0);
                    ActorDto {
                        r#type: "api_key".into(),
                        id: kid.0,
                        display_name: info.map(|(name, _)| name.clone()),
                        key_type: info.map(|(_, kt)| kt.clone()),
                        account_status: None,
                    }
                }
            };

            CommentDto {
                id: c.id.0,
                task_id,
                document_id,
                body: c.body,
                author,
                created_at: c.created_at,
                updated_at: c.updated_at,
            }
        })
        .collect();

    Ok(dtos)
}
