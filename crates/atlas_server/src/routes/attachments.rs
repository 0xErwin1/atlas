//! Workspace-wide attachment surface.
//!
//! Every file uploaded anywhere in a workspace — on a note, on a task, or on a
//! comment of either — is listed here with its metadata and its owner, and can
//! be renamed from here. Download and delete keep living on the per-attachment
//! routes in [`super::documents`], which this module supplies the owner-agnostic
//! authorization for.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use atlas_api::{
    dtos::documents::{AttachmentOwnerDto, RenameAttachmentRequest, WorkspaceAttachmentDto},
    pagination::{Cursor, Page},
};
use atlas_domain::{
    WorkspaceCtx,
    entities::documents::{
        Attachment, AttachmentOwner, AttachmentOwnerKind, AttachmentOwnerRef, WorkspaceAttachment,
        WorkspaceAttachmentQuery,
    },
    ids::{AttachmentId, CommentId, DocumentId, TaskId},
    permissions::{Capability, CapabilityAction, CapabilityFamily, ResourceRole},
    ports::{
        boards_tasks::TaskRepo,
        comments::CommentRepo as CommentRepoPort,
        documents::{AttachmentRepo, DocumentRepo, WorkspaceAttachmentRepo},
    },
};

use crate::{
    authz::{
        WorkspaceAccess, WorkspaceMember, build_board_chain, build_document_chain,
        enforce_api_key_scope, resolve_effective_role,
    },
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, PgApiKeyRepo, PgAttachmentRepo, PgCommentRepo, PgDocumentRepo, PgTaskRepo,
        PgUserRepo, PgWorkspaceAttachmentRepo, UserRepo,
    },
    routes::{
        documents::{member_to_actor, member_to_principal},
        validation::{validate_name, validate_upload_extension},
    },
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    /// Case-insensitive substring match on the file name.
    q: Option<String>,
    /// Owner kind: `document` or `task`. Absent lists both.
    owner: Option<String>,
    /// Content-type prefix, e.g. `image/`.
    content_type: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct AttachmentPath {
    #[allow(dead_code)]
    ws: String,
    attachment_id: uuid::Uuid,
}

fn parse_owner_kind(raw: Option<&str>) -> Result<Option<AttachmentOwnerKind>, ApiError> {
    match raw.map(str::trim) {
        None | Some("") | Some("all") => Ok(None),
        Some("document") => Ok(Some(AttachmentOwnerKind::Document)),
        Some("task") => Ok(Some(AttachmentOwnerKind::Task)),
        Some(other) => Err(ApiError::InvalidInput {
            message: format!("unknown owner filter '{other}'; use document, task or all"),
        }),
    }
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/attachments
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/attachments",
    operation_id = "list_workspace_attachments",
    tag = "attachments",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size, default 50, clamped to [1,200]"),
        ("q" = Option<String>, Query, description = "Case-insensitive substring match on the file name"),
        ("owner" = Option<String>, Query, description = "Owner kind: document | task | all (default)"),
        ("content_type" = Option<String>, Query, description = "Content-type prefix, e.g. image/"),
    ),
    responses(
        (status = 200, description = "Attachment page", body = inline(Page<WorkspaceAttachmentDto>)),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or principal has no access"),
        (status = 422, description = "Unknown owner filter"),
    )
)]
/// Lists every attachment in the workspace the principal may see.
///
/// Rows are permission-filtered in SQL against the resource that owns them, so
/// the page limit applies only to visible rows. Attachments still held by an
/// unpublished comment draft are never listed.
pub(crate) async fn list_workspace_attachments(
    auth: WorkspaceAccess,
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<Json<Page<WorkspaceAttachmentDto>>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200) as u64;
    let owner_kind = parse_owner_kind(params.owner.as_deref())?;

    let ctx = WorkspaceCtx::new(
        auth.workspace.id,
        crate::routes::documents::principal_to_actor(&auth.principal),
    );

    let query = WorkspaceAttachmentQuery {
        file_name: params.q,
        owner_kind,
        content_type_prefix: params.content_type,
        after: params
            .cursor
            .as_deref()
            .and_then(Cursor::decode)
            .map(|c| AttachmentId(c.0)),
        limit: limit + 1,
    };

    let mut items = PgWorkspaceAttachmentRepo::new((*state.db).clone())
        .list(&ctx, &auth.principal, &query, auth.bypass)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = items.len() as u64 > limit;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        items.last().map(|item| Cursor(item.attachment.id.0))
    } else {
        None
    };

    let mut dtos: Vec<WorkspaceAttachmentDto> = items
        .into_iter()
        .map(|item| workspace_attachment_to_dto(&auth.workspace.slug, item))
        .collect();

    hydrate_uploaders(&state, &mut dtos).await?;

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

