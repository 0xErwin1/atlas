//! Shared helpers for building comment DTOs, used by both the task-owned and
//! document-owned comment routes. Comments are polymorphic (`CommentOwner`), so
//! the owner is threaded through here rather than read off the row, and the DTO
//! carries whichever of `task_id` / `document_id` identifies the owner.

use std::collections::HashMap;

use atlas_api::dtos::{
    boards_tasks::{
        CommentDto, CommentFeedEntryDto, CommentLinkProjectionDto, CommentLinkTargetDto,
    },
    documents::ActorDto,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::comments::{
        Comment, CommentFeedCursor, CommentFeedEntry, CommentLinkTarget, CommentOwner,
    },
    ids::{ApiKeyId, CommentId, UserId},
    ports::comments::CommentLinkRepo,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    authz::batch_authorization::{
        BatchAuthorizationService, PgBatchAuthorizationSource, ProjectionAuthContext,
        ProjectionSubject,
    },
    error::ApiError,
    persistence::repos::{ApiKeyRepo, PgApiKeyRepo, PgCommentLinkRepo, PgUserRepo, UserRepo},
    routes::tasks::resolve_actor_dto,
    state::AppState,
};

const UNAVAILABLE_LABEL: &str = "Recurso no disponible";

#[derive(Serialize, Deserialize)]
struct FeedCursorWire {
    created_at: DateTime<Utc>,
    id: uuid::Uuid,
}

pub(crate) fn decode_feed_cursor(
    cursor: Option<&str>,
) -> Result<Option<CommentFeedCursor>, ApiError> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };

    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| ApiError::Internal {
            message: "invalid comment feed cursor".into(),
        })?;
    let cursor: FeedCursorWire =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::Internal {
            message: "invalid comment feed cursor".into(),
        })?;

    Ok(Some(CommentFeedCursor {
        created_at: cursor.created_at,
        id: cursor.id,
    }))
}

pub(crate) fn encode_feed_cursor(cursor: CommentFeedCursor) -> Result<String, ApiError> {
    let body = serde_json::to_vec(&FeedCursorWire {
        created_at: cursor.created_at,
        id: cursor.id,
    })
    .map_err(|error| ApiError::Internal {
        message: error.to_string(),
    })?;

    Ok(URL_SAFE_NO_PAD.encode(body))
}

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

/// Projects a merged comment feed only after batch authorization has resolved every
/// derived target for the request-bound principal.
pub(crate) async fn project_comment_feed(
    state: &AppState,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    projection_context: &ProjectionAuthContext,
    after: Option<CommentFeedCursor>,
    limit: u64,
) -> Result<(Vec<CommentFeedEntryDto>, Option<String>, bool), ApiError> {
    let repo = PgCommentLinkRepo::new((*state.db).clone());
    let page = repo
        .feed_for_owner(ctx, owner, after, limit)
        .await
        .map_err(ApiError::Domain)?;

    let comment_ids = page
        .entries
        .iter()
        .filter_map(|entry| match entry {
            CommentFeedEntry::Comment(comment) => Some(comment.id),
            CommentFeedEntry::Event(_) => None,
        })
        .collect::<Vec<_>>();
    let links = repo
        .links_for_comments(ctx, &comment_ids)
        .await
        .map_err(ApiError::Domain)?;

    let mut links_by_comment: HashMap<CommentId, Vec<CommentLinkTarget>> = HashMap::new();
    let mut subjects = Vec::new();
    for link in links {
        subjects.push(subject_for_target(link.target));
        links_by_comment
            .entry(link.comment_id)
            .or_default()
            .push(link.target);
    }
    for entry in &page.entries {
        if let CommentFeedEntry::Event(event) = entry
            && let Some(target) = event.target
        {
            subjects.push(subject_for_target(target));
        }
    }

    let decisions = authorize_targets(state, projection_context, &subjects).await?;
    let target_visibility = subjects
        .into_iter()
        .zip(decisions)
        .collect::<HashMap<_, _>>();
    let actors = load_event_actors(state, &page.entries).await?;

    let next_cursor = if page.has_more {
        entries_cursor(&page.entries).transpose()?
    } else {
        None
    };

    let mut entries = Vec::with_capacity(page.entries.len());
    for entry in page.entries {
        match entry {
            CommentFeedEntry::Comment(comment) => {
                let comment_dto = comment_to_dto(state, ctx, owner, comment.clone()).await;
                let links = links_by_comment
                    .remove(&comment.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|target| CommentLinkProjectionDto {
                        target: project_target(target, &target_visibility),
                    })
                    .collect();
                entries.push(CommentFeedEntryDto::Comment {
                    comment: comment_dto,
                    links,
                });
            }
            CommentFeedEntry::Event(event) => {
                let target = event
                    .target
                    .map(|target| project_target(target, &target_visibility));
                entries.push(CommentFeedEntryDto::Event {
                    id: event.id.0,
                    kind: event_kind(event.kind).into(),
                    comment_id: event.comment_id.0,
                    target,
                    actor: actors.get(&actor_key(event.actor)).cloned(),
                    created_at: event.created_at,
                });
            }
        }
    }

    Ok((entries, next_cursor, page.has_more))
}

