#![allow(clippy::indexing_slicing)]

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;

use atlas_api::{
    dtos::boards_tasks::{
        CommentDto, CommentListResponseDto, CreateCommentRequest, UpdateCommentRequest,
    },
    dtos::documents::{
        ActorDto, AttachmentDto, BacklinkDto, CommentAttachmentDto, CommentBacklinkParentDto,
        CommentBacklinkSourceDto, CopyDocumentRequest, CreateDocumentRequest, DocumentDto,
        DocumentSummaryDto, FrontmatterDto, MoveDocumentRequest, RevisionContentDto,
        RevisionMetaDto, UpdateContentRequest, UpdateDocumentRequest,
    },
    pagination::{Cursor, Page},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::comments::{CommentLinkTarget, CommentOwner},
    entities::documents::{AttachmentOwner, ExtractedLink, NewAttachment, NewDocument},
    entities::identity::MemberRole,
    ids::{AttachmentId, CommentId, DocumentId, FolderId, RevisionId, UserId},
    permissions::{Capability, CapabilityAction, CapabilityFamily, Principal},
    ports::comments::{CommentLinkRepo, CommentRepo},
    resolve_collision, slugify,
};

use crate::{
    authz::{
        Authorized, DocsCreate, DocsDelete, DocsRead, DocsUpdate, EditorMin, MinRole, ViewerMin,
        WorkspaceMember, authorize_folder_destination,
        authorized::{DocumentSlugRes, ProjectRes},
        batch_authorization::{
            BatchAuthorizationService, PgBatchAuthorizationSource, ProjectionSubject,
        },
        enforce_api_key_scope, resolve_folder_ancestry,
    },
    error::ApiError,
    persistence::entities::documents::document,
    persistence::repos::{
        AttachmentRepo, DocumentLinkRepo, DocumentRepo, PgAttachmentLifecycle, PgAttachmentRepo,
        PgCommentLinkRepo, PgCommentRepo, PgDocumentLinkRepo, PgDocumentRepo,
    },
    routes::comments::{
        comment_to_dto, decode_feed_cursor, enrich_comment_entries, project_comment_feed,
    },
    routes::validation::{validate_comment_body, validate_name, validate_upload},
    services::DocumentService,
    state::AppState,
};

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    feed: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RevisionPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    slug: String,
    seq: i64,
}