/// Fills in each uploader's display name with two batched lookups.
///
/// The rows carry only the actor's id, and a listing that shows who uploaded a
/// file cannot issue one query per row.
async fn hydrate_uploaders(
    state: &AppState,
    dtos: &mut [WorkspaceAttachmentDto],
) -> Result<(), ApiError> {
    use std::collections::HashMap;

    let user_ids: Vec<atlas_domain::ids::UserId> = dtos
        .iter()
        .filter_map(|dto| dto.actor.as_ref())
        .filter(|actor| actor.r#type == "user")
        .map(|actor| atlas_domain::ids::UserId(actor.id))
        .collect();

    let key_ids: Vec<atlas_domain::ids::ApiKeyId> = dtos
        .iter()
        .filter_map(|dto| dto.actor.as_ref())
        .filter(|actor| actor.r#type == "api_key")
        .map(|actor| atlas_domain::ids::ApiKeyId(actor.id))
        .collect();

    let users: HashMap<uuid::Uuid, String> = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&user_ids)
    .await
    .map_err(ApiError::Domain)?
    .into_iter()
    .map(|user| (user.id.0, user.display_name))
    .collect();

    let keys: HashMap<uuid::Uuid, (String, String)> = PgApiKeyRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&key_ids)
    .await
    .map_err(ApiError::Domain)?
    .into_iter()
    .map(|key| (key.id.0, (key.name, key.type_.as_str().to_string())))
    .collect();

    for dto in dtos {
        let Some(actor) = dto.actor.as_mut() else {
            continue;
        };

        if actor.r#type == "user" {
            actor.display_name = users.get(&actor.id).cloned();
        } else if let Some((name, key_type)) = keys.get(&actor.id) {
            actor.display_name = Some(name.clone());
            actor.key_type = Some(key_type.clone());
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/attachments/{attachment_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/attachments/{attachment_id}",
    operation_id = "rename_workspace_attachment",
    tag = "attachments",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("attachment_id" = String, Path, description = "Attachment UUID"),
    ),
    request_body = RenameAttachmentRequest,
    responses(
        (status = 200, description = "Attachment renamed", body = WorkspaceAttachmentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Attachment not found"),
        (status = 422, description = "Invalid file name, or the name is already taken here"),
    )
)]
/// Renames an attachment and rewrites every reference that addressed it by name.
///
/// A `[[file:…]]` wikilink is the only name-bound way to reference an
/// attachment — embedded images and links carry the attachment id — so the
/// rewrite is confined to the owner's own body, and it goes through the normal
/// content-save path so the note keeps its revision history and its link rows.
pub(crate) async fn rename_attachment(
    member: WorkspaceMember,
    Path(path): Path<AttachmentPath>,
    State(state): State<AppState>,
    Json(body): Json<RenameAttachmentRequest>,
) -> Result<Json<WorkspaceAttachmentDto>, ApiError> {
    validate_name("file_name", &body.file_name)?;
    let file_name = body.file_name.trim().to_owned();
    validate_upload_extension(&file_name, state.upload_allowed_extensions.as_deref())?;

    let attachment_id = AttachmentId(path.attachment_id);
    let ctx = WorkspaceCtx::new(member.workspace.id, member_to_actor(&member));

    let attachment_repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };
    let attachment = attachment_repo
        .find(&ctx, attachment_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    let owner = authorize_attachment(&state, &member, &attachment, ResourceRole::Editor).await?;
    enforce_attachment_scope(&state, &member, &owner, CapabilityAction::Update).await?;

    let previous_name = attachment.file_name.clone();
    if previous_name == file_name {
        return Ok(Json(workspace_attachment_to_dto(
            &member.workspace.slug,
            WorkspaceAttachment { attachment, owner },
        )));
    }

    reject_duplicate_name(&state, &ctx, &attachment, &file_name).await?;

    let renamed = attachment_repo
        .rename_for_owner(
            &ctx,
            attachment_id,
            attachment_owner_of(&attachment)?,
            file_name.clone(),
        )
        .await
        .map_err(|error| match error {
            atlas_domain::DomainError::NotFound { .. } => ApiError::NotFound,
            other => ApiError::Domain(other),
        })?;

    rewrite_file_references(&state, &ctx, &owner, &previous_name, &file_name).await?;

    Ok(Json(workspace_attachment_to_dto(
        &member.workspace.slug,
        WorkspaceAttachment {
            attachment: renamed,
            owner,
        },
    )))
}

/// The owner selector `AttachmentRepo::rename_for_owner` scopes the update by.
///
/// Mirrors the row's own columns rather than the resolved parent, so a comment
/// attachment stays scoped to its comment.
fn attachment_owner_of(attachment: &Attachment) -> Result<AttachmentOwner, ApiError> {
    if let Some(comment_id) = attachment.comment_id {
        return Ok(AttachmentOwner::Comment(comment_id));
    }
    if let Some(document_id) = attachment.document_id {
        return Ok(AttachmentOwner::Document(document_id));
    }
    if let Some(task_id) = attachment.task_id {
        return Ok(AttachmentOwner::Task(task_id));
    }

    Err(ApiError::NotFound)
}

/// Refuses a name already taken by a sibling attachment of the same owner.
///
/// `[[file:…]]` resolves by name within one owner, so two live siblings sharing
/// a name would make every such link ambiguous.
async fn reject_duplicate_name(
    state: &AppState,
    ctx: &WorkspaceCtx,
    attachment: &Attachment,
    file_name: &str,
) -> Result<(), ApiError> {
    let siblings = PgAttachmentRepo {
        conn: (*state.db).clone(),
    }
    .list_for_owner(ctx, attachment_owner_of(attachment)?)
    .await
    .map_err(ApiError::Domain)?;

    let taken = siblings
        .iter()
        .any(|sibling| sibling.id != attachment.id && sibling.file_name == file_name);

    if taken {
        return Err(ApiError::InvalidInput {
            message: format!(
                "another attachment on this {} is already named '{file_name}'",
                match attachment.task_id {
                    Some(_) => "task",
                    None => "note",
                }
            ),
        });
    }

    Ok(())
}

/// Rewrites `[[file:<old>]]` links in the body that owns the attachment.
///
/// Only a document or task body can carry a `file:` link — comment bodies
/// address attachments by URL — so a comment-owned attachment has nothing to
/// rewrite and the owner's body is left untouched.
async fn rewrite_file_references(
    state: &AppState,
    ctx: &WorkspaceCtx,
    owner: &AttachmentOwnerRef,
    previous_name: &str,
    file_name: &str,
) -> Result<(), ApiError> {
    if owner.comment_id.is_some() {
        return Ok(());
    }

    match owner.kind {
        AttachmentOwnerKind::Document => {
            rewrite_document_references(state, ctx, owner.document_id, previous_name, file_name)
                .await
        }
        AttachmentOwnerKind::Task => {
            rewrite_task_references(state, ctx, owner.task_id, previous_name, file_name).await
        }
    }
}

async fn rewrite_document_references(
    state: &AppState,
    ctx: &WorkspaceCtx,
    document_id: Option<DocumentId>,
    previous_name: &str,
    file_name: &str,
) -> Result<(), ApiError> {
    let Some(document_id) = document_id else {
        return Ok(());
    };

    let service = state.document_service();
    let repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);

    let document = repo
        .get(ctx, document_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    let rewritten = atlas_domain::rename_file_links(&document.content, previous_name, file_name);
    if rewritten == document.content {
        return Ok(());
    }

    let saved = service
        .update_content(ctx, document_id, document.current_revision_id, &rewritten)
        .await
        .map_err(ApiError::Domain)?;

    crate::routes::documents::update_document_links(
        ctx,
        &crate::persistence::repos::PgDocumentLinkRepo {
            conn: (*state.db).clone(),
        },
        saved.id,
        &saved.content,
    )
    .await
}

async fn rewrite_task_references(
    state: &AppState,
    ctx: &WorkspaceCtx,
    task_id: Option<TaskId>,
    previous_name: &str,
    file_name: &str,
) -> Result<(), ApiError> {
    let Some(task_id) = task_id else {
        return Ok(());
    };

    let repo = PgTaskRepo {
        conn: (*state.db).clone(),
    };
    let task = repo
        .find(ctx, task_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    let description = task.description;
    let rewritten = atlas_domain::rename_file_links(&description, previous_name, file_name);
    if rewritten == description {
        return Ok(());
    }

    state
        .task_service()
        .patch(
            ctx,
            task_id,
            atlas_domain::entities::boards_tasks::TaskPatch {
                description: Some(rewritten),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(ApiError::Domain)
}

// ---------------------------------------------------------------------------
// Owner-agnostic authorization
// ---------------------------------------------------------------------------

/// The capability family that governs an attachment, taken from its owner.
///
/// A file on a task is task data even though every attachment shares one route,
/// so an agent's scope is checked against the family it actually reads or writes.
pub(crate) fn owner_capability_family(kind: AttachmentOwnerKind) -> CapabilityFamily {
    match kind {
        AttachmentOwnerKind::Document => CapabilityFamily::Docs,
        AttachmentOwnerKind::Task => CapabilityFamily::Tasks,
    }
}

/// Enforces the owner's capability family on an api-key principal.
pub(crate) async fn enforce_attachment_scope(
    state: &AppState,
    member: &WorkspaceMember,
    owner: &AttachmentOwnerRef,
    action: CapabilityAction,
) -> Result<(), ApiError> {
    let Some(key_id) = member.api_key_id else {
        return Ok(());
    };

    enforce_api_key_scope(
        &state.db,
        key_id,
        Capability {
            family: owner_capability_family(owner.kind),
            action,
        },
    )
    .await
}

/// Authorizes `member` against whatever resource owns `attachment`.
///
/// Returns the resolved owner so callers can render or act on it without a
/// second lookup. A comment-owned attachment resolves to the comment's parent
/// for the role check, and mutating it additionally requires being the
/// comment's author or a workspace owner/admin — the same rule the comment
/// attachment routes enforce.
pub(crate) async fn authorize_attachment(
    state: &AppState,
    member: &WorkspaceMember,
    attachment: &Attachment,
    min_role: ResourceRole,
) -> Result<AttachmentOwnerRef, ApiError> {
    let ctx = WorkspaceCtx::new(member.workspace.id, member_to_actor(member));

    let owner = PgWorkspaceAttachmentRepo::new((*state.db).clone())
        .owner_of(&ctx, attachment.id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    let chain = match owner.kind {
        AttachmentOwnerKind::Document => {
            let document_id = owner.document_id.ok_or(ApiError::NotFound)?;
            let document = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval)
                .get(&ctx, document_id)
                .await
                .map_err(ApiError::Domain)?
                .ok_or(ApiError::NotFound)?;

            build_document_chain(&state.db, &member.workspace, &document).await?
        }
        AttachmentOwnerKind::Task => {
            let task_id = owner.task_id.ok_or(ApiError::NotFound)?;
            let task = PgTaskRepo {
                conn: (*state.db).clone(),
            }
            .find(&ctx, task_id)
            .await
            .map_err(ApiError::Domain)?
            .ok_or(ApiError::NotFound)?;

            build_board_chain(&state.db, &member.workspace, task.board_id, task.project_id).await?
        }
    };

    let effective = resolve_effective_role(
        &state.db,
        &member_to_principal(member),
        member.membership.as_ref().map(|m| m.role.clone()),
        &member.workspace,
        &chain,
    )
    .await?
    .ok_or(ApiError::NotFound)?;

    if effective < min_role {
        return Err(ApiError::NotFound);
    }

    if min_role > ResourceRole::Viewer
        && let Some(comment_id) = owner.comment_id
    {
        require_comment_authorship(state, member, &ctx, &owner, comment_id).await?;
    }

    Ok(owner)
}

async fn require_comment_authorship(
    state: &AppState,
    member: &WorkspaceMember,
    ctx: &WorkspaceCtx,
    owner: &AttachmentOwnerRef,
    comment_id: CommentId,
) -> Result<(), ApiError> {
    use atlas_domain::entities::{comments::CommentOwner, identity::MemberRole};

    let comment_owner = match owner.kind {
        AttachmentOwnerKind::Document => {
            CommentOwner::Document(owner.document_id.ok_or(ApiError::NotFound)?)
        }
        AttachmentOwnerKind::Task => CommentOwner::Task(owner.task_id.ok_or(ApiError::NotFound)?),
    };

    let comment = PgCommentRepo::new((*state.db).clone())
        .get_for_owner(ctx, comment_owner, comment_id)
        .await
        .map_err(ApiError::Domain)?;

    let can_moderate = matches!(
        member.membership.as_ref().map(|m| &m.role),
        Some(MemberRole::Owner) | Some(MemberRole::Admin)
    );

    if comment.created_by != ctx.actor && !can_moderate {
        return Err(ApiError::Domain(atlas_domain::DomainError::Forbidden {
            message: "only the comment's author or a workspace admin/owner may manage attachments"
                .into(),
        }));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// DTO mapping
// ---------------------------------------------------------------------------

fn workspace_attachment_to_dto(
    workspace_slug: &str,
    item: WorkspaceAttachment,
) -> WorkspaceAttachmentDto {
    let WorkspaceAttachment { attachment, owner } = item;

    WorkspaceAttachmentDto {
        id: attachment.id.0,
        file_name: attachment.file_name,
        content_type: attachment.content_type,
        size_bytes: attachment.size_bytes,
        sha256: attachment.sha256,
        actor: crate::routes::documents::make_actor_dto(
            attachment.created_by_user_id.map(|u| u.0),
            attachment.created_by_api_key_id.map(|k| k.0),
        ),
        created_at: attachment.created_at,
        updated_at: attachment.updated_at,
        content_url: attachment_content_url(workspace_slug, attachment.id),
        owner: AttachmentOwnerDto {
            kind: match owner.kind {
                AttachmentOwnerKind::Document => "document".into(),
                AttachmentOwnerKind::Task => "task".into(),
            },
            title: owner.title,
            document_id: owner.document_id.map(|id| id.0),
            document_slug: owner.document_slug,
            task_id: owner.task_id.map(|id| id.0),
            task_readable_id: owner.task_readable_id,
            project_slug: owner.project_slug,
            comment_id: owner.comment_id.map(|id| id.0),
        },
    }
}

fn attachment_content_url(workspace_slug: &str, id: AttachmentId) -> String {
    format!("/api/workspaces/{workspace_slug}/attachments/{}", id.0)
}