fn subject_for_target(target: CommentLinkTarget) -> ProjectionSubject {
    match target {
        CommentLinkTarget::Document(id) => ProjectionSubject::Document(id.0),
        CommentLinkTarget::Task(id) => ProjectionSubject::Task(id.0),
        CommentLinkTarget::Attachment(id) => ProjectionSubject::Attachment(id.0),
    }
}

async fn authorize_targets(
    state: &AppState,
    context: &ProjectionAuthContext,
    subjects: &[ProjectionSubject],
) -> Result<Vec<bool>, ApiError> {
    if subjects.is_empty() {
        return Ok(Vec::new());
    }

    let service =
        BatchAuthorizationService::new(PgBatchAuthorizationSource::new((*state.db).clone()));
    service
        .authorize(context, subjects)
        .await
        .map_err(ApiError::Domain)
}

fn project_target(
    target: CommentLinkTarget,
    visibility: &HashMap<ProjectionSubject, bool>,
) -> CommentLinkTargetDto {
    if visibility
        .get(&subject_for_target(target))
        .copied()
        .unwrap_or(false)
    {
        let (r#type, id) = match target {
            CommentLinkTarget::Document(id) => ("document", id.0),
            CommentLinkTarget::Task(id) => ("task", id.0),
            CommentLinkTarget::Attachment(id) => ("attachment", id.0),
        };
        CommentLinkTargetDto::Available {
            r#type: r#type.into(),
            id,
        }
    } else {
        CommentLinkTargetDto::Unavailable {
            label: UNAVAILABLE_LABEL.into(),
        }
    }
}

fn event_kind(kind: atlas_domain::entities::comments::CommentLinkEventKind) -> &'static str {
    match kind {
        atlas_domain::entities::comments::CommentLinkEventKind::LinkAdded => "link_added",
        atlas_domain::entities::comments::CommentLinkEventKind::LinkRemoved => "link_removed",
        atlas_domain::entities::comments::CommentLinkEventKind::CommentDeleted => "comment_deleted",
    }
}

fn entries_cursor(entries: &[CommentFeedEntry]) -> Option<Result<String, ApiError>> {
    entries
        .last()
        .map(|entry| encode_feed_cursor(entry.cursor()))
}

async fn load_event_actors(
    state: &AppState,
    entries: &[CommentFeedEntry],
) -> Result<HashMap<(bool, uuid::Uuid), ActorDto>, ApiError> {
    let mut user_ids = Vec::new();
    let mut key_ids = Vec::new();
    for entry in entries {
        if let CommentFeedEntry::Event(event) = entry {
            match event.actor {
                Actor::User(id) => user_ids.push(id),
                Actor::ApiKey(id) => key_ids.push(id),
            }
        }
    }
    user_ids.sort_by_key(|id| id.0);
    user_ids.dedup();
    key_ids.sort_by_key(|id| id.0);
    key_ids.dedup();

    let users = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&user_ids)
    .await
    .map_err(ApiError::Domain)?;
    let keys = PgApiKeyRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&key_ids)
    .await
    .map_err(ApiError::Domain)?;

    let mut actors = HashMap::new();
    for user in users {
        if user.disabled_at.is_none() {
            actors.insert(
                actor_key(Actor::User(user.id)),
                ActorDto {
                    r#type: "user".into(),
                    id: user.id.0,
                    display_name: Some(user.display_name),
                    key_type: None,
                    account_status: None,
                },
            );
        }
    }
    for key in keys {
        if key.revoked_at.is_none() {
            actors.insert(
                actor_key(Actor::ApiKey(key.id)),
                ActorDto {
                    r#type: "api_key".into(),
                    id: key.id.0,
                    display_name: Some(key.name),
                    key_type: Some(key.type_.as_str().into()),
                    account_status: None,
                },
            );
        }
    }
    Ok(actors)
}

fn actor_key(actor: Actor) -> (bool, uuid::Uuid) {
    match actor {
        Actor::User(id) => (true, id.0),
        Actor::ApiKey(id) => (false, id.0),
    }
}