#[derive(Deserialize)]
pub(crate) struct AttachmentPath {
    #[allow(dead_code)]
    ws: String,
    attachment_id: uuid::Uuid,
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/projects/{project_slug}/documents
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/projects/{project_slug}/documents",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
    ),
    request_body = CreateDocumentRequest,
    responses(
        (status = 201, description = "Document created", body = DocumentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn create_document(
    auth: Authorized<ProjectRes, EditorMin, DocsCreate>,
    State(state): State<AppState>,
    Json(body): Json<CreateDocumentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    validate_name("title", &body.title)?;

    let doc_svc = state.document_service();

    let base_slug = slugify(&body.title);
    let existing = collect_existing_slugs_for_workspace(&state, &ctx).await?;
    let taken: Vec<&str> = existing.iter().map(String::as_str).collect();
    let slug = resolve_collision(&base_slug, &taken);

    let project_id = auth.resource.0.id;
    let folder_id = body.folder_id.map(FolderId);

    if let Some(fid) = folder_id {
        let ancestry = resolve_folder_ancestry(&state.db, auth.workspace.id, fid).await?;

        let folder_project = ancestry.last().and_then(|f| f.project_id);
        if folder_project != Some(project_id) {
            return Err(ApiError::InvalidInput {
                message: "target folder does not exist in this workspace".to_string(),
            });
        }
    }

    let content = body.content.unwrap_or_default();

    let doc = persist_new_document(
        &state,
        &ctx,
        &doc_svc,
        body.title,
        slug,
        content,
        folder_id,
        Some(project_id),
    )
    .await?;

    Ok((StatusCode::CREATED, Json(document_to_dto(doc))))
}

/// Persists a new document and its first revision exactly the way the normal
/// create path does: derives frontmatter from the content, inserts via the
/// repository (which writes the fresh first revision), and replaces the document's
/// outbound wikilinks. Shared by `create_document` and `copy_document` so a copied
/// document is indistinguishable from a freshly created one.
#[allow(clippy::too_many_arguments)]
async fn persist_new_document(
    state: &AppState,
    ctx: &WorkspaceCtx,
    doc_svc: &DocumentService,
    title: String,
    slug: String,
    content: String,
    folder_id: Option<FolderId>,
    project_id: Option<atlas_domain::ids::ProjectId>,
) -> Result<atlas_domain::entities::documents::Document, ApiError> {
    let frontmatter = derive_frontmatter(&content);

    let doc = doc_svc
        .create(
            ctx,
            NewDocument {
                title,
                slug: Some(slug),
                content,
                folder_id,
                project_id,
                frontmatter: Some(frontmatter),
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);
    let link_repo = PgDocumentLinkRepo {
        conn: (*state.db).clone(),
    };
    update_document_links(ctx, &doc_repo, &link_repo, doc.id, &doc.content).await?;

    Ok(doc)
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/projects/{project_slug}/documents
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/projects/{project_slug}/documents",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("project_slug" = String, Path, description = "Project slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated document list", body = Page<DocumentSummaryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_documents(
    auth: Authorized<ProjectRes, ViewerMin, DocsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<DocumentSummaryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let project_id = auth.resource.0.id;

    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);

    let mut items = doc_repo
        .list_visible(&ctx, &auth.principal, Some(project_id), after_id, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = items.len() > limit as usize;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        items.last().map(|d| Cursor(d.id.0))
    } else {
        None
    };

    let dtos: Vec<DocumentSummaryDto> = items
        .into_iter()
        .map(|d| DocumentSummaryDto {
            id: d.id.0,
            slug: d.slug,
            title: d.title,
            folder_id: d.folder_id.map(|f| f.0),
            head_seq: d.current_revision_seq,
            updated_at: d.updated_at,
        })
        .collect();

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/documents/{slug}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    responses(
        (status = 200, description = "Document", body = DocumentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn get_document(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(_state): State<AppState>,
) -> Result<Json<DocumentDto>, ApiError> {
    Ok(Json(document_to_dto(auth.resource.0)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/documents/{slug}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/documents/{slug}",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    request_body = UpdateDocumentRequest,
    responses(
        (status = 200, description = "Document updated", body = DocumentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn update_document(
    auth: Authorized<DocumentSlugRes, EditorMin, DocsUpdate>,
    State(state): State<AppState>,
    Json(body): Json<UpdateDocumentRequest>,
) -> Result<Json<DocumentDto>, ApiError> {
    let doc = auth.resource.0;
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let doc_svc = state.document_service();

    if let Some(ref new_title) = body.title {
        validate_name("title", new_title)?;
    }

    let doc = if let Some(new_title) = body.title {
        if new_title != doc.title {
            doc_svc
                .rename(&ctx, doc.id, new_title)
                .await
                .map_err(ApiError::Domain)?
        } else {
            doc
        }
    } else {
        doc
    };

    let doc = if body.folder_id.is_some() {
        if let Some(fid) = body.folder_id {
            authorize_folder_destination(
                &state.db,
                &auth.principal,
                auth.membership.clone(),
                &auth.workspace,
                FolderId(fid),
                EditorMin::ROLE,
            )
            .await?;
        }

        let folder_id = body.folder_id.map(FolderId);
        doc_svc
            .move_to(&ctx, doc.id, folder_id, doc.project_id)
            .await
            .map_err(ApiError::Domain)?;
        let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);
        doc_repo
            .get(&ctx, doc.id)
            .await
            .map_err(ApiError::Domain)?
            .ok_or(ApiError::NotFound)?
    } else {
        doc
    };

    Ok(Json(document_to_dto(doc)))
}

// ---------------------------------------------------------------------------
// PUT /api/workspaces/{ws}/documents/{slug}/content
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/api/workspaces/{ws}/documents/{slug}/content",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    request_body = UpdateContentRequest,
    responses(
        (status = 200, description = "Content updated", body = DocumentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
        (status = 409, description = "Revision conflict"),
    )
)]
pub(crate) async fn update_content(
    auth: Authorized<DocumentSlugRes, EditorMin, DocsUpdate>,
    State(state): State<AppState>,
    Json(body): Json<UpdateContentRequest>,
) -> Result<Json<DocumentDto>, ApiError> {
    let doc = auth.resource.0;
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let doc_svc = state.document_service();

    let updated = doc_svc
        .update_content(
            &ctx,
            doc.id,
            RevisionId(body.base_revision_id),
            &body.content,
        )
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::Conflict(c) => ApiError::RevisionConflict(c),
            other => ApiError::Domain(other),
        })?;

    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);
    let link_repo = PgDocumentLinkRepo {
        conn: (*state.db).clone(),
    };
    update_document_links(&ctx, &doc_repo, &link_repo, updated.id, &updated.content).await?;

    Ok(Json(document_to_dto(updated)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/documents/{slug}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/documents/{slug}",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    responses(
        (status = 204, description = "Document deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn delete_document(
    auth: Authorized<DocumentSlugRes, EditorMin, DocsDelete>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let doc_svc = state.document_service();

    doc_svc
        .soft_delete(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/documents/{slug}/history
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/history",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size"),
    ),
    responses(
        (status = 200, description = "Revision history"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn list_history(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<RevisionMetaDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);

    let mut revisions = doc_repo
        .history(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    // History returned ascending by seq; reverse for newest-first.
    revisions.reverse();

    if let Some(cursor_uuid) = after_id
        && let Some(pos) = revisions.iter().position(|r| r.id.0 == cursor_uuid)
    {
        revisions = revisions.into_iter().skip(pos + 1).collect();
    }

    let has_more = revisions.len() > limit as usize;
    if has_more {
        revisions.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        revisions.last().map(|r| Cursor(r.id.0))
    } else {
        None
    };

    let dtos: Vec<RevisionMetaDto> = revisions
        .into_iter()
        .map(|r| RevisionMetaDto {
            id: r.id.0,
            seq: r.seq,
            is_anchor: r.is_anchor,
            actor: make_actor_dto(
                r.created_by_user_id.map(|u| u.0),
                r.created_by_api_key_id.map(|k| k.0),
            ),
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/documents/{slug}/revisions/{seq}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/revisions/{seq}",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
        ("seq" = i64, Path, description = "Revision sequence number"),
    ),
    responses(
        (status = 200, description = "Content at revision", body = RevisionContentDto),
        (status = 404, description = "Document or revision not found"),
    )
)]
pub(crate) async fn get_revision_content(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    Path(rev_path): Path<RevisionPath>,
    State(state): State<AppState>,
) -> Result<Json<RevisionContentDto>, ApiError> {
    let seq = rev_path.seq;

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);

    let revisions = doc_repo
        .history(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let rev_meta = revisions
        .into_iter()
        .find(|r| r.seq == seq)
        .ok_or(ApiError::NotFound)?;

    let content = doc_repo
        .content_at(&ctx, auth.resource.0.id, seq)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(RevisionContentDto {
        id: rev_meta.id.0,
        seq,
        content,
        actor: make_actor_dto(
            rev_meta.created_by_user_id.map(|u| u.0),
            rev_meta.created_by_api_key_id.map(|k| k.0),
        ),
        created_at: rev_meta.created_at,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/documents/{slug}/backlinks
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/backlinks",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size"),
    ),
    responses(
        (status = 200, description = "Backlinks", body = Page<BacklinkDto>),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn list_backlinks(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<BacklinkDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let link_repo = PgDocumentLinkRepo {
        conn: (*state.db).clone(),
    };

    let mut links = link_repo
        .backlinks(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    if let Some(cursor_uuid) = after_id
        && let Some(pos) = links.iter().position(|l| l.id.0 == cursor_uuid)
    {
        links = links.into_iter().skip(pos + 1).collect();
    }

    let has_more = links.len() > limit as usize;
    if has_more {
        links.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        links.last().map(|l| Cursor(l.id.0))
    } else {
        None
    };

    let source_ids = links
        .iter()
        .filter_map(|link| link.source_document_id.map(|id| id.0))
        .collect::<Vec<_>>();
    let sources = document::Entity::find()
        .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(document::Column::DeletedAt.is_null())
        .filter(document::Column::Id.is_in(source_ids))
        .all(&*state.db)
        .await
        .map_err(|error| ApiError::Internal {
            message: error.to_string(),
        })?
        .into_iter()
        .map(|source| (source.id, source))
        .collect::<std::collections::HashMap<_, _>>();

    let mut dtos: Vec<BacklinkDto> = Vec::with_capacity(links.len());
    for link in links {
        let Some(src_doc_id) = link.source_document_id else {
            continue;
        };
        let Some(source_doc) = sources.get(&src_doc_id.0) else {
            continue;
        };

        dtos.push(BacklinkDto {
            source_document_id: src_doc_id.0,
            source_slug: source_doc.slug.clone(),
            source_title: source_doc.title.clone(),
            display_title: link.target_title,
            comment_source: None,
        });
    }

    let comment_links = PgCommentLinkRepo::new((*state.db).clone())
        .backlinks_for_target(&ctx, CommentLinkTarget::Document(auth.resource.0.id))
        .await
        .map_err(ApiError::Domain)?;
    let subjects = comment_links
        .iter()
        .map(|link| ProjectionSubject::SourceComment(link.comment_id.0))
        .collect::<Vec<_>>();
    let decisions = if subjects.is_empty() {
        Vec::new()
    } else {
        BatchAuthorizationService::new(PgBatchAuthorizationSource::new((*state.db).clone()))
            .authorize(auth.projection_context(), &subjects)
            .await
            .map_err(ApiError::Domain)?
    };
    for (link, allowed) in comment_links.into_iter().zip(decisions) {
        if !allowed {
            continue;
        }
        let parent = match link.parent {
            CommentOwner::Task(id) => CommentBacklinkParentDto::Task {
                id: id.0,
                readable_id: link.parent_readable_id.unwrap_or_default(),
                title: link.parent_title,
            },
            CommentOwner::Document(id) => CommentBacklinkParentDto::Document {
                id: id.0,
                slug: link.parent_slug,
                title: link.parent_title,
            },
        };
        dtos.push(BacklinkDto {
            source_document_id: link.comment_id.0,
            source_slug: None,
            source_title: String::new(),
            display_title: String::new(),
            comment_source: Some(CommentBacklinkSourceDto {
                kind: "comment".into(),
                comment_id: link.comment_id.0,
                parent,
            }),
        });
    }

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/documents/{slug}/frontmatter
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/frontmatter",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    responses(
        (status = 200, description = "Frontmatter data", body = FrontmatterDto),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn get_frontmatter(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(_state): State<AppState>,
) -> Result<Json<FrontmatterDto>, ApiError> {
    Ok(Json(FrontmatterDto {
        data: auth.resource.0.frontmatter,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/documents/{slug}/attachments
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/documents/{slug}/attachments",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    responses(
        (status = 201, description = "Attachment created", body = AttachmentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 413, description = "Payload too large"),
    )
)]
pub(crate) async fn upload_attachment(
    auth: Authorized<DocumentSlugRes, EditorMin, DocsUpdate>,
    State(state): State<AppState>,
    request: Request,
) -> Result<impl IntoResponse, ApiError> {
    let file_name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    // Bound the read at the cap (plus one byte to detect an exactly-at-cap+1 body)
    // so an oversize upload is rejected during streaming instead of being fully
    // buffered into memory first.
    let read_limit = state.max_attachment_bytes.saturating_add(1) as usize;

    let body: Bytes = match axum::body::to_bytes(request.into_body(), read_limit).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(ApiError::PayloadTooLarge {
                message: format!(
                    "attachment exceeds maximum size of {} bytes",
                    state.max_attachment_bytes
                ),
            });
        }
    };

    if body.len() as u64 > state.max_attachment_bytes {
        return Err(ApiError::PayloadTooLarge {
            message: format!(
                "attachment exceeds maximum size of {} bytes",
                state.max_attachment_bytes
            ),
        });
    }

    validate_upload(
        &file_name,
        &body,
        state.upload_allowed_extensions.as_deref(),
    )?;

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let attachment = PgAttachmentLifecycle::store_and_record(
        state.db.as_ref(),
        &ctx,
        NewAttachment {
            document_id: Some(auth.resource.0.id),
            task_id: None,
            comment_id: None,
            file_name,
            content_type,
            size_bytes: body.len() as i64,
            sha256: String::new(),
        },
        &body,
        state.attachments.as_ref(),
    )
    .await
    .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(attachment_to_dto(attachment))))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/documents/{slug}/attachments
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/attachments",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size"),
    ),
    responses(
        (status = 200, description = "Attachment list"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn list_attachments(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<AttachmentDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let attachment_repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };

    let mut items = attachment_repo
        .list_for_owner(&ctx, AttachmentOwner::Document(auth.resource.0.id))
        .await
        .map_err(ApiError::Domain)?;

    if let Some(cursor_uuid) = after_id
        && let Some(pos) = items.iter().position(|a| a.id.0 == cursor_uuid)
    {
        items = items.into_iter().skip(pos + 1).collect();
    }

    let has_more = items.len() > limit as usize;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        items.last().map(|a| Cursor(a.id.0))
    } else {
        None
    };

    let dtos = items.into_iter().map(attachment_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/attachments/{attachment_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/attachments/{attachment_id}",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("attachment_id" = String, Path, description = "Attachment UUID"),
    ),
    responses(
        (status = 200, description = "Binary attachment content"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Attachment not found"),
    )
)]
pub(crate) async fn download_attachment(
    member: WorkspaceMember,
    Path(att_path): Path<AttachmentPath>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let attachment_id = AttachmentId(att_path.attachment_id);

    let actor = member_to_actor(&member);
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);

    let attachment_repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };

    let attachment = attachment_repo
        .find(&ctx, attachment_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    authorize_attachment_document(&state, &member, &attachment, ViewerMin::ROLE).await?;

    if let Some(key_id) = member.api_key_id {
        enforce_api_key_scope(
            &state.db,
            key_id,
            Capability {
                family: CapabilityFamily::Docs,
                action: CapabilityAction::Read,
            },
        )
        .await?;
    }

    let bytes = state
        .attachments
        .get(&attachment.sha256)
        .await
        .map_err(|e| match e {
            atlas_domain::DomainError::NotFound { .. } => ApiError::NotFound,
            other => ApiError::Internal {
                message: other.to_string(),
            },
        })?;

    let content_type = attachment.content_type.clone();
    let content_disposition = content_disposition_attachment(&attachment.file_name);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(response)
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/attachments/{attachment_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/attachments/{attachment_id}",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("attachment_id" = String, Path, description = "Attachment UUID"),
    ),
    responses(
        (status = 204, description = "Attachment deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Attachment not found"),
    )
)]
pub(crate) async fn delete_attachment(
    member: WorkspaceMember,
    Path(att_path): Path<AttachmentPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let attachment_id = AttachmentId(att_path.attachment_id);

    let actor = member_to_actor(&member);
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);

    let attachment_repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };

    let attachment = attachment_repo
        .find(&ctx, attachment_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    authorize_attachment_document(&state, &member, &attachment, EditorMin::ROLE).await?;

    if let Some(key_id) = member.api_key_id {
        enforce_api_key_scope(
            &state.db,
            key_id,
            Capability {
                family: CapabilityFamily::Docs,
                action: CapabilityAction::Update,
            },
        )
        .await?;
    }

    attachment_repo
        .soft_delete(&ctx, attachment_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
    operation_id = "upload_document_comment_attachment",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("slug" = String, Path), ("comment_id" = String, Path)),
    responses((status = 201, body = CommentAttachmentDto), (status = 404), (status = 413), (status = 422))
)]
pub(crate) async fn upload_comment_attachment(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsUpdate>,
    Path(path): Path<DocumentCommentPath>,
    State(state): State<AppState>,
    request: Request,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let owner = CommentOwner::Document(auth.resource.0.id);
    let comment = PgCommentRepo::new((*state.db).clone())
        .get_for_owner(&ctx, owner, CommentId(path.comment_id))
        .await
        .map_err(ApiError::Domain)?;
    let can_moderate = matches!(
        auth.membership,
        Some(MemberRole::Owner) | Some(MemberRole::Admin)
    );
    if comment.created_by != ctx.actor && !can_moderate {
        return Err(ApiError::Domain(atlas_domain::DomainError::Forbidden {
            message: "only the comment's author or a workspace admin/owner may manage attachments"
                .into(),
        }));
    }
    let file_name = request
        .headers()
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("upload")
        .to_string();
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let read_limit = state.max_attachment_bytes.saturating_add(1) as usize;
    let body = axum::body::to_bytes(request.into_body(), read_limit)
        .await
        .map_err(|_| ApiError::PayloadTooLarge {
            message: format!(
                "attachment exceeds maximum size of {} bytes",
                state.max_attachment_bytes
            ),
        })?;
    if body.len() as u64 > state.max_attachment_bytes {
        return Err(ApiError::PayloadTooLarge {
            message: format!(
                "attachment exceeds maximum size of {} bytes",
                state.max_attachment_bytes
            ),
        });
    }
    validate_upload(
        &file_name,
        &body,
        state.upload_allowed_extensions.as_deref(),
    )?;
    let attachment = PgAttachmentLifecycle::store_and_record(
        state.db.as_ref(),
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: None,
            comment_id: Some(comment.id),
            file_name,
            content_type,
            size_bytes: body.len() as i64,
            sha256: String::new(),
        },
        &body,
        state.attachments.as_ref(),
    )
    .await
    .map_err(ApiError::Domain)?;
    Ok((
        StatusCode::CREATED,
        Json(comment_attachment_to_dto(attachment)),
    ))
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
    operation_id = "list_document_comment_attachments",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("slug" = String, Path), ("comment_id" = String, Path)),
    responses((status = 200, body = Vec<CommentAttachmentDto>), (status = 404))
)]
pub(crate) async fn list_comment_attachments(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    Path(path): Path<DocumentCommentPath>,
    State(state): State<AppState>,
) -> Result<Json<Vec<CommentAttachmentDto>>, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let comment_id = CommentId(path.comment_id);
    PgCommentRepo::new((*state.db).clone())
        .get_for_owner(&ctx, CommentOwner::Document(auth.resource.0.id), comment_id)
        .await
        .map_err(ApiError::Domain)?;
    let items = PgAttachmentRepo {
        conn: (*state.db).clone(),
    }
    .list_for_owner(&ctx, AttachmentOwner::Comment(comment_id))
    .await
    .map_err(ApiError::Domain)?;
    Ok(Json(
        items.into_iter().map(comment_attachment_to_dto).collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
    operation_id = "download_document_comment_attachment",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("slug" = String, Path), ("comment_id" = String, Path), ("attachment_id" = String, Path)),
    responses((status = 200, description = "Binary attachment content"), (status = 404))
)]
pub(crate) async fn download_comment_attachment(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    Path(path): Path<DocumentCommentAttachmentPath>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let comment_id = CommentId(path.comment_id);
    PgCommentRepo::new((*state.db).clone())
        .get_for_owner(&ctx, CommentOwner::Document(auth.resource.0.id), comment_id)
        .await
        .map_err(ApiError::Domain)?;
    let attachment = PgAttachmentRepo {
        conn: (*state.db).clone(),
    }
    .find(&ctx, AttachmentId(path.attachment_id))
    .await
    .map_err(ApiError::Domain)?
    .filter(|attachment| attachment.comment_id == Some(comment_id))
    .ok_or(ApiError::NotFound)?;
    comment_attachment_response(&state, attachment).await
}

#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
    operation_id = "delete_document_comment_attachment",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("slug" = String, Path), ("comment_id" = String, Path), ("attachment_id" = String, Path)),
    responses((status = 204), (status = 404))
)]
pub(crate) async fn delete_comment_attachment(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsUpdate>,
    Path(path): Path<DocumentCommentAttachmentPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let comment_id = CommentId(path.comment_id);
    let comment = PgCommentRepo::new((*state.db).clone())
        .get_for_owner(&ctx, CommentOwner::Document(auth.resource.0.id), comment_id)
        .await
        .map_err(ApiError::Domain)?;
    let can_moderate = matches!(
        auth.membership,
        Some(MemberRole::Owner) | Some(MemberRole::Admin)
    );
    if comment.created_by != ctx.actor && !can_moderate {
        return Err(ApiError::Domain(atlas_domain::DomainError::Forbidden {
            message: "only the comment's author or a workspace admin/owner may manage attachments"
                .into(),
        }));
    }
    let repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };
    let attachment_id = AttachmentId(path.attachment_id);
    let attachment = repo
        .find(&ctx, attachment_id)
        .await
        .map_err(ApiError::Domain)?;
    if attachment
        .and_then(|attachment| (attachment.comment_id == Some(comment_id)).then_some(attachment))
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    repo.soft_delete(&ctx, attachment_id)
        .await
        .map_err(ApiError::Domain)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/documents/{slug}/move
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/documents/{slug}/move",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    request_body = MoveDocumentRequest,
    responses(
        (status = 200, description = "Document moved", body = DocumentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn move_document(
    auth: Authorized<DocumentSlugRes, EditorMin, DocsUpdate>,
    State(state): State<AppState>,
    Json(body): Json<MoveDocumentRequest>,
) -> Result<Json<DocumentDto>, ApiError> {
    let doc = auth.resource.0;
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let doc_svc = state.document_service();

    if let Some(fid) = body.folder_id {
        authorize_folder_destination(
            &state.db,
            &auth.principal,
            auth.membership.clone(),
            &auth.workspace,
            FolderId(fid),
            EditorMin::ROLE,
        )
        .await?;
    }

    let folder_id = body.folder_id.map(FolderId);
    doc_svc
        .move_to(&ctx, doc.id, folder_id, doc.project_id)
        .await
        .map_err(ApiError::Domain)?;

    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);
    let updated = doc_repo
        .get(&ctx, doc.id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(document_to_dto(updated)))
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/documents/{slug}/copy
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/documents/{slug}/copy",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Source document slug"),
    ),
    request_body = CopyDocumentRequest,
    responses(
        (status = 201, description = "Document copied", body = DocumentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn copy_document(
    auth: Authorized<DocumentSlugRes, EditorMin, DocsCreate>,
    State(state): State<AppState>,
    Json(body): Json<CopyDocumentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let source = auth.resource.0;
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let doc_svc = state.document_service();

    let folder_id = match body.folder_id {
        Some(fid) => Some(FolderId(fid)),
        None => source.folder_id,
    };

    if let Some(fid) = body.folder_id {
        authorize_folder_destination(
            &state.db,
            &auth.principal,
            auth.membership.clone(),
            &auth.workspace,
            FolderId(fid),
            EditorMin::ROLE,
        )
        .await?;
    }

    let title = format!("{} (copy)", source.title);

    let base_slug = slugify(&title);
    let existing = collect_existing_slugs_for_workspace(&state, &ctx).await?;
    let taken: Vec<&str> = existing.iter().map(String::as_str).collect();
    let slug = resolve_collision(&base_slug, &taken);

    let copy = persist_new_document(
        &state,
        &ctx,
        &doc_svc,
        title,
        slug,
        source.content,
        folder_id,
        source.project_id,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(document_to_dto(copy))))
}

/// Copies a single source document into `folder_id` / `project_id`, keeping the
/// source title verbatim (no " (copy)" suffix) and a fresh collision-resolved
/// slug. Used by the recursive folder copy to duplicate every document in the
/// source subtree.
pub(crate) async fn copy_document_into(
    state: &AppState,
    ctx: &WorkspaceCtx,
    doc_svc: &DocumentService,
    source: &atlas_domain::entities::documents::Document,
    folder_id: Option<FolderId>,
    project_id: Option<atlas_domain::ids::ProjectId>,
) -> Result<atlas_domain::entities::documents::Document, ApiError> {
    let base_slug = slugify(&source.title);
    let existing = collect_existing_slugs_for_workspace(state, ctx).await?;
    let taken: Vec<&str> = existing.iter().map(String::as_str).collect();
    let slug = resolve_collision(&base_slug, &taken);

    persist_new_document(
        state,
        ctx,
        doc_svc,
        source.title.clone(),
        slug,
        source.content.clone(),
        folder_id,
        project_id,
    )
    .await
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn collect_existing_slugs_for_workspace(
    state: &AppState,
    ctx: &WorkspaceCtx,
) -> Result<Vec<String>, ApiError> {
    use sea_orm::FromQueryResult;

    #[derive(FromQueryResult)]
    struct SlugRow {
        slug: String,
    }

    let rows = SlugRow::find_by_statement(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT slug FROM documents WHERE workspace_id = $1 AND deleted_at IS NULL AND slug IS NOT NULL",
        [ctx.workspace_id.0.into()],
    ))
    .all(&*state.db)
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(rows.into_iter().map(|r| r.slug).collect())
}

/// Authorizes the request principal against the document that owns `attachment`,
/// requiring at least `min_role` on that document's permission chain.
///
/// Attachment binaries are reached by id without going through the document
/// extractor, so this re-applies the same document-level resolution the rest of
/// the document routes use. A principal lacking the role is rejected with
/// `NotFound` to avoid disclosing the attachment's or document's existence.
async fn authorize_attachment_document(
    state: &AppState,
    member: &WorkspaceMember,
    attachment: &atlas_domain::entities::documents::Attachment,
    min_role: atlas_domain::permissions::ResourceRole,
) -> Result<(), ApiError> {
    let document_id = attachment.document_id.ok_or(ApiError::NotFound)?;

    let ctx = WorkspaceCtx::new(member.workspace.id, member_to_actor(member));
    let doc_repo = PgDocumentRepo::new((*state.db).clone(), state.anchor_interval);

    let doc = doc_repo
        .get(&ctx, document_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    let principal = member_to_principal(member);
    let membership = member.membership.as_ref().map(|m| m.role.clone());

    let chain = crate::authz::build_document_chain(&state.db, &member.workspace, &doc).await?;

    let effective = crate::authz::resolve_effective_role(
        &state.db,
        &principal,
        membership,
        &member.workspace,
        &chain,
    )
    .await?
    .ok_or(ApiError::NotFound)?;

    if effective < min_role {
        return Err(ApiError::NotFound);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Document comments
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct DocumentCommentPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    slug: String,
    comment_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct DocumentCommentAttachmentPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    slug: String,
    comment_id: uuid::Uuid,
    attachment_id: uuid::Uuid,
}

// GET /api/workspaces/{ws}/documents/{slug}/comments
#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/documents/{slug}/comments",
    operation_id = "list_document_comments",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size"),
        ("feed" = Option<String>, Query, description = "Set to `full` for authorized links and retained events"),
    ),
    responses(
        (status = 200, description = "Comment page. `feed=full` returns authorized links and retained events.", body = CommentListResponseDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
    )
)]
pub(crate) async fn list_comments(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Response, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let document_id = auth.resource.0.id;

    if q.feed.as_deref() == Some("full") {
        let after = decode_feed_cursor(q.cursor.as_deref())?;
        let (entries, next_cursor, has_more) = project_comment_feed(
            &state,
            &ctx,
            CommentOwner::Document(document_id),
            auth.projection_context(),
            after,
            limit,
        )
        .await?;
        return Ok(Json(Page {
            items: entries,
            next_cursor,
            has_more,
        })
        .into_response());
    }

    let after_id = q
        .cursor
        .as_deref()
        .and_then(Cursor::decode)
        .map(|c| CommentId(c.0));

    let mut entries = state
        .document_service()
        .list_comments(&ctx, document_id, after_id, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = entries.len() > limit as usize;
    if has_more {
        entries.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        entries.last().map(|c| Cursor(c.id.0))
    } else {
        None
    };

    let dtos = enrich_comment_entries(&state, CommentOwner::Document(document_id), entries).await?;

    Ok(Json(Page::new(dtos, next_cursor, has_more)).into_response())
}

// POST /api/workspaces/{ws}/documents/{slug}/comments
#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/documents/{slug}/comments",
    operation_id = "create_document_comment",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
    ),
    request_body = CreateCommentRequest,
    responses(
        (status = 201, description = "Comment created", body = CommentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Document not found"),
        (status = 422, description = "Comment body is blank or exceeds the maximum length"),
    )
)]
pub(crate) async fn create_comment(
    auth: Authorized<DocumentSlugRes, EditorMin, DocsUpdate>,
    State(state): State<AppState>,
    Json(body): Json<CreateCommentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    validate_comment_body(&body.body)?;

    let document_id = auth.resource.0.id;

    let comment = state
        .document_service()
        .add_comment(&ctx, document_id, body.body)
        .await
        .map_err(ApiError::Domain)?;

    let dto = comment_to_dto(&state, &ctx, CommentOwner::Document(document_id), comment).await;
    Ok((StatusCode::CREATED, Json(dto)))
}

// PATCH /api/workspaces/{ws}/documents/{slug}/comments/{comment_id}
#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
    operation_id = "update_document_comment",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
        ("comment_id" = String, Path, description = "Comment UUID"),
    ),
    request_body = UpdateCommentRequest,
    responses(
        (status = 200, description = "Comment updated", body = CommentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Only the comment's author may edit it"),
        (status = 404, description = "Document or comment not found"),
        (status = 422, description = "Comment body is blank or exceeds the maximum length"),
    )
)]
pub(crate) async fn update_comment(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsUpdate>,
    Path(p): Path<DocumentCommentPath>,
    State(state): State<AppState>,
    Json(body): Json<UpdateCommentRequest>,
) -> Result<Json<CommentDto>, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    validate_comment_body(&body.body)?;

    let document_id = auth.resource.0.id;

    let comment = state
        .document_service()
        .update_comment(&ctx, document_id, CommentId(p.comment_id), body.body)
        .await
        .map_err(ApiError::Domain)?;

    let dto = comment_to_dto(&state, &ctx, CommentOwner::Document(document_id), comment).await;
    Ok(Json(dto))
}

// DELETE /api/workspaces/{ws}/documents/{slug}/comments/{comment_id}
#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
    operation_id = "delete_document_comment",
    tag = "documents",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("slug" = String, Path, description = "Document slug"),
        ("comment_id" = String, Path, description = "Comment UUID"),
    ),
    responses(
        (status = 204, description = "Comment deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Neither the comment's author nor a workspace admin/owner"),
        (status = 404, description = "Document or comment not found"),
    )
)]
pub(crate) async fn delete_comment(
    auth: Authorized<DocumentSlugRes, ViewerMin, DocsUpdate>,
    Path(p): Path<DocumentCommentPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let can_moderate = matches!(
        auth.membership,
        Some(MemberRole::Owner) | Some(MemberRole::Admin)
    );

    state
        .document_service()
        .remove_comment(
            &ctx,
            auth.resource.0.id,
            CommentId(p.comment_id),
            can_moderate,
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Derives the frontmatter JSON object from document content by parsing the
/// leading YAML block. Returns an empty object when there is no frontmatter.
///
/// Shared by create and content-update so both paths produce identical
/// frontmatter from the same content.
fn derive_frontmatter(content: &str) -> serde_json::Value {
    let (yaml, _body) = atlas_domain::frontmatter::strip_frontmatter(content);
    atlas_domain::frontmatter::parse_frontmatter_yaml(yaml.unwrap_or(""))
}

async fn update_document_links(
    ctx: &WorkspaceCtx,
    doc_repo: &PgDocumentRepo,
    link_repo: &PgDocumentLinkRepo,
    doc_id: DocumentId,
    content: &str,
) -> Result<(), ApiError> {
    let raw_links = atlas_domain::parse_wikilinks(content);

    let mut extracted = Vec::with_capacity(raw_links.len());
    for raw in raw_links {
        let (target_id, title) = atlas_domain::parse_wikilink_target(&raw);

        let target_document_id = match target_id {
            Some(id) => doc_repo
                .get(ctx, DocumentId(id))
                .await
                .map_err(ApiError::Domain)?
                .map(|d| d.id),
            None => doc_repo
                .find_by_slug(ctx, &slugify(&title))
                .await
                .map_err(ApiError::Domain)?
                .map(|d| d.id),
        };

        extracted.push(ExtractedLink {
            target_title: title,
            target_document_id,
        });
    }

    link_repo
        .replace_for_source(ctx, doc_id, extracted)
        .await
        .map_err(ApiError::Domain)?;

    Ok(())
}

fn document_to_dto(doc: atlas_domain::entities::documents::Document) -> DocumentDto {
    DocumentDto {
        id: doc.id.0,
        workspace_id: doc.workspace_id.0,
        project_id: doc.project_id.map(|p| p.0),
        folder_id: doc.folder_id.map(|f| f.0),
        slug: doc.slug,
        title: doc.title,
        content: doc.content,
        head_revision_id: doc.current_revision_id.0,
        head_seq: doc.current_revision_seq,
        frontmatter: doc.frontmatter,
        created_at: doc.created_at,
        updated_at: doc.updated_at,
    }
}

fn attachment_to_dto(a: atlas_domain::entities::documents::Attachment) -> AttachmentDto {
    AttachmentDto {
        id: a.id.0,
        document_id: a.document_id.map(|d| d.0).unwrap_or_else(uuid::Uuid::nil),
        file_name: a.file_name,
        content_type: a.content_type,
        size_bytes: a.size_bytes,
        sha256: a.sha256,
        actor: make_actor_dto(
            a.created_by_user_id.map(|u| u.0),
            a.created_by_api_key_id.map(|k| k.0),
        ),
        created_at: a.created_at,
    }
}

fn comment_attachment_to_dto(
    attachment: atlas_domain::entities::documents::Attachment,
) -> CommentAttachmentDto {
    CommentAttachmentDto {
        id: attachment.id.0,
        comment_id: attachment
            .comment_id
            .map(|id| id.0)
            .unwrap_or_else(uuid::Uuid::nil),
        file_name: attachment.file_name,
        content_type: attachment.content_type,
        size_bytes: attachment.size_bytes,
        sha256: attachment.sha256,
        actor: make_actor_dto(
            attachment.created_by_user_id.map(|id| id.0),
            attachment.created_by_api_key_id.map(|id| id.0),
        ),
        created_at: attachment.created_at,
    }
}

async fn comment_attachment_response(
    state: &AppState,
    attachment: atlas_domain::entities::documents::Attachment,
) -> Result<Response, ApiError> {
    let bytes = state
        .attachments
        .get(&attachment.sha256)
        .await
        .map_err(|error| match error {
            atlas_domain::DomainError::NotFound { .. } => ApiError::NotFound,
            other => ApiError::Internal {
                message: other.to_string(),
            },
        })?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, attachment.content_type)
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition_attachment(&attachment.file_name),
        )
        .header("x-content-type-options", "nosniff")
        .body(Body::from(bytes))
        .map_err(|error| ApiError::Internal {
            message: error.to_string(),
        })
}

/// Builds a `Content-Disposition: attachment` header value for a client-supplied
/// file name without letting that name break out of the header.
///
/// The name is stored verbatim from the upload, so it can contain quotes, control
/// characters, or non-ASCII bytes. We emit an ASCII `filename=` fallback (control
/// chars stripped, quotes and backslashes escaped) plus an RFC 5987 `filename*`
/// carrying the full UTF-8 name percent-encoded, which modern clients prefer.
pub(crate) fn content_disposition_attachment(file_name: &str) -> String {
    let ascii_fallback: String = file_name
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            c if c.is_ascii() => c.to_string(),
            _ => '_'.to_string(),
        })
        .collect();

    let encoded = rfc5987_encode(file_name);

    format!("attachment; filename=\"{ascii_fallback}\"; filename*=UTF-8''{encoded}")
}

/// Percent-encodes `value` per RFC 5987 `value-chars`, keeping only the
/// unreserved attr-char set and encoding every other byte as `%XX`.
fn rfc5987_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        let is_attr_char = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            );

        if is_attr_char {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }

    out
}

fn make_actor_dto(user_id: Option<uuid::Uuid>, api_key_id: Option<uuid::Uuid>) -> Option<ActorDto> {
    if let Some(uid) = user_id {
        Some(ActorDto {
            r#type: "user".into(),
            id: uid,
            display_name: None,
            key_type: None,
            account_status: None,
        })
    } else {
        api_key_id.map(|kid| ActorDto {
            r#type: "api_key".into(),
            id: kid,
            display_name: None,
            key_type: None,
            account_status: None,
        })
    }
}

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => Actor::User(atlas_domain::ids::UserId(uuid::Uuid::nil())),
    }
}

fn member_to_actor(member: &WorkspaceMember) -> Actor {
    if let Some(user) = &member.user {
        Actor::User(user.id)
    } else if let Some(kid) = member.api_key_id {
        Actor::ApiKey(kid)
    } else {
        Actor::User(UserId::new())
    }
}

fn member_to_principal(member: &WorkspaceMember) -> Principal {
    if let Some(user) = &member.user {
        Principal::User(user.id)
    } else if let Some(kid) = member.api_key_id {
        Principal::ApiKey(kid)
    } else {
        Principal::User(UserId::new())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn content_disposition_escapes_quote_and_strips_control_chars() {
        let malicious = "a\"; rm -rf /\r\nX-Evil: 1.txt";
        let header = content_disposition_attachment(malicious);

        assert!(
            !header.contains('\r') && !header.contains('\n'),
            "control chars must not appear in the header: {header}"
        );

        let ascii_part = header
            .split("; filename*=")
            .next()
            .expect("ascii filename part");
        assert!(
            ascii_part.contains("\\\""),
            "embedded quote must be escaped in the ASCII fallback: {header}"
        );

        assert!(
            header.contains("filename*=UTF-8''"),
            "header must carry an RFC 5987 filename*: {header}"
        );
        assert!(
            header.contains("%0D%0A"),
            "control bytes must be percent-encoded in filename*: {header}"
        );
    }

    #[test]
    fn content_disposition_percent_encodes_non_ascii() {
        let header = content_disposition_attachment("résumé.pdf");
        assert!(
            header.contains("filename*=UTF-8''r%C3%A9sum%C3%A9.pdf"),
            "non-ASCII name must be UTF-8 percent-encoded: {header}"
        );
    }
}
