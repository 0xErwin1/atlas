#![allow(clippy::indexing_slicing)]

use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use sha2::Digest;

use crate::authz::ResourceRole;
use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::boards_tasks::AssigneeRef;
use atlas_acta::entities::boards_tasks::Board;
use atlas_acta::entities::boards_tasks::BoardColumn;
use atlas_acta::entities::boards_tasks::InitialTaskReference;
use atlas_acta::entities::boards_tasks::NewTask;
use atlas_acta::entities::boards_tasks::NewTaskChecklistItem;
use atlas_acta::entities::boards_tasks::NewTaskReference;
use atlas_acta::entities::boards_tasks::PositionBetween;
use atlas_acta::entities::boards_tasks::Priority;
use atlas_acta::entities::boards_tasks::Task;
use atlas_acta::entities::boards_tasks::TaskActivity;
use atlas_acta::entities::boards_tasks::TaskAssignee;
use atlas_acta::entities::boards_tasks::TaskChecklistItem;
use atlas_acta::entities::boards_tasks::TaskPatch;
use atlas_acta::entities::boards_tasks::TaskReference;
use atlas_acta::entities::comments::CommentDraftMetadata;
use atlas_acta::entities::comments::CommentLinkTarget;
use atlas_acta::entities::comments::CommentOwner;
use atlas_acta::entities::comments::NewCommentAttachmentDraftUpload;
use atlas_acta::entities::comments::comment_draft_upload_digest_input;
use atlas_acta::entities::documents::AttachmentOwner;
use atlas_acta::entities::documents::NewAttachment;
use atlas_acta::entities::documents::RankedTaskDescriptionLink;
use atlas_acta::entities::documents::rank_task_description_links;
use atlas_acta::entities::identity::MemberRole;
use atlas_acta::entities::identity::Workspace;
use atlas_acta::entities::task_views::ActorTypeFilter;
use atlas_acta::entities::task_views::AssigneeFilter;
use atlas_acta::entities::task_views::TaskSort;
use atlas_acta::entities::task_views::TaskViewFilters;
use atlas_acta::entities::workspace_core::AppliesTo;
use atlas_acta::entities::workspace_core::PropertyDefinition;
use atlas_acta::ids::AttachmentId;
use atlas_acta::ids::BoardId;
use atlas_acta::ids::ChecklistItemId;
use atlas_acta::ids::ColumnId;
use atlas_acta::ids::CommentDraftId;
use atlas_acta::ids::CommentId;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::TaskActivityId;
use atlas_acta::ids::TaskId;
use atlas_acta::ids::TaskReferenceId;
use atlas_acta::permissions::ResourceRef;
use atlas_acta::ports::boards_tasks::WorkspaceActivityFilters;
use atlas_acta::ports::boards_tasks::WorkspaceActivityScope;
use atlas_acta::ports::comments::CommentAttachmentDraftRepo;
use atlas_acta::ports::comments::CommentLinkRepo;
use atlas_acta::ports::comments::CommentRepo;
use atlas_acta::ports::documents::DocumentLinkRepo;
use atlas_api::{
    dtos::boards_tasks::{
        ActivityEntryDto, AddAssigneeRequest, AssigneeDto, ChecklistItemDto, CommentDto,
        CommentListResponseDto, CreateChecklistItemRequest, CreateCommentRequest,
        CreateReferenceBatchRequest, CreateReferenceBatchResultDto, CreateReferenceRequest,
        CreateSubtaskRequest, CreateTaskRequest, CreateTaskResponseDto, DocumentBacklinkSourceDto,
        MoveTaskRequest, PromoteChecklistItemRequest, PromotionDto, ReferenceDto,
        ReferenceOriginDto, RenameTaskAttachmentRequest, SetTaskParentRequest, TaskAttachmentDto,
        TaskBacklinkDto, TaskDto, TaskGraphDto, TaskGraphEdgeDto, TaskGraphNodeDto, TaskSummaryDto,
        UnifiedReferenceDto, UpdateChecklistItemRequest, UpdateCommentRequest, UpdateTaskRequest,
        WorkspaceTaskQueryParams,
    },
    dtos::documents::{
        ActorDto, CommentAttachmentDto, CommentBacklinkParentDto, CommentBacklinkSourceDto,
        CommentDraftDto,
    },
    pagination::{Cursor, Page, SearchCursor, SortKey},
};
use atlas_core::error::DomainError;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::Principal;
use atlas_core::principal::UserId;
use atlas_custos::capability::Capability;
use atlas_custos::capability::CapabilityAction;
use atlas_custos::capability::CapabilityFamily;

use crate::{
    authz::{
        Authorized, BoardRes, EditorMin, MinRole, TaskRes, TasksCreate, TasksDelete, TasksRead,
        TasksUpdate, ViewerMin, WorkspaceMember, authorize_board_destination,
        batch_authorization::{
            BatchAuthorizationService, PgBatchAuthorizationSource, ProjectionSubject,
        },
        build_board_chain, build_document_chain, enforce_api_key_scope,
        policy::{ChainSegment, ResolutionInput, ResourceChain, resolve},
        resolve_effective_role,
    },
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, AttachmentRepo, PgAttachmentLifecycle, PgAttachmentRepo, PgProjectRepo,
        ProjectRepo, UserRepo,
    },
    routes::comment_attachment_markdown,
    routes::comments::{
        comment_to_dto, decode_feed_cursor, enrich_comment_entries, project_comment_feed,
    },
    routes::documents::content_disposition_attachment,
    routes::validation::{
        validate_comment_body, validate_custom_entry_count, validate_custom_properties,
        validate_description, validate_labels, validate_name, validate_upload,
        validate_upload_extension,
    },
    services::CommentDraftService,
    state::AppState,
    task_graph::{self, GraphEdge, GraphNodeKind, TaskGraphExplorer},
};
use atlas_acta_postgres::entities::boards_tasks::task;
use atlas_acta_postgres::entities::documents::document;
use atlas_acta_postgres::repos::boards_tasks::{
    PgBoardRepo, PgTaskActivityRepo, PgTaskAssigneeRepo, PgTaskChecklistRepo, PgTaskReferenceRepo,
    PgTaskRepo, TaskActivityRepo, TaskAssigneeRepo, TaskChecklistRepo, TaskReferenceRepo, TaskRepo,
};
use atlas_acta_postgres::repos::comment_links::PgCommentLinkRepo;
use atlas_acta_postgres::repos::comments::PgCommentRepo;
use atlas_acta_postgres::repos::documents::{DocumentRepo, PgDocumentLinkRepo, PgDocumentRepo};
use atlas_acta_postgres::repos::identity::{MembershipRepo, PgMembershipRepo};
use atlas_acta_postgres::repos::workspace_core::{
    PgPropertyDefinitionRepo, PropertyDefinitionRepo,
};
use atlas_custos_postgres::repos::identity::{PgApiKeyRepo, PgUserRepo};
use atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo;

// ---------------------------------------------------------------------------
// Shared path structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    feed: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct AssigneePath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    /// Encoded as `user:{uuid}` or `api_key:{uuid}`.
    assignee_ref: String,
}

#[derive(Deserialize)]
pub(crate) struct ReferencePath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    reference_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct ChecklistItemPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    item_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct TaskAttachmentPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    attachment_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct CommentPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    #[serde(alias = "draft_id")]
    comment_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct CommentAttachmentPath {
    #[allow(dead_code)]
    ws: String,
    #[allow(dead_code)]
    readable_id: String,
    comment_id: uuid::Uuid,
    attachment_id: uuid::Uuid,
}

fn comment_draft_to_dto(
    draft: atlas_acta::entities::comments::CommentAttachmentDraft,
) -> CommentDraftDto {
    CommentDraftDto {
        id: draft.id.0,
        expires_at: draft.expires_at,
    }
}

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/comment-drafts",
    operation_id = "create_task_comment_draft",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path),
        ("readable_id" = String, Path),
        ("x-create-token" = String, Header, description = "UUID replay token"),
    ),
    responses(
        (status = 201, body = CommentDraftDto),
        (status = 200, body = CommentDraftDto),
        (status = 404),
        (status = 409),
        (status = 410),
        (status = 422),
    )
)]
pub(crate) async fn create_comment_draft(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<CommentDraftDto>), ApiError> {
    let create_token = headers
        .get("x-create-token")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<uuid::Uuid>().ok())
        .ok_or_else(|| ApiError::InvalidInput {
            message: "x-create-token must be a UUID".into(),
        })?
        .to_string();
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let service =
        CommentDraftService::new(std::sync::Arc::new(state.comment_attachment_draft_repo()));
    let result = service
        .create_or_replay(
            &ctx,
            CommentOwner::Task(auth.resource.0.id),
            create_token,
            chrono::Utc::now() + chrono::Duration::hours(24),
        )
        .await
        .map_err(ApiError::Domain)?;

    let status = if result.replayed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    Ok((status, Json(comment_draft_to_dto(result.draft))))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}",
    operation_id = "cancel_task_comment_draft",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("readable_id" = String, Path), ("draft_id" = String, Path)),
    responses((status = 204), (status = 404), (status = 409), (status = 410))
)]
pub(crate) async fn cancel_comment_draft(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    Path(path): Path<CommentPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let draft_id = CommentDraftId(path.comment_id);
    let draft = state
        .comment_attachment_draft_repo()
        .get_for_owner_and_creator(&ctx, CommentOwner::Task(auth.resource.0.id), draft_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    if draft.state == atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized {
        return Err(ApiError::Domain(
            atlas_core::error::DomainError::CommentDraftConflict {
                reason: "draft is already finalized".into(),
            },
        ));
    }

    if draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Active {
        return Err(ApiError::Domain(
            atlas_core::error::DomainError::CommentDraftGone {
                reason: "draft is no longer active".into(),
            },
        ));
    }

    PgAttachmentLifecycle::cancel_draft(
        state.db.as_ref(),
        &ctx,
        draft_id,
        state.attachments.as_ref(),
    )
    .await
    .map_err(ApiError::Domain)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

pub(crate) fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(atlas_acta::actor::UserAttributionId(uid.0)),
        Principal::ApiKey(kid) => Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(kid.0)),
        Principal::Group(_) => Actor::User(atlas_acta::actor::UserAttributionId(uuid::Uuid::nil())),
    }
}

/// Loads the workspace's non-deleted property definitions that apply to tasks
/// (`applies_to` of `task` or `both`), used to schema-validate a task's custom
/// property values.
async fn task_property_definitions(
    state: &AppState,
    ctx: &WorkspaceCtx,
) -> Result<Vec<PropertyDefinition>, ApiError> {
    let repo = PgPropertyDefinitionRepo {
        conn: (*state.db).clone(),
    };

    let mut definitions = repo.list(ctx).await.map_err(ApiError::Domain)?;
    definitions.retain(|d| matches!(d.applies_to, AppliesTo::Task | AppliesTo::Both));

    Ok(definitions)
}

fn actor_to_dto(actor: &Actor) -> ActorDto {
    match actor {
        Actor::User(uid) => ActorDto {
            r#type: "user".into(),
            id: uid.0,
            display_name: None,
            key_type: None,
            account_status: None,
        },
        Actor::ApiKey(kid) => ActorDto {
            r#type: "api_key".into(),
            id: kid.0,
            display_name: None,
            key_type: None,
            account_status: None,
        },
    }
}

fn task_to_dto(t: Task, board_name: String, column_name: String) -> TaskDto {
    TaskDto {
        id: t.id.0,
        workspace_id: t.workspace_id.0,
        project_id: t.project_id.0,
        board_id: t.board_id.0,
        column_id: t.column_id.0,
        parent_task_id: t.parent_task_id.map(|p| p.0),
        readable_id: t.readable_id,
        title: t.title,
        description: t.description,
        priority: t.priority.map(|p| p.as_str().to_string()),
        due_date: t.due_date,
        estimate: t.estimate,
        labels: t.labels,
        properties: t.properties,
        created_by: actor_to_dto(&t.created_by),
        created_at: t.created_at,
        updated_at: t.updated_at,
        board_name,
        column_name,
    }
}

/// Builds a `ReferenceDto` from a `TaskReference` and pre-resolved liveness info.
///
/// `target_resolved` and `target_readable_id` must be computed by the caller
/// against the live DB state so that soft-deleted targets are treated as broken.
fn reference_to_dto(
    r: TaskReference,
    target_resolved: bool,
    target_readable_id: Option<String>,
    target_title: Option<String>,
) -> ReferenceDto {
    ReferenceDto {
        id: r.id.0,
        kind: r.kind.as_str().to_string(),
        target_task_id: r.target_task_id.map(|id| id.0),
        target_readable_id,
        target_document_id: r.target_document_id.map(|id| id.0),
        target_title,
        target_resolved,
        created_by: actor_to_dto(&r.created_by),
        created_at: r.created_at,
    }
}

fn manual_reference_to_unified_dto(
    r: TaskReference,
    target_resolved: bool,
    target_readable_id: Option<String>,
    target_title: Option<String>,
) -> UnifiedReferenceDto {
    UnifiedReferenceDto {
        id: r.id.0,
        origins: vec![ReferenceOriginDto::Manual],
        wikilink_reference_id: None,
        manual_reference_id: Some(r.id.0),
        manual_kind: Some(r.kind.as_str().to_string()),
        target_task_id: r.target_task_id.map(|id| id.0),
        target_readable_id,
        target_document_id: r.target_document_id.map(|id| id.0),
        target_title,
        target_resolved,
        manual_created_by: Some(actor_to_dto(&r.created_by)),
        manual_created_at: Some(r.created_at),
    }
}

async fn can_view_reference_target(
    state: &AppState,
    principal: &Principal,
    membership: Option<MemberRole>,
    workspace: &atlas_acta::entities::identity::Workspace,
    chain: &ResourceChain,
) -> Result<bool, ApiError> {
    Ok(matches!(
        resolve_effective_role(&state.db, principal, membership, workspace, chain).await?,
        Some(role) if role >= ResourceRole::Viewer
    ))
}

fn wikilink_to_unified_dto(
    link: RankedTaskDescriptionLink,
    target_resolved: bool,
    target_title: Option<String>,
) -> UnifiedReferenceDto {
    let target_document_id = link.link.target_document_id.map(|id| id.0);

    UnifiedReferenceDto {
        id: link.link.id.0,
        origins: vec![ReferenceOriginDto::Wikilink],
        wikilink_reference_id: Some(link.link.id.0),
        manual_reference_id: None,
        manual_kind: None,
        target_task_id: None,
        target_readable_id: None,
        target_document_id,
        target_title: target_title.or_else(|| {
            target_document_id
                .is_none()
                .then_some(link.link.target_title)
        }),
        target_resolved,
        manual_created_by: None,
        manual_created_at: None,
    }
}

fn attachment_to_dto(a: atlas_acta::entities::documents::Attachment) -> TaskAttachmentDto {
    let created_by = if let Some(uid) = a.created_by_user_id {
        actor_to_dto(&Actor::User(atlas_acta::actor::UserAttributionId(uid.0)))
    } else if let Some(kid) = a.created_by_api_key_id {
        actor_to_dto(&Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(
            kid.0,
        )))
    } else {
        actor_to_dto(&Actor::User(atlas_acta::actor::UserAttributionId(
            uuid::Uuid::nil(),
        )))
    };

    TaskAttachmentDto {
        id: a.id.0,
        file_name: a.file_name,
        content_type: a.content_type,
        size_bytes: a.size_bytes,
        created_by,
        created_at: a.created_at,
    }
}

fn assignee_ref_to_actor(r: AssigneeRef) -> Actor {
    match r {
        AssigneeRef::User(uid) => Actor::User(atlas_acta::actor::UserAttributionId(uid.0)),
        AssigneeRef::ApiKey(kid) => Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(kid.0)),
    }
}

use super::account_status;

/// Builds an `ActorDto` for attribution contexts (created_by, assigned_by, etc.).
///
/// Attribution actors carry only id and display name. `account_status` is always
/// absent here — it is a display signal for the assignee/member UI, not for audit
/// trails or activity feeds.
pub(crate) async fn resolve_actor_dto(
    state: &AppState,
    _ctx: &WorkspaceCtx,
    actor: &Actor,
) -> ActorDto {
    match actor {
        Actor::User(uid) => {
            let repo = PgUserRepo {
                conn: (*state.db).clone(),
            };
            let display_name = repo
                .find_by_id(UserId(uid.0))
                .await
                .ok()
                .flatten()
                .map(|u| u.display_name);
            ActorDto {
                r#type: "user".into(),
                id: uid.0,
                display_name,
                key_type: None,
                account_status: None,
            }
        }
        Actor::ApiKey(kid) => {
            let repo = PgApiKeyRepo {
                conn: (*state.db).clone(),
            };
            let key = repo.get_by_id(ApiKeyId(kid.0)).await.ok().flatten();
            ActorDto {
                r#type: "api_key".into(),
                id: kid.0,
                display_name: key.as_ref().map(|k| k.name.clone()),
                key_type: key.as_ref().map(|k| k.type_.as_str().to_string()),
                account_status: None,
            }
        }
    }
}

/// Builds an `ActorDto` for user-assignee display paths.
///
/// Unlike `resolve_actor_dto`, this function populates `account_status` for user
/// actors so the client can mark deactivated and pending assignees without hiding them.
/// Api-key actors get `account_status: None` (tier-2 principals use `revoked_at`).
async fn resolve_assignee_actor_dto(state: &AppState, actor: &Actor) -> ActorDto {
    match actor {
        Actor::User(uid) => {
            let repo = PgUserRepo {
                conn: (*state.db).clone(),
            };
            let user = repo.find_by_id(UserId(uid.0)).await.ok().flatten();
            ActorDto {
                r#type: "user".into(),
                id: uid.0,
                display_name: user.as_ref().map(|u| u.display_name.clone()),
                key_type: None,
                account_status: user
                    .as_ref()
                    .map(|u| account_status(u.disabled_at, u.activated_at).to_string()),
            }
        }
        Actor::ApiKey(kid) => {
            let repo = PgApiKeyRepo {
                conn: (*state.db).clone(),
            };
            let key = repo.get_by_id(ApiKeyId(kid.0)).await.ok().flatten();
            ActorDto {
                r#type: "api_key".into(),
                id: kid.0,
                display_name: key.as_ref().map(|k| k.name.clone()),
                key_type: key.as_ref().map(|k| k.type_.as_str().to_string()),
                account_status: None,
            }
        }
    }
}

async fn assignee_to_dto(state: &AppState, ctx: &WorkspaceCtx, a: TaskAssignee) -> AssigneeDto {
    AssigneeDto {
        assignee: resolve_assignee_actor_dto(state, &assignee_ref_to_actor(a.assignee)).await,
        assigned_by: resolve_actor_dto(state, ctx, &a.assigned_by).await,
        assigned_at: a.assigned_at,
    }
}

/// Batch-loads the assignees for a page of tasks and resolves their display
/// names in a fixed number of queries (the assignee rows, the referenced users,
/// and the workspace's api keys), grouped by task id — so a board listing never
/// issues one query per card.
async fn board_assignees_by_task(
    state: &AppState,
    ctx: &WorkspaceCtx,
    tasks: &[Task],
) -> Result<std::collections::HashMap<uuid::Uuid, Vec<ActorDto>>, ApiError> {
    use std::collections::HashMap;

    let task_ids: Vec<TaskId> = tasks.iter().map(|t| t.id).collect();

    let rows = PgTaskAssigneeRepo::new((*state.db).clone())
        .list_for_tasks(ctx, &task_ids)
        .await
        .map_err(ApiError::Domain)?;

    let mut user_ids: Vec<UserId> = Vec::new();
    let mut needs_keys = false;
    for r in &rows {
        match r.assignee {
            AssigneeRef::User(uid) => user_ids.push(uid),
            AssigneeRef::ApiKey(_) => needs_keys = true,
        }
    }
    user_ids.sort_by_key(|u| u.0);
    user_ids.dedup_by_key(|u| u.0);

    // list_by_ids returns the full User — carry it in the map so we can derive
    // account_status without an extra query.
    let user_map: HashMap<uuid::Uuid, atlas_custos::entities::identity::User> = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&user_ids)
    .await
    .map_err(ApiError::Domain)?
    .into_iter()
    .map(|u| (u.id.0, u))
    .collect();

    let key_info: HashMap<uuid::Uuid, (String, String)> = if needs_keys {
        PgApiKeyRepo {
            conn: (*state.db).clone(),
        }
        .list_granted_in_workspace(atlas_custos::WorkspaceScope(ctx.workspace_id.0))
        .await
        .map_err(ApiError::Domain)?
        .into_iter()
        .map(|k| (k.id.0, (k.name, k.type_.as_str().to_string())))
        .collect()
    } else {
        HashMap::new()
    };

    let mut by_task: HashMap<uuid::Uuid, Vec<ActorDto>> = HashMap::new();
    for r in rows {
        let actor = assignee_ref_to_actor(r.assignee);
        let dto = match &actor {
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
        by_task.entry(r.task_id.0).or_default().push(dto);
    }

    Ok(by_task)
}

/// Batch-loads the board name and column name for a page of tasks.
///
/// Issues two `IN (...)` queries — one for columns, one for boards — both
/// scoped to `ctx.workspace_id`, then builds a map from column id to
/// `(board_name, column_name)`. The caller can then populate `board_name` and
/// `column_name` on each `TaskSummaryDto` without an N+1 query.
///
/// When a column or board row cannot be found (data-integrity error), the
/// entry is absent from the returned map; callers substitute a fallback
/// so a missing row does not abort the listing.
///
/// Returns a map keyed by `column_id` → `(board_id, board_name, column_name)`.
async fn board_column_names_by_task(
    state: &AppState,
    ctx: &WorkspaceCtx,
    tasks: &[Task],
) -> Result<std::collections::HashMap<uuid::Uuid, (uuid::Uuid, String, String)>, ApiError> {
    use std::collections::HashMap;

    let mut col_ids: Vec<uuid::Uuid> = tasks.iter().map(|t| t.column_id.0).collect();
    col_ids.sort_unstable();
    col_ids.dedup();

    let repo = PgBoardRepo::new((*state.db).clone());

    let columns: Vec<BoardColumn> = repo
        .list_columns_by_ids(ctx.workspace_id.0, &col_ids)
        .await
        .map_err(ApiError::Domain)?;

    let mut board_ids: Vec<uuid::Uuid> = columns.iter().map(|c| c.board_id.0).collect();
    board_ids.sort_unstable();
    board_ids.dedup();

    let boards: Vec<Board> = repo
        .list_boards_by_ids(ctx.workspace_id.0, &board_ids)
        .await
        .map_err(ApiError::Domain)?;

    let board_names: HashMap<uuid::Uuid, String> =
        boards.into_iter().map(|b| (b.id.0, b.name)).collect();

    let by_column: HashMap<uuid::Uuid, (uuid::Uuid, String, String)> = columns
        .into_iter()
        .filter_map(|col| {
            let board_name = board_names.get(&col.board_id.0)?.clone();
            Some((col.id.0, (col.board_id.0, board_name, col.name)))
        })
        .collect();

    Ok(by_column)
}

/// Counts the direct sub-tasks of each task in a page in a single query.
///
/// Returns a map from task id to child count. Tasks with no sub-tasks are absent
/// from the map; callers default those to 0. Used to populate `subtask_count` on
/// each `TaskSummaryDto` without an N+1 query.
async fn subtask_counts_by_task(
    state: &AppState,
    ctx: &WorkspaceCtx,
    tasks: &[Task],
) -> Result<std::collections::HashMap<uuid::Uuid, i64>, ApiError> {
    let task_ids: Vec<TaskId> = tasks.iter().map(|t| t.id).collect();

    let counts = PgTaskRepo::new((*state.db).clone())
        .count_children_for_parents(ctx, &task_ids)
        .await
        .map_err(ApiError::Domain)?;

    Ok(counts
        .into_iter()
        .map(|(id, count)| (id.0, count))
        .collect())
}

fn checklist_item_to_dto(item: TaskChecklistItem) -> ChecklistItemDto {
    ChecklistItemDto {
        id: item.id.0,
        task_id: item.task_id.0,
        title: item.title,
        checked: item.checked,
        position_key: item.position_key,
        promoted_task_id: item.promoted_task_id.map(|id| id.0),
        promoted_readable_id: None,
        created_at: item.created_at,
        updated_at: item.updated_at,
    }
}

/// Batch-loads display info for all distinct actors in a page of activity entries
/// and converts them to DTOs with enriched actor fields (display_name, key_type,
/// account_status).
///
/// Activity actors use the same enrichment shape as assignee actors (display_name
/// + key_type + account_status) because the workspace activity feed is a
///   people-facing audit view where the deactivated/pending badge is meaningful.
///
/// `task_readable_id` is the same for all entries when called from the per-task
/// feed; the workspace feed caller supplies per-entry readable ids via the
/// `WorkspaceActivityRow` type.
async fn enrich_activity_entries(
    state: &AppState,
    ctx: &WorkspaceCtx,
    entries: Vec<TaskActivity>,
    shared_readable_id: &str,
) -> Result<Vec<ActivityEntryDto>, ApiError> {
    use std::collections::HashMap;

    let mut user_ids: Vec<UserId> = Vec::new();
    let mut needs_keys = false;

    for e in &entries {
        match &e.actor {
            Actor::User(uid) => user_ids.push(UserId(uid.0)),
            Actor::ApiKey(_) => needs_keys = true,
        }
    }

    user_ids.sort_by_key(|u| u.0);
    user_ids.dedup_by_key(|u| u.0);

    let user_map: HashMap<uuid::Uuid, atlas_custos::entities::identity::User> = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&user_ids)
    .await
    .map_err(ApiError::Domain)?
    .into_iter()
    .map(|u| (u.id.0, u))
    .collect();

    let key_info: HashMap<uuid::Uuid, (String, String)> = if needs_keys {
        PgApiKeyRepo {
            conn: (*state.db).clone(),
        }
        .list_granted_in_workspace(atlas_custos::WorkspaceScope(ctx.workspace_id.0))
        .await
        .map_err(ApiError::Domain)?
        .into_iter()
        .map(|k| (k.id.0, (k.name, k.type_.as_str().to_string())))
        .collect()
    } else {
        HashMap::new()
    };

    let dtos = entries
        .into_iter()
        .map(|a| {
            let actor_dto = match &a.actor {
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
            ActivityEntryDto {
                id: a.id.0,
                kind: a.kind.as_str().to_string(),
                actor: actor_dto,
                payload: serde_json::to_value(&a.payload).unwrap_or(serde_json::Value::Null),
                created_at: a.created_at,
                task_id: a.task_id.0,
                task_readable_id: shared_readable_id.to_string(),
            }
        })
        .collect();

    Ok(dtos)
}

/// Parses the `{assignee_ref}` path segment (`user:{uuid}` or `api_key:{uuid}`).
fn parse_assignee_ref(s: &str) -> Result<AssigneeRef, ApiError> {
    if let Some(rest) = s.strip_prefix("user:") {
        let uuid = rest.parse::<uuid::Uuid>().map_err(|_| ApiError::NotFound)?;
        return Ok(AssigneeRef::User(UserId(uuid)));
    }
    if let Some(rest) = s.strip_prefix("api_key:") {
        let uuid = rest.parse::<uuid::Uuid>().map_err(|_| ApiError::NotFound)?;
        return Ok(AssigneeRef::ApiKey(ApiKeyId(uuid)));
    }
    Err(ApiError::NotFound)
}

/// Checks that an `AssigneeRef` is a member of `ctx.workspace_id` and, for user
/// assignees, that the account is not disabled.
///
/// Rules:
/// - A user must have a membership row; missing → 404 (conceal existence).
/// - A disabled user (`disabled_at IS NOT NULL`) → 422 InvalidInput. Pending users
///   (activated_at IS NULL, not disabled) remain assignable.
/// - An api_key must have a grant in the workspace; missing → 404.
async fn validate_assignee_is_workspace_member(
    assignee: &AssigneeRef,
    ctx: &WorkspaceCtx,
    state: &AppState,
) -> Result<(), ApiError> {
    match assignee {
        AssigneeRef::User(uid) => {
            let membership_repo = PgMembershipRepo {
                conn: (*state.db).clone(),
            };
            let member = membership_repo
                .find(ctx, *uid)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;
            if member.is_none() {
                return Err(ApiError::NotFound);
            }

            let user_repo = PgUserRepo {
                conn: (*state.db).clone(),
            };
            let user = user_repo
                .find_by_id(*uid)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;
            if user.map(|u| u.disabled_at.is_some()).unwrap_or(false) {
                return Err(ApiError::InvalidInput {
                    message: "Cannot assign a deactivated user; re-enable the account first."
                        .to_string(),
                });
            }
        }
        AssigneeRef::ApiKey(kid) => {
            let grant_repo = PgPermissionGrantRepo {
                conn: (*state.db).clone(),
            };
            let has_grant = grant_repo
                .principal_has_any_grant_in_workspace(
                    atlas_custos::WorkspaceScope(ctx.workspace_id.0),
                    None,
                    Some(*kid),
                )
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;
            if !has_grant {
                return Err(ApiError::NotFound);
            }
        }
    }
    Ok(())
}

/// Parses `priority` from the wire representation (a nullable string).
fn parse_priority(val: Option<serde_json::Value>) -> Result<Option<Option<Priority>>, ApiError> {
    match val {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(s)) => {
            let p: Priority = s.parse().map_err(|_| ApiError::InvalidInput {
                message: format!("unknown priority: {s}; must be low|medium|high|urgent"),
            })?;
            Ok(Some(Some(p)))
        }
        _ => Err(ApiError::InvalidInput {
            message: "priority must be a string or null".into(),
        }),
    }
}

/// Parses `estimate` from the wire representation (a nullable non-negative integer).
fn parse_estimate(val: Option<serde_json::Value>) -> Result<Option<Option<i32>>, ApiError> {
    match val {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::Number(n)) => {
            let i = n.as_i64().ok_or_else(|| ApiError::InvalidInput {
                message: "estimate must be an integer".into(),
            })?;
            if i < 0 {
                return Err(ApiError::InvalidInput {
                    message: "estimate must be non-negative".into(),
                });
            }
            Ok(Some(Some(i32::try_from(i).map_err(|_| {
                ApiError::InvalidInput {
                    message: "estimate out of range".into(),
                }
            })?)))
        }
        _ => Err(ApiError::InvalidInput {
            message: "estimate must be a number or null".into(),
        }),
    }
}

/// Parses `due_date` from the wire representation (a nullable datetime string).
fn parse_due_date(
    val: Option<serde_json::Value>,
) -> Result<Option<Option<chrono::DateTime<chrono::Utc>>>, ApiError> {
    match val {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(s)) => {
            let dt =
                s.parse::<chrono::DateTime<chrono::Utc>>()
                    .map_err(|_| ApiError::InvalidInput {
                        message: format!("invalid due_date: {s}; must be RFC 3339"),
                    })?;
            Ok(Some(Some(dt)))
        }
        _ => Err(ApiError::InvalidInput {
            message: "due_date must be a string or null".into(),
        }),
    }
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/boards/{board_id}/tasks
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/boards/{board_id}/tasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
    ),
    request_body = CreateTaskRequest,
    responses(
        (status = 201, description = "Task created", body = CreateTaskResponseDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn create_task(
    auth: Authorized<BoardRes, EditorMin, TasksCreate>,
    State(state): State<AppState>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let board = &auth.resource.0;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let input = validate_task_input(
        &state,
        &auth.principal,
        auth.membership.clone(),
        &auth.workspace,
        &ctx,
        TaskInput {
            title: body.title,
            description: body.description,
            properties: body.properties,
            references: body.references,
        },
    )
    .await?;

    create_task_response(
        &state,
        &ctx,
        NewTask {
            project_id: board.project_id,
            board_id: board.id,
            column_id: ColumnId(body.column_id),
            title: input.title,
            description: input.description,
            priority: input.priority,
            due_date: input.due_date,
            estimate: input.estimate,
            labels: input.labels,
            properties: input.custom,
            position: PositionBetween {
                before: body.before,
                after: body.after,
            },
        },
        None,
        input.references,
    )
    .await
}

/// The task-shaped fields shared by task and sub-task creation.
struct TaskInput {
    title: String,
    description: Option<String>,
    properties: Option<atlas_api::dtos::boards_tasks::TaskPropertiesDto>,
    references: Vec<CreateReferenceRequest>,
}

/// `TaskInput` after validation, with every field in its domain representation
/// and every reference target resolved and permission-checked.
struct ValidatedTaskInput {
    title: String,
    description: String,
    priority: Option<Priority>,
    due_date: Option<chrono::DateTime<chrono::Utc>>,
    estimate: Option<i32>,
    labels: Vec<String>,
    custom: Option<serde_json::Value>,
    references: Vec<ResolvedReferenceTarget>,
}

/// Validates the fields a task creation accepts and resolves its initial
/// references. Shared by `create_task` and `create_subtask`: a sub-task is an
/// ordinary task, so it takes exactly the same input under exactly the same rules.
async fn validate_task_input(
    state: &AppState,
    principal: &Principal,
    membership: Option<MemberRole>,
    workspace: &Workspace,
    ctx: &WorkspaceCtx,
    input: TaskInput,
) -> Result<ValidatedTaskInput, ApiError> {
    validate_name("title", &input.title)?;

    let description = input.description.unwrap_or_default();
    validate_description(&description)?;

    let props = input.properties.unwrap_or_default();
    let priority = props
        .priority
        .as_deref()
        .map(|s| {
            s.parse::<Priority>().map_err(|_| ApiError::InvalidInput {
                message: format!("unknown priority: {s}; must be low|medium|high|urgent"),
            })
        })
        .transpose()?;

    if let Some(est) = props.estimate
        && est < 0
    {
        return Err(ApiError::InvalidInput {
            message: "estimate must be non-negative".into(),
        });
    }

    validate_labels(&props.labels)?;

    if input.references.len() > 100 {
        return Err(ApiError::InvalidInput {
            message: "references must contain at most 100 items".into(),
        });
    }

    if let Some(ref custom) = props.custom {
        validate_custom_entry_count(custom)?;
        let definitions = task_property_definitions(state, ctx).await?;
        validate_custom_properties(custom, &definitions)?;
    }

    let mut references = Vec::with_capacity(input.references.len());
    for reference in &input.references {
        references.push(
            resolve_reference_target(state, principal, membership.clone(), workspace, reference)
                .await?,
        );
    }

    Ok(ValidatedTaskInput {
        title: input.title,
        description,
        priority,
        due_date: props.due_date,
        estimate: props.estimate,
        labels: props.labels,
        custom: props.custom,
        references,
    })
}

/// Creates the task (or sub-task, when `parent` is set) with its resolved
/// references and renders the shared 201 response.
async fn create_task_response(
    state: &AppState,
    ctx: &WorkspaceCtx,
    new: NewTask,
    parent: Option<TaskId>,
    resolved_references: Vec<ResolvedReferenceTarget>,
) -> Result<(StatusCode, Json<CreateTaskResponseDto>), ApiError> {
    let created = state
        .task_service()
        .create_with_references_under(
            ctx,
            new,
            parent,
            resolved_references
                .iter()
                .map(|reference| InitialTaskReference {
                    kind: reference.kind.clone(),
                    target_task_id: reference.target_task_id,
                    target_document_id: reference.target_document_id,
                })
                .collect(),
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateTaskResponseDto {
            task: task_to_dto(created.task, String::new(), String::new()),
            references: created
                .references
                .into_iter()
                .zip(resolved_references)
                .map(|(reference, resolved)| {
                    reference_to_dto(
                        reference,
                        true,
                        resolved.target_readable_id,
                        resolved.target_title,
                    )
                })
                .collect(),
        }),
    ))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/boards/{board_id}/tasks
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/boards/{board_id}/tasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("board_id" = String, Path, description = "Board UUID"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Paginated task list", body = Page<TaskSummaryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Board not found"),
    )
)]
pub(crate) async fn list_tasks(
    auth: Authorized<BoardRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<TaskSummaryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskRepo::new((*state.db).clone());

    let mut all = repo
        .list_by_board(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);
    if let Some(id) = after_id
        && let Some(pos) = all.iter().position(|t| t.id.0 == id)
    {
        all = all.into_iter().skip(pos + 1).collect();
    }

    let has_more = all.len() > limit;
    if has_more {
        all.truncate(limit);
    }

    let next_cursor = if has_more {
        all.last().map(|t| Cursor(t.id.0))
    } else {
        None
    };

    let mut assignees_by_task = board_assignees_by_task(&state, &ctx, &all).await?;
    let board_column_names = board_column_names_by_task(&state, &ctx, &all).await?;
    let subtask_counts = subtask_counts_by_task(&state, &ctx, &all).await?;
    let board_name_fallback = auth.resource.0.name.clone();

    let board_id_fallback = auth.resource.0.id.0;

    let dtos = all
        .into_iter()
        .map(|t| {
            let (board_id, board_name, column_name) = board_column_names
                .get(&t.column_id.0)
                .cloned()
                .unwrap_or_else(|| {
                    (
                        board_id_fallback,
                        board_name_fallback.clone(),
                        String::new(),
                    )
                });

            TaskSummaryDto {
                id: t.id.0,
                readable_id: t.readable_id,
                board_id,
                column_id: t.column_id.0,
                title: t.title,
                priority: t.priority.map(|p| p.as_str().to_string()),
                estimate: t.estimate,
                labels: t.labels,
                assignees: assignees_by_task.remove(&t.id.0).unwrap_or_default(),
                board_name,
                column_name,
                subtask_count: subtask_counts.get(&t.id.0).copied().unwrap_or(0),
                updated_at: t.updated_at,
            }
        })
        .collect();

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID (e.g. ATL-42)"),
    ),
    responses(
        (status = 200, description = "Task", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn get_task(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
) -> Result<Json<TaskDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let board_column_names =
        board_column_names_by_task(&state, &ctx, std::slice::from_ref(&auth.resource.0)).await?;

    let (_board_id, board_name, column_name) = board_column_names
        .get(&auth.resource.0.column_id.0)
        .cloned()
        .unwrap_or_else(|| (auth.resource.0.board_id.0, String::new(), String::new()));

    Ok(Json(task_to_dto(auth.resource.0, board_name, column_name)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/tasks/{readable_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/workspaces/{ws}/tasks/{readable_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = UpdateTaskRequest,
    responses(
        (status = 200, description = "Task updated", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn update_task(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<TaskDto>, ApiError> {
    let task_id = auth.resource.0.id;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    if let Some(ref title) = body.title {
        validate_name("title", title)?;
    }

    if let Some(ref desc) = body.description {
        validate_description(desc)?;
    }

    if let Some(ref labels) = body.labels {
        validate_labels(labels)?;
    }

    let priority = parse_priority(body.priority)?;
    let due_date = parse_due_date(body.due_date)?;
    let estimate = parse_estimate(body.estimate)?;

    if let Some(ref props) = body.properties {
        validate_custom_entry_count(props)?;
        let definitions = task_property_definitions(&state, &ctx).await?;
        validate_custom_properties(props, &definitions)?;
    }

    let patch = TaskPatch {
        title: body.title,
        description: body.description,
        priority,
        due_date,
        estimate,
        labels: body.labels,
        properties: body.properties,
    };

    let updated = state
        .task_service()
        .patch(&ctx, task_id, patch)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_to_dto(updated, String::new(), String::new())))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/tasks/{readable_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/workspaces/{ws}/tasks/{readable_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 204, description = "Task deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn delete_task(
    auth: Authorized<TaskRes, EditorMin, TasksDelete>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    state
        .task_service()
        .delete_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/tasks/{readable_id}/move
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/move",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = MoveTaskRequest,
    responses(
        (status = 200, description = "Task moved", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Position exhausted — retry"),
    )
)]
pub(crate) async fn move_task(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<MoveTaskRequest>,
) -> Result<Json<TaskDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    // The Authorized<TaskRes, EditorMin> extractor only proves edit rights on the
    // task's current board. A move may target a column on a different board, so
    // independently require edit rights on the destination board too; otherwise a
    // user could relocate a task into a board they cannot access.
    authorize_board_destination(
        &state.db,
        &auth.principal,
        auth.membership.clone(),
        &auth.workspace,
        ColumnId(body.column_id),
        EditorMin::ROLE,
    )
    .await?;

    let moved = state
        .task_service()
        .move_task(
            &ctx,
            auth.resource.0.id,
            ColumnId(body.column_id),
            PositionBetween {
                before: body.before,
                after: body.after,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_to_dto(moved, String::new(), String::new())))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/assignees
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/assignees",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 200, description = "Assignee list", body = Vec<AssigneeDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_assignees(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
) -> Result<Json<Vec<AssigneeDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskAssigneeRepo::new((*state.db).clone());

    let items = repo
        .list_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let mut dtos = Vec::with_capacity(items.len());
    for item in items {
        dtos.push(assignee_to_dto(&state, &ctx, item).await);
    }

    Ok(Json(dtos))
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/tasks/{readable_id}/assignees
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/assignees",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = AddAssigneeRequest,
    responses(
        (status = 201, description = "Assignee added", body = AssigneeDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or principal not found"),
        (status = 409, description = "Assignee already added"),
    )
)]
pub(crate) async fn add_assignee(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<AddAssigneeRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let task_id = auth.resource.0.id;

    let assignee = match body.assignee_type.as_str() {
        "user" => AssigneeRef::User(UserId(body.assignee_id)),
        "api_key" => AssigneeRef::ApiKey(ApiKeyId(body.assignee_id)),
        _ => {
            return Err(ApiError::InvalidInput {
                message: "assignee_type must be 'user' or 'api_key'".into(),
            });
        }
    };

    validate_assignee_is_workspace_member(&assignee, &ctx, &state).await?;

    let result = state
        .task_service()
        .assign(&ctx, task_id, assignee)
        .await
        .map_err(|e| match e {
            atlas_core::error::DomainError::Forbidden { message }
                if message.contains("already") =>
            {
                ApiError::Conflict
            }
            other => ApiError::Domain(other),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(assignee_to_dto(&state, &ctx, result).await),
    ))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("assignee_ref" = String, Path, description = "user:{uuid} or api_key:{uuid}"),
    ),
    responses(
        (status = 204, description = "Assignee removed"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or assignee not found"),
    )
)]
pub(crate) async fn remove_assignee(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    Path(p): Path<AssigneePath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let assignee = parse_assignee_ref(&p.assignee_ref)?;

    state
        .task_service()
        .unassign(&ctx, auth.resource.0.id, assignee)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/references
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/references",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 200, description = "Reference list", body = Vec<UnifiedReferenceDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_references(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
) -> Result<Json<Vec<UnifiedReferenceDto>>, ApiError> {
    use std::collections::{HashMap, HashSet};

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskReferenceRepo::new((*state.db).clone());

    let refs = repo
        .list_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let task_repo = PgTaskRepo::new((*state.db).clone());
    let doc_repo = PgDocumentRepo::new((*state.db).clone(), 0);

    let target_task_ids: Vec<TaskId> = refs
        .iter()
        .filter_map(|r| r.target_task_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let target_doc_ids: Vec<DocumentId> = refs
        .iter()
        .filter_map(|r| r.target_document_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut live_task_targets: HashMap<TaskId, (String, String)> = HashMap::new();
    for tid in target_task_ids {
        if let Some(t) = task_repo.find(&ctx, tid).await.map_err(ApiError::Domain)? {
            let chain =
                build_board_chain(&state.db, &auth.workspace, t.board_id, t.project_id).await?;
            if can_view_reference_target(
                &state,
                &auth.principal,
                auth.membership.clone(),
                &auth.workspace,
                &chain,
            )
            .await?
            {
                live_task_targets.insert(tid, (t.readable_id, t.title));
            }
        }
    }

    let mut live_doc_titles: HashMap<DocumentId, String> = HashMap::new();
    for did in target_doc_ids {
        if let Some(doc) = doc_repo.get(&ctx, did).await.map_err(ApiError::Domain)? {
            let chain = build_document_chain(&state.db, &auth.workspace, &doc).await?;
            if can_view_reference_target(
                &state,
                &auth.principal,
                auth.membership.clone(),
                &auth.workspace,
                &chain,
            )
            .await?
            {
                live_doc_titles.insert(did, doc.title);
            }
        }
    }

    let mut dtos: Vec<_> = refs
        .into_iter()
        .map(|r| {
            let (target_resolved, target_readable_id, target_title) =
                match (r.target_task_id, r.target_document_id) {
                    (Some(tid), _) => {
                        let target = live_task_targets.get(&tid);
                        let target_readable_id = target.map(|(readable_id, _)| readable_id.clone());
                        let target_title = target.map(|(_, title)| title.clone());
                        (target.is_some(), target_readable_id, target_title)
                    }
                    (_, Some(did)) => {
                        let title = live_doc_titles.get(&did).cloned();
                        (title.is_some(), None, title)
                    }
                    _ => (false, None, None),
                };
            manual_reference_to_unified_dto(r, target_resolved, target_readable_id, target_title)
        })
        .collect();

    let link_repo = PgDocumentLinkRepo {
        conn: (*state.db).clone(),
    };
    let link_snapshot = link_repo
        .outgoing_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    if let Some(snapshot) = link_snapshot {
        for link in rank_task_description_links(&snapshot) {
            let target_document_id = link.link.target_document_id;

            if let Some(document_id) = target_document_id
                && let Some(manual) = dtos.iter_mut().find(|entry| {
                    entry.target_document_id == Some(document_id.0)
                        && entry.manual_reference_id.is_some()
                })
            {
                manual.origins.push(ReferenceOriginDto::Wikilink);
                manual.wikilink_reference_id = Some(link.link.id.0);
                continue;
            }

            let (target_resolved, target_title) = match target_document_id {
                Some(document_id) => match doc_repo
                    .get(&ctx, document_id)
                    .await
                    .map_err(ApiError::Domain)?
                {
                    Some(document) => {
                        let chain =
                            build_document_chain(&state.db, &auth.workspace, &document).await?;
                        let visible = can_view_reference_target(
                            &state,
                            &auth.principal,
                            auth.membership.clone(),
                            &auth.workspace,
                            &chain,
                        )
                        .await?;
                        (visible, visible.then_some(document.title))
                    }
                    None => (false, None),
                },
                None => (false, None),
            };

            dtos.push(wikilink_to_unified_dto(link, target_resolved, target_title));
        }
    }

    Ok(Json(dtos))
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/tasks/{readable_id}/references
// ---------------------------------------------------------------------------

struct ResolvedReferenceTarget {
    kind: atlas_acta::entities::boards_tasks::ReferenceKind,
    target_task_id: Option<TaskId>,
    target_document_id: Option<DocumentId>,
    target_readable_id: Option<String>,
    target_title: Option<String>,
}

async fn resolve_reference_target(
    state: &AppState,
    principal: &Principal,
    membership: Option<MemberRole>,
    workspace: &Workspace,
    body: &CreateReferenceRequest,
) -> Result<ResolvedReferenceTarget, ApiError> {
    use atlas_acta::entities::boards_tasks::ReferenceKind;

    let kind = match body.kind.as_str() {
        "relates" => ReferenceKind::Relates,
        "blocks" => ReferenceKind::Blocks,
        "parent" => ReferenceKind::Parent,
        "spec" => ReferenceKind::Spec,
        "docs" => ReferenceKind::Docs,
        other => {
            return Err(ApiError::InvalidInput {
                message: format!(
                    "unknown reference kind: {other}; must be relates|blocks|parent|spec|docs"
                ),
            });
        }
    };

    let has_task_target = body.target_task_readable_id.is_some();
    let has_document_target = body.target_document_id.is_some();
    if has_task_target == has_document_target {
        return Err(ApiError::InvalidInput {
            message:
                "exactly one of target_task_readable_id or target_document_id must be provided"
                    .into(),
        });
    }

    let ctx = WorkspaceCtx::new(workspace.id, principal_to_actor(principal));
    if let Some(readable_id) = &body.target_task_readable_id {
        let target = PgTaskRepo::new((*state.db).clone())
            .find_by_readable_id(&ctx, readable_id)
            .await
            .map_err(ApiError::Domain)?
            .ok_or(ApiError::NotFound)?;
        let chain =
            build_board_chain(&state.db, workspace, target.board_id, target.project_id).await?;
        if !can_view_reference_target(state, principal, membership, workspace, &chain).await? {
            return Err(ApiError::NotFound);
        }

        return Ok(ResolvedReferenceTarget {
            kind,
            target_task_id: Some(target.id),
            target_document_id: None,
            target_readable_id: Some(target.readable_id),
            target_title: Some(target.title),
        });
    }

    let document_id = DocumentId(body.target_document_id.ok_or(ApiError::NotFound)?);
    let target = PgDocumentRepo::new((*state.db).clone(), 0)
        .get(&ctx, document_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;
    let chain = build_document_chain(&state.db, workspace, &target).await?;
    if !can_view_reference_target(state, principal, membership, workspace, &chain).await? {
        return Err(ApiError::NotFound);
    }

    Ok(ResolvedReferenceTarget {
        kind,
        target_task_id: None,
        target_document_id: Some(document_id),
        target_readable_id: None,
        target_title: Some(target.title),
    })
}

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/references",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = CreateReferenceRequest,
    responses(
        (status = 201, description = "Reference created", body = ReferenceDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Duplicate reference"),
        (status = 422, description = "Invalid reference kind"),
    )
)]
/// Creates a reference from the authorized source task to exactly one target.
///
/// `task_references` targets are backed by foreign-key columns: the row cannot
/// be stored without a real target in this workspace. Both-null and both-set
/// bodies are rejected here as 422 before reaching the DB, preventing a CHECK
/// constraint violation or a silent both-null insert.
pub(crate) async fn create_reference(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<CreateReferenceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    use atlas_acta::entities::boards_tasks::ReferenceKind;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let kind: ReferenceKind = match body.kind.as_str() {
        "relates" => ReferenceKind::Relates,
        "blocks" => ReferenceKind::Blocks,
        "parent" => ReferenceKind::Parent,
        "spec" => ReferenceKind::Spec,
        "docs" => ReferenceKind::Docs,
        other => {
            return Err(ApiError::InvalidInput {
                message: format!(
                    "unknown reference kind: {other}; must be relates|blocks|parent|spec|docs"
                ),
            });
        }
    };

    let has_task_target = body.target_task_readable_id.is_some();
    let has_doc_target = body.target_document_id.is_some();

    if !has_task_target && !has_doc_target {
        return Err(ApiError::InvalidInput {
            message:
                "exactly one of target_task_readable_id or target_document_id must be provided"
                    .into(),
        });
    }
    if has_task_target && has_doc_target {
        return Err(ApiError::InvalidInput {
            message:
                "exactly one of target_task_readable_id or target_document_id must be provided"
                    .into(),
        });
    }

    let mut target_readable_id: Option<String> = None;
    let mut target_title: Option<String> = None;

    let target_task_id = if let Some(rid) = body.target_task_readable_id {
        let repo = PgTaskRepo::new((*state.db).clone());
        let found = repo
            .find_by_readable_id(&ctx, &rid)
            .await
            .map_err(ApiError::Domain)?;

        match found {
            Some(t) => {
                let chain =
                    build_board_chain(&state.db, &auth.workspace, t.board_id, t.project_id).await?;
                if !can_view_reference_target(
                    &state,
                    &auth.principal,
                    auth.membership.clone(),
                    &auth.workspace,
                    &chain,
                )
                .await?
                {
                    return Err(ApiError::NotFound);
                }

                target_readable_id = Some(t.readable_id.clone());
                target_title = Some(t.title);
                Some(t.id)
            }
            None => return Err(ApiError::NotFound),
        }
    } else {
        None
    };

    let target_document_id = if let Some(raw_id) = body.target_document_id {
        let doc_id = DocumentId(raw_id);
        let doc_repo = PgDocumentRepo::new((*state.db).clone(), 0);
        let found = doc_repo.get(&ctx, doc_id).await.map_err(ApiError::Domain)?;

        match found {
            Some(doc) => {
                let chain = build_document_chain(&state.db, &auth.workspace, &doc).await?;
                if !can_view_reference_target(
                    &state,
                    &auth.principal,
                    auth.membership.clone(),
                    &auth.workspace,
                    &chain,
                )
                .await?
                {
                    return Err(ApiError::NotFound);
                }

                target_title = Some(doc.title);
                Some(doc_id)
            }
            None => return Err(ApiError::NotFound),
        }
    } else {
        None
    };

    let reference = state
        .task_service()
        .add_reference(
            &ctx,
            NewTaskReference {
                source_task_id: auth.resource.0.id,
                kind,
                target_task_id,
                target_document_id,
            },
        )
        .await
        .map_err(|e| match e {
            atlas_core::error::DomainError::Forbidden { .. } => ApiError::Conflict,
            other => ApiError::Domain(other),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(reference_to_dto(
            reference,
            true,
            target_readable_id,
            target_title,
        )),
    ))
}

fn reference_batch_problem(index: usize, error: ApiError) -> CreateReferenceBatchResultDto {
    let (status, r#type, title, hint) = match error {
        ApiError::NotFound => (
            404,
            "urn:atlas:error:not-found".into(),
            "Not Found".into(),
            Some("Check the identifier — it may not exist or you may not have access.".into()),
        ),
        ApiError::InvalidInput { .. } | ApiError::Domain(DomainError::InvalidInput { .. }) => (
            422,
            "urn:atlas:error:invalid-input".into(),
            "Invalid Input".into(),
            None,
        ),
        ApiError::Conflict | ApiError::Domain(DomainError::AlreadyExists { .. }) => (
            409,
            "urn:atlas:error:already-exists".into(),
            "Already Exists".into(),
            Some("An item with this name already exists here — choose a different name.".into()),
        ),
        ApiError::Domain(DomainError::NotFound { .. }) => (
            404,
            "urn:atlas:error:not-found".into(),
            "Not Found".into(),
            Some("Check the identifier — it may not exist or you may not have access.".into()),
        ),
        _ => (
            500,
            "urn:atlas:error:internal".into(),
            "Internal Server Error".into(),
            None,
        ),
    };

    CreateReferenceBatchResultDto::Problem {
        index,
        status,
        r#type,
        title,
        hint,
    }
}

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/references/batch",
    tag = "tasks",
    security(("bearer_auth" = [])),
    request_body = CreateReferenceBatchRequest,
    responses(
        (status = 200, body = Vec<CreateReferenceBatchResultDto>),
        (status = 413),
        (status = 422),
    )
)]
pub(crate) async fn create_references_batch(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<CreateReferenceBatchRequest>,
) -> Result<Json<Vec<CreateReferenceBatchResultDto>>, ApiError> {
    if body.references.len() > 100 {
        return Err(ApiError::InvalidInput {
            message: "references must contain at most 100 items".into(),
        });
    }

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let mut results = Vec::with_capacity(body.references.len());

    for (index, request) in body.references.iter().enumerate() {
        let resolved = match resolve_reference_target(
            &state,
            &auth.principal,
            auth.membership.clone(),
            &auth.workspace,
            request,
        )
        .await
        {
            Ok(resolved) => resolved,
            Err(error) => {
                results.push(reference_batch_problem(index, error));
                continue;
            }
        };

        let result = state
            .task_service()
            .add_reference(
                &ctx,
                NewTaskReference {
                    source_task_id: auth.resource.0.id,
                    kind: resolved.kind,
                    target_task_id: resolved.target_task_id,
                    target_document_id: resolved.target_document_id,
                },
            )
            .await;

        match result {
            Ok(reference) => results.push(CreateReferenceBatchResultDto::Success {
                index,
                reference: reference_to_dto(
                    reference,
                    true,
                    resolved.target_readable_id,
                    resolved.target_title,
                ),
            }),
            Err(DomainError::Forbidden { .. }) => {
                results.push(reference_batch_problem(index, ApiError::Conflict));
            }
            Err(error) => results.push(reference_batch_problem(index, ApiError::Domain(error))),
        }
    }

    Ok(Json(results))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("reference_id" = String, Path, description = "Reference UUID"),
    ),
    responses(
        (status = 204, description = "Reference deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Reference not found"),
    )
)]
pub(crate) async fn delete_reference(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    Path(p): Path<ReferencePath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let ref_id = TaskReferenceId(p.reference_id);

    state
        .task_service()
        .remove_reference(&ctx, auth.resource.0.id, ref_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/tasks/{readable_id}/attachments
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/attachments",
    operation_id = "upload_task_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body(
        content = String,
        description = "Multipart form-data carrying the file in a part named `file`",
        content_type = "multipart/form-data"
    ),
    responses(
        (status = 201, description = "Attachment created", body = TaskAttachmentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 413, description = "Payload too large"),
        (status = 422, description = "Missing file part or invalid file name"),
    )
)]
/// Uploads a file and attaches it to the resolved task.
///
/// The body is `multipart/form-data` with the file in a part named `file`. Bytes are
/// streamed and accumulated up to `max_attachment_bytes`; the first chunk that would
/// exceed the cap aborts the upload with 413 so an oversize body is never fully
/// buffered. The stored blob is content-addressed, so re-uploading identical bytes
/// reuses the existing object.
pub(crate) async fn upload_attachment(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let max = state.max_attachment_bytes;

    let mut captured: Option<(String, String, Vec<u8>)> = None;

    while let Some(mut field) =
        multipart
            .next_field()
            .await
            .map_err(|e| ApiError::InvalidInput {
                message: format!("invalid multipart body: {e}"),
            })?
    {
        if field.name() != Some("file") {
            continue;
        }

        let file_name = field.file_name().map(|s| s.to_string()).unwrap_or_default();

        let content_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        validate_name("file_name", &file_name)?;

        let mut data: Vec<u8> = Vec::new();

        while let Some(chunk) = field.chunk().await.map_err(|e| ApiError::InvalidInput {
            message: format!("error reading upload: {e}"),
        })? {
            if data.len() as u64 + chunk.len() as u64 > max {
                return Err(ApiError::PayloadTooLarge {
                    message: format!("attachment exceeds maximum size of {max} bytes"),
                });
            }
            data.extend_from_slice(&chunk);
        }

        captured = Some((file_name, content_type, data));
        break;
    }

    let (file_name, content_type, data) = captured.ok_or_else(|| ApiError::InvalidInput {
        message: "multipart form must contain a 'file' part".into(),
    })?;

    validate_upload(
        &file_name,
        &data,
        state.upload_allowed_extensions.as_deref(),
    )?;

    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let attachment = PgAttachmentLifecycle::store_and_record(
        state.db.as_ref(),
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: Some(auth.resource.0.id),
            comment_id: None,
            file_name,
            content_type,
            size_bytes: data.len() as i64,
            sha256: String::new(),
        },
        &data,
        state.attachments.as_ref(),
    )
    .await
    .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(attachment_to_dto(attachment))))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/attachments
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/attachments",
    operation_id = "list_task_attachments",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 200, description = "Attachment list", body = Vec<TaskAttachmentDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_attachments(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskAttachmentDto>>, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let attachment_repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };

    let items = attachment_repo
        .list_for_owner(&ctx, AttachmentOwner::Task(auth.resource.0.id))
        .await
        .map_err(ApiError::Domain)?;

    let dtos = items.into_iter().map(attachment_to_dto).collect();
    Ok(Json(dtos))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content",
    operation_id = "download_task_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("attachment_id" = String, Path, description = "Attachment UUID"),
    ),
    responses(
        (status = 200, description = "Binary attachment content"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or attachment not found"),
    )
)]
/// Streams an attachment's bytes through the API for the resolved task.
///
/// The attachment must belong to this task; an attachment owned by another task (or
/// a document) resolves to 404 rather than leaking across owners. Bytes are fetched
/// from the configured `AttachmentStore`, so this works for both the disk and S3
/// backends.
pub(crate) async fn download_attachment(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    Path(p): Path<TaskAttachmentPath>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let attachment_repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };

    let attachment = attachment_repo
        .find(&ctx, AttachmentId(p.attachment_id))
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    if attachment.task_id != Some(auth.resource.0.id) {
        return Err(ApiError::NotFound);
    }

    let bytes = state
        .attachments
        .get(&attachment.sha256)
        .await
        .map_err(|e| match e {
            atlas_core::error::DomainError::NotFound { .. } => ApiError::NotFound,
            other => ApiError::Internal {
                message: other.to_string(),
            },
        })?;

    let content_disposition = content_disposition_attachment(&attachment.file_name);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, attachment.content_type.clone())
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(response)
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
    operation_id = "rename_task_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("attachment_id" = String, Path, description = "Attachment UUID"),
    ),
    request_body = RenameTaskAttachmentRequest,
    responses(
        (status = 200, description = "Attachment renamed", body = TaskAttachmentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or attachment not found"),
        (status = 422, description = "Invalid file name"),
    )
)]
/// Renames task attachment metadata without changing the stored object.
pub(crate) async fn rename_attachment(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    Path(p): Path<TaskAttachmentPath>,
    State(state): State<AppState>,
    Json(body): Json<RenameTaskAttachmentRequest>,
) -> Result<Json<TaskAttachmentDto>, ApiError> {
    validate_name("file_name", &body.file_name)?;
    let file_name = body.file_name.trim().to_owned();
    validate_upload_extension(&file_name, state.upload_allowed_extensions.as_deref())?;
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let attachment = PgAttachmentRepo {
        conn: (*state.db).clone(),
    }
    .rename_for_owner(
        &ctx,
        AttachmentId(p.attachment_id),
        AttachmentOwner::Task(auth.resource.0.id),
        file_name,
    )
    .await
    .map_err(|error| match error {
        atlas_core::error::DomainError::NotFound { .. } => ApiError::NotFound,
        other => ApiError::Domain(other),
    })?;

    Ok(Json(attachment_to_dto(attachment)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
    operation_id = "delete_task_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("attachment_id" = String, Path, description = "Attachment UUID"),
    ),
    responses(
        (status = 204, description = "Attachment deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or attachment not found"),
    )
)]
/// Soft-deletes a task attachment row.
///
/// Only the DB row is marked deleted; the content-addressed blob is left in place
/// because the same bytes may be referenced by other attachments.
pub(crate) async fn delete_attachment(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    Path(p): Path<TaskAttachmentPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));

    let attachment_repo = PgAttachmentRepo {
        conn: (*state.db).clone(),
    };

    let attachment = attachment_repo
        .find(&ctx, AttachmentId(p.attachment_id))
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    if attachment.task_id != Some(auth.resource.0.id) {
        return Err(ApiError::NotFound);
    }

    attachment_repo
        .soft_delete(&ctx, AttachmentId(p.attachment_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
    operation_id = "upload_task_comment_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("comment_id" = String, Path, description = "Comment UUID"),
    ),
    request_body(content = String, content_type = "multipart/form-data"),
    responses((status = 201, body = CommentAttachmentDto), (status = 404), (status = 413), (status = 422))
)]
pub(crate) async fn upload_comment_attachment(
    auth: Authorized<TaskRes, ViewerMin, TasksUpdate>,
    Path(path): Path<CommentPath>,
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    upload_comment_attachment_for_path(auth, path, state, headers, multipart, false).await
}

async fn upload_comment_attachment_for_path(
    auth: Authorized<TaskRes, ViewerMin, TasksUpdate>,
    path: CommentPath,
    state: AppState,
    headers: HeaderMap,
    mut multipart: Multipart,
    force_draft: bool,
) -> Result<impl IntoResponse, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let owner = CommentOwner::Task(auth.resource.0.id);
    let draft_id = CommentDraftId(path.comment_id);
    let comment = if force_draft {
        None
    } else {
        match PgCommentRepo::new((*state.db).clone())
            .get_for_owner(&ctx, owner, CommentId(path.comment_id))
            .await
        {
            Ok(comment) => Some(comment),
            Err(DomainError::NotFound { .. }) => None,
            Err(error) => return Err(ApiError::Domain(error)),
        }
    };
    let draft = if comment.is_none() {
        let draft = state
            .comment_attachment_draft_repo()
            .get_for_owner_and_creator(&ctx, owner, draft_id)
            .await
            .map_err(ApiError::Domain)?;

        if force_draft {
            Some(draft.ok_or(ApiError::NotFound)?)
        } else {
            draft
        }
    } else {
        None
    };
    let can_moderate = matches!(
        auth.membership,
        Some(MemberRole::Owner) | Some(MemberRole::Admin)
    );
    if let Some(comment) = &comment
        && comment.created_by != ctx.actor
        && !can_moderate
    {
        return Err(ApiError::Domain(
            atlas_core::error::DomainError::Forbidden {
                message:
                    "only the comment's author or a workspace admin/owner may manage attachments"
                        .into(),
            },
        ));
    }

    let max = state.max_attachment_bytes;
    let mut captured: Option<(String, String, Vec<u8>)> = None;
    while let Some(mut field) =
        multipart
            .next_field()
            .await
            .map_err(|e| ApiError::InvalidInput {
                message: format!("invalid multipart body: {e}"),
            })?
    {
        if field.name() != Some("file") {
            continue;
        }
        let file_name = field
            .file_name()
            .map(ToString::to_string)
            .unwrap_or_default();
        let content_type = field
            .content_type()
            .map(ToString::to_string)
            .unwrap_or_else(|| "application/octet-stream".to_string());
        validate_name("file_name", &file_name)?;
        let mut data = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(|e| ApiError::InvalidInput {
            message: format!("error reading upload: {e}"),
        })? {
            if data.len() as u64 + chunk.len() as u64 > max {
                return Err(ApiError::PayloadTooLarge {
                    message: format!("attachment exceeds maximum size of {max} bytes"),
                });
            }
            data.extend_from_slice(&chunk);
        }
        captured = Some((file_name, content_type, data));
        break;
    }
    let (file_name, content_type, data) = captured.ok_or_else(|| ApiError::InvalidInput {
        message: "multipart form must contain a 'file' part".into(),
    })?;
    validate_upload(
        &file_name,
        &data,
        state.upload_allowed_extensions.as_deref(),
    )?;
    if let Some(draft) = draft {
        if draft.state == atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized {
            return Err(ApiError::Domain(
                atlas_core::error::DomainError::CommentDraftConflict {
                    reason: "draft is already finalized".into(),
                },
            ));
        }

        if draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Active {
            return Err(ApiError::Domain(
                atlas_core::error::DomainError::CommentDraftGone {
                    reason: "draft is no longer active".into(),
                },
            ));
        }
        let upload_token = headers
            .get("x-upload-token")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<uuid::Uuid>().ok())
            .ok_or_else(|| ApiError::InvalidInput {
                message: "x-upload-token must be a UUID".into(),
            })?
            .to_string();
        let metadata =
            CommentDraftMetadata::normalize(&file_name, &content_type).map_err(ApiError::Domain)?;
        let payload_digest = sha2::Sha256::digest(&data).to_vec();
        let request_digest = sha2::Sha256::digest(comment_draft_upload_digest_input(
            draft.id.0,
            &upload_token,
            &metadata.file_name,
            &metadata.content_type,
            data.len() as i64,
            &payload_digest,
        ))
        .to_vec();
        let (attachment, replayed) = PgAttachmentLifecycle::store_and_record_draft(
            state.db.as_ref(),
            &ctx,
            owner,
            draft.id,
            NewCommentAttachmentDraftUpload {
                attachment_id: None,
                upload_token,
                request_digest,
                payload_digest,
                metadata,
                size_bytes: data.len() as i64,
            },
            &data,
            state.attachments.as_ref(),
        )
        .await
        .map_err(ApiError::Domain)?;
        let url = format!(
            "/api/workspaces/{}/tasks/{}/comments/{}/attachments/{attachment_id}/content",
            auth.workspace.slug,
            auth.resource.0.readable_id,
            draft.id.0,
            attachment_id = attachment.id.0,
        );
        let mut dto = comment_attachment_to_dto(attachment);
        dto.comment_id = draft.id.0;
        dto.url = Some(url.clone());
        dto.markdown = Some(comment_attachment_markdown(
            &dto.file_name,
            &dto.content_type,
            &url,
        ));
        let status = if replayed {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        };
        return Ok((status, Json(dto)));
    }

    let comment_id = comment
        .map(|comment| comment.id)
        .ok_or_else(|| ApiError::Internal {
            message: "published comment was not resolved".into(),
        })?;
    let attachment = PgAttachmentLifecycle::store_and_record(
        state.db.as_ref(),
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: None,
            comment_id: Some(comment_id),
            file_name,
            content_type,
            size_bytes: data.len() as i64,
            sha256: String::new(),
        },
        &data,
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
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
    operation_id = "upload_task_comment_draft_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path),
        ("readable_id" = String, Path),
        ("draft_id" = String, Path),
        ("x-upload-token" = String, Header, description = "UUID replay token"),
    ),
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 201, body = CommentAttachmentDto),
        (status = 200, body = CommentAttachmentDto),
        (status = 404),
        (status = 409),
        (status = 410),
        (status = 413),
        (status = 422),
    )
)]
pub(crate) async fn upload_comment_draft_attachment(
    auth: Authorized<TaskRes, ViewerMin, TasksUpdate>,
    path: Path<CommentPath>,
    state: State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    upload_comment_attachment_for_path(auth, path.0, state.0, headers, multipart, true).await
}

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
    operation_id = "list_task_comment_attachments",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("readable_id" = String, Path), ("comment_id" = String, Path)),
    responses(
        (status = 200, body = Vec<CommentAttachmentDto>),
        (status = 404),
        (status = 410, description = "Draft attachment is terminal"),
    )
)]
pub(crate) async fn list_comment_attachments(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    Path(path): Path<CommentPath>,
    State(state): State<AppState>,
) -> Result<Json<Vec<CommentAttachmentDto>>, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let comment_id = CommentId(path.comment_id);
    let draft_id = CommentDraftId(path.comment_id);
    let draft_repo = state.comment_attachment_draft_repo();

    if let Some(draft) = draft_repo
        .get_for_owner_and_creator(&ctx, CommentOwner::Task(auth.resource.0.id), draft_id)
        .await
        .map_err(ApiError::Domain)?
    {
        if draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Active
            && draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized
        {
            return Err(ApiError::Domain(
                atlas_core::error::DomainError::CommentDraftGone {
                    reason: "draft is no longer active".into(),
                },
            ));
        }

        if draft.state == atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized {
            // Fall through to the published comment owner below.
        } else {
            let items = PgAttachmentLifecycle::list_active_draft_attachments(
                state.db.as_ref(),
                &ctx,
                CommentOwner::Task(auth.resource.0.id),
                draft_id,
            )
            .await
            .map_err(ApiError::Domain)?;

            return Ok(Json(
                items
                    .into_iter()
                    .map(|attachment| {
                        let url = format!(
                            "/api/workspaces/{}/tasks/{}/comments/{}/attachments/{}/content",
                            auth.workspace.slug,
                            auth.resource.0.readable_id,
                            draft_id.0,
                            attachment.id.0,
                        );
                        comment_attachment_to_dto_with_url(attachment, draft_id.0, url)
                    })
                    .collect(),
            ));
        }
    }

    PgCommentRepo::new((*state.db).clone())
        .get_for_owner(&ctx, CommentOwner::Task(auth.resource.0.id), comment_id)
        .await
        .map_err(ApiError::Domain)?;
    let items = PgAttachmentRepo {
        conn: (*state.db).clone(),
    }
    .list_for_owner(&ctx, AttachmentOwner::Comment(comment_id))
    .await
    .map_err(ApiError::Domain)?;
    Ok(Json(
        items
            .into_iter()
            .map(|attachment| {
                let url = format!(
                    "/api/workspaces/{}/tasks/{}/comments/{}/attachments/{}/content",
                    auth.workspace.slug, auth.resource.0.readable_id, comment_id.0, attachment.id.0,
                );
                comment_attachment_to_dto_with_url(attachment, comment_id.0, url)
            })
            .collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
    operation_id = "download_task_comment_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("readable_id" = String, Path), ("comment_id" = String, Path), ("attachment_id" = String, Path)),
    responses(
        (status = 200, description = "Binary attachment content", content_type = "application/octet-stream", headers(
            ("Content-Type" = String, description = "Stored attachment media type"),
            ("Content-Disposition" = String, description = "RFC 5987 attachment filename"),
            ("X-Content-Type-Options" = String, description = "Always nosniff"),
        )),
        (status = 404),
        (status = 410),
    )
)]
pub(crate) async fn download_comment_attachment(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    Path(path): Path<CommentAttachmentPath>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let comment_id = CommentId(path.comment_id);
    let draft_id = CommentDraftId(path.comment_id);
    let draft_repo = state.comment_attachment_draft_repo();

    if let Some(draft) = draft_repo
        .get_for_owner_and_creator(&ctx, CommentOwner::Task(auth.resource.0.id), draft_id)
        .await
        .map_err(ApiError::Domain)?
    {
        if draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Active
            && draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized
        {
            return Err(ApiError::Domain(
                atlas_core::error::DomainError::CommentDraftGone {
                    reason: "draft is no longer active".into(),
                },
            ));
        }

        if draft.state == atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized {
            // Fall through to the published comment owner below.
        } else {
            let attachment = PgAttachmentLifecycle::find_active_draft_attachment(
                state.db.as_ref(),
                &ctx,
                CommentOwner::Task(auth.resource.0.id),
                draft_id,
                AttachmentId(path.attachment_id),
            )
            .await
            .map_err(ApiError::Domain)?;

            return comment_attachment_response(&state, attachment).await;
        }
    }

    PgCommentRepo::new((*state.db).clone())
        .get_for_owner(&ctx, CommentOwner::Task(auth.resource.0.id), comment_id)
        .await
        .map_err(ApiError::Domain)?;
    if PgAttachmentLifecycle::is_tombstoned_draft_attachment(
        state.db.as_ref(),
        draft_id,
        AttachmentId(path.attachment_id),
    )
    .await
    .map_err(ApiError::Domain)?
    {
        return Err(ApiError::Domain(
            atlas_core::error::DomainError::CommentDraftGone {
                reason: "draft attachment was deleted".into(),
            },
        ));
    }
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
    path = "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
    operation_id = "delete_task_comment_attachment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path), ("readable_id" = String, Path), ("comment_id" = String, Path), ("attachment_id" = String, Path)),
    responses(
        (status = 204),
        (status = 404),
        (status = 410, description = "Draft attachment is terminal"),
    )
)]
pub(crate) async fn delete_comment_attachment(
    auth: Authorized<TaskRes, ViewerMin, TasksUpdate>,
    Path(path): Path<CommentAttachmentPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let ctx = WorkspaceCtx::new(auth.workspace.id, principal_to_actor(&auth.principal));
    let comment_id = CommentId(path.comment_id);
    let draft_id = CommentDraftId(path.comment_id);
    let draft_repo = state.comment_attachment_draft_repo();

    if let Some(draft) = draft_repo
        .get_for_owner_and_creator(&ctx, CommentOwner::Task(auth.resource.0.id), draft_id)
        .await
        .map_err(ApiError::Domain)?
    {
        if draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Active
            && draft.state != atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized
        {
            return Err(ApiError::Domain(
                atlas_core::error::DomainError::CommentDraftGone {
                    reason: "draft is no longer active".into(),
                },
            ));
        }

        if draft.state == atlas_acta::entities::comments::CommentAttachmentDraftState::Finalized {
            // Fall through to the published comment owner below.
        } else {
            PgAttachmentLifecycle::delete_draft_attachment(
                state.db.as_ref(),
                &ctx,
                CommentOwner::Task(auth.resource.0.id),
                draft_id,
                AttachmentId(path.attachment_id),
                state.attachments.as_ref(),
            )
            .await
            .map_err(ApiError::Domain)?;

            return Ok(StatusCode::NO_CONTENT);
        }
    }

    let comment = PgCommentRepo::new((*state.db).clone())
        .get_for_owner(&ctx, CommentOwner::Task(auth.resource.0.id), comment_id)
        .await
        .map_err(ApiError::Domain)?;
    let can_moderate = matches!(
        auth.membership,
        Some(MemberRole::Owner) | Some(MemberRole::Admin)
    );
    if comment.created_by != ctx.actor && !can_moderate {
        return Err(ApiError::Domain(
            atlas_core::error::DomainError::Forbidden {
                message:
                    "only the comment's author or a workspace admin/owner may manage attachments"
                        .into(),
            },
        ));
    }
    let attachment_id = AttachmentId(path.attachment_id);
    if PgAttachmentLifecycle::is_tombstoned_draft_attachment(
        state.db.as_ref(),
        draft_id,
        attachment_id,
    )
    .await
    .map_err(ApiError::Domain)?
    {
        return Err(ApiError::Domain(
            atlas_core::error::DomainError::CommentDraftGone {
                reason: "draft attachment was deleted".into(),
            },
        ));
    }
    PgAttachmentLifecycle::delete_comment_attachment(
        state.db.as_ref(),
        &ctx,
        comment_id,
        attachment_id,
    )
    .await
    .map_err(ApiError::Domain)?;
    Ok(StatusCode::NO_CONTENT)
}

fn comment_attachment_to_dto(
    attachment: atlas_acta::entities::documents::Attachment,
) -> CommentAttachmentDto {
    let comment_id = attachment
        .comment_id
        .map(|id| id.0)
        .unwrap_or_else(uuid::Uuid::nil);

    comment_attachment_to_dto_with_comment_id(attachment, comment_id)
}

fn comment_attachment_to_dto_with_comment_id(
    attachment: atlas_acta::entities::documents::Attachment,
    comment_id: uuid::Uuid,
) -> CommentAttachmentDto {
    CommentAttachmentDto {
        id: attachment.id.0,
        comment_id,
        file_name: attachment.file_name,
        content_type: attachment.content_type,
        size_bytes: attachment.size_bytes,
        sha256: attachment.sha256,
        actor: attachment
            .created_by_user_id
            .map(|id| actor_to_dto(&Actor::User(atlas_acta::actor::UserAttributionId(id.0))))
            .or_else(|| {
                attachment.created_by_api_key_id.map(|id| {
                    actor_to_dto(&Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(id.0)))
                })
            }),
        created_at: attachment.created_at,
        url: None,
        markdown: None,
    }
}

fn comment_attachment_to_dto_with_url(
    attachment: atlas_acta::entities::documents::Attachment,
    comment_id: uuid::Uuid,
    url: String,
) -> CommentAttachmentDto {
    let mut dto = comment_attachment_to_dto_with_comment_id(attachment, comment_id);
    dto.markdown = Some(comment_attachment_markdown(
        &dto.file_name,
        &dto.content_type,
        &url,
    ));
    dto.url = Some(url);
    dto
}

async fn comment_attachment_response(
    state: &AppState,
    attachment: atlas_acta::entities::documents::Attachment,
) -> Result<Response, ApiError> {
    let bytes = state
        .attachments
        .get(&attachment.sha256)
        .await
        .map_err(|error| match error {
            atlas_core::error::DomainError::NotFound { .. } => ApiError::NotFound,
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

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/graph
// ---------------------------------------------------------------------------

/// Query parameters for the task graph read.
#[derive(Debug, Deserialize)]
pub(crate) struct TaskGraphQuery {
    /// Traversal depth in edges. Default 2, clamped to [1, 5].
    pub depth: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/graph",
    operation_id = "get_task_graph",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("depth" = Option<u32>, Query, description = "Traversal depth in edges. Default 2, clamped to [1,5]"),
    ),
    responses(
        (status = 200, description = "Reachable references, subtasks and linked documents", body = TaskGraphDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn get_task_graph(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
    Query(params): Query<TaskGraphQuery>,
) -> Result<Json<TaskGraphDto>, ApiError> {
    let depth = task_graph::resolve_depth(params.depth);
    let root = auth.resource.0.id.0;
    let explorer = TaskGraphExplorer::new((*state.db).clone());

    let mut depths: std::collections::HashMap<uuid::Uuid, u32> =
        std::collections::HashMap::from([(root, 0)]);
    let mut visible: std::collections::HashSet<uuid::Uuid> =
        std::collections::HashSet::from([root]);
    let mut tasks = vec![root];
    let mut documents: Vec<uuid::Uuid> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut frontier = vec![root];
    let mut truncated = false;

    for level in 1..=depth {
        if frontier.is_empty() {
            break;
        }

        let found = explorer
            .expand(auth.workspace.id, &frontier)
            .await
            .map_err(ApiError::Domain)?;
        edges.extend(found.edges);

        let fresh_tasks = only_new(&found.tasks, &visible);
        let fresh_documents = only_new(&found.documents, &visible);
        let (allowed_tasks, allowed_documents) =
            authorize_graph_nodes(&state, &auth, &fresh_tasks, &fresh_documents).await?;

        // The budget is checked before admitting a level, so a huge fan-out
        // reports truncation instead of returning a partial level that reads
        // like the whole neighbourhood.
        if visible.len() + allowed_tasks.len() + allowed_documents.len() > task_graph::MAX_NODES {
            truncated = true;
            break;
        }

        for id in allowed_tasks.iter().chain(allowed_documents.iter()) {
            visible.insert(*id);
            depths.entry(*id).or_insert(level);
        }
        tasks.extend(allowed_tasks.iter().copied());
        documents.extend(allowed_documents.iter().copied());
        frontier = allowed_tasks;
    }

    let nodes = explorer
        .hydrate(auth.workspace.id, &depths, &tasks, &documents)
        .await
        .map_err(ApiError::Domain)?;
    // Hydration is also a liveness filter: a node deleted between traversal and
    // read has no row, and an edge to it must not survive either.
    let hydrated: std::collections::HashSet<uuid::Uuid> =
        nodes.iter().map(|node| node.id).collect();

    Ok(Json(TaskGraphDto {
        root,
        edges: task_graph::retain_edges_within(edges, &hydrated)
            .into_iter()
            .map(|edge| TaskGraphEdgeDto {
                from: edge.from,
                to: edge.to,
                kind: edge.kind,
            })
            .collect(),
        nodes: nodes
            .into_iter()
            .map(|node| TaskGraphNodeDto {
                id: node.id,
                kind: match node.kind {
                    GraphNodeKind::Task => "task".to_owned(),
                    GraphNodeKind::Document => "document".to_owned(),
                },
                readable_id: node.readable_id,
                document_slug: node.document_slug,
                title: node.title,
                column_name: node.column_name,
                depth: node.depth,
            })
            .collect(),
        truncated,
    }))
}

fn only_new(
    candidates: &[uuid::Uuid],
    seen: &std::collections::HashSet<uuid::Uuid>,
) -> Vec<uuid::Uuid> {
    let mut fresh: Vec<uuid::Uuid> = candidates
        .iter()
        .copied()
        .filter(|id| !seen.contains(id))
        .collect();
    fresh.sort_unstable();
    fresh.dedup();
    fresh
}

/// Keeps only the discovered nodes this caller may read.
///
/// A reference is visible to whoever can read the referencing task; its target
/// carries its own authority. Without this the graph would disclose the
/// existence, title and status of tasks in projects the caller has no access to.
async fn authorize_graph_nodes(
    state: &AppState,
    auth: &Authorized<TaskRes, ViewerMin, TasksRead>,
    tasks: &[uuid::Uuid],
    documents: &[uuid::Uuid],
) -> Result<(Vec<uuid::Uuid>, Vec<uuid::Uuid>), ApiError> {
    if tasks.is_empty() && documents.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let subjects: Vec<ProjectionSubject> = tasks
        .iter()
        .map(|id| ProjectionSubject::Task(*id))
        .chain(documents.iter().map(|id| ProjectionSubject::Document(*id)))
        .collect();

    let decisions =
        BatchAuthorizationService::new(PgBatchAuthorizationSource::new((*state.db).clone()))
            .authorize(auth.projection_context(), &subjects)
            .await
            .map_err(ApiError::Domain)?;

    let mut allowed_tasks = Vec::new();
    let mut allowed_documents = Vec::new();
    for (subject, allowed) in subjects.into_iter().zip(decisions) {
        if !allowed {
            continue;
        }
        match subject {
            ProjectionSubject::Task(id) => allowed_tasks.push(id),
            ProjectionSubject::Document(id) => allowed_documents.push(id),
            _ => {}
        }
    }
    Ok((allowed_tasks, allowed_documents))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/backlinks
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/backlinks",
    operation_id = "list_task_backlinks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Inbound reference list", body = Page<TaskBacklinkDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_backlinks(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<TaskBacklinkDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let ref_repo = PgTaskReferenceRepo::new((*state.db).clone());

    let mut inbound = ref_repo
        .list_inbound(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);
    if let Some(id) = after_id
        && let Some(pos) = inbound.iter().position(|r| r.id.0 == id)
    {
        inbound = inbound.into_iter().skip(pos + 1).collect();
    }

    let has_more = inbound.len() > limit;
    if has_more {
        inbound.truncate(limit);
    }

    let next_cursor = if has_more {
        inbound.last().map(|r| Cursor(r.id.0))
    } else {
        None
    };

    let source_ids = inbound
        .iter()
        .map(|reference| reference.source_task_id.0)
        .collect::<Vec<_>>();
    let sources = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(task::Column::DeletedAt.is_null())
        .filter(task::Column::Id.is_in(source_ids))
        .all(&*state.db)
        .await
        .map_err(|error| ApiError::Internal {
            message: error.to_string(),
        })?
        .into_iter()
        .map(|source| (source.id, source))
        .collect::<std::collections::HashMap<_, _>>();

    let mut dtos = Vec::with_capacity(inbound.len());
    for r in inbound {
        if let Some(t) = sources.get(&r.source_task_id.0) {
            dtos.push(TaskBacklinkDto {
                source_task_id: t.id,
                source_readable_id: t.readable_id.clone(),
                source_title: t.title.clone(),
                kind: r.kind.as_str().to_string(),
                comment_source: None,
                document_source: None,
            });
        }
    }

    let comment_links = PgCommentLinkRepo::new((*state.db).clone())
        .backlinks_for_target(&ctx, CommentLinkTarget::Task(auth.resource.0.id))
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
        dtos.push(TaskBacklinkDto {
            source_task_id: link.comment_id.0,
            source_readable_id: String::new(),
            source_title: String::new(),
            kind: "comment".into(),
            comment_source: Some(CommentBacklinkSourceDto {
                kind: "comment".into(),
                comment_id: link.comment_id.0,
                parent,
            }),
            document_source: None,
        });
    }

    dtos.extend(wikilink_backlinks(&state, &ctx, auth.resource.0.id).await?);

    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

/// Builds the `[[task:…]]` wikilink backlinks pointing at a task.
///
/// Like the comment-sourced rows above, these are appended outside the
/// reference cursor: the endpoint paginates over typed references only.
async fn wikilink_backlinks(
    state: &AppState,
    ctx: &WorkspaceCtx,
    target: TaskId,
) -> Result<Vec<TaskBacklinkDto>, ApiError> {
    let links = PgDocumentLinkRepo {
        conn: (*state.db).clone(),
    }
    .backlinks_for_task(ctx, target)
    .await
    .map_err(ApiError::Domain)?;

    let task_sources = load_task_sources(state, ctx, &links).await?;
    let document_sources = load_document_sources(state, ctx, &links).await?;

    let mut dtos = Vec::with_capacity(links.len());
    for link in links {
        if let Some(source) = link.source_task_id.and_then(|id| task_sources.get(&id.0)) {
            dtos.push(TaskBacklinkDto {
                source_task_id: source.id,
                source_readable_id: source.readable_id.clone(),
                source_title: source.title.clone(),
                kind: "wikilink".into(),
                comment_source: None,
                document_source: None,
            });
            continue;
        }

        if let Some(source) = link
            .source_document_id
            .and_then(|id| document_sources.get(&id.0))
        {
            dtos.push(TaskBacklinkDto {
                source_task_id: source.id,
                source_readable_id: String::new(),
                source_title: String::new(),
                kind: "wikilink".into(),
                comment_source: None,
                document_source: Some(DocumentBacklinkSourceDto {
                    document_id: source.id,
                    slug: source.slug.clone(),
                    title: source.title.clone(),
                }),
            });
        }
    }

    Ok(dtos)
}

async fn load_task_sources(
    state: &AppState,
    ctx: &WorkspaceCtx,
    links: &[atlas_acta::entities::documents::DocumentLink],
) -> Result<std::collections::HashMap<uuid::Uuid, task::Model>, ApiError> {
    let ids = links
        .iter()
        .filter_map(|link| link.source_task_id.map(|id| id.0))
        .collect::<Vec<_>>();

    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    Ok(task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(task::Column::DeletedAt.is_null())
        .filter(task::Column::Id.is_in(ids))
        .all(&*state.db)
        .await
        .map_err(|error| ApiError::Internal {
            message: error.to_string(),
        })?
        .into_iter()
        .map(|source| (source.id, source))
        .collect())
}

async fn load_document_sources(
    state: &AppState,
    ctx: &WorkspaceCtx,
    links: &[atlas_acta::entities::documents::DocumentLink],
) -> Result<std::collections::HashMap<uuid::Uuid, document::Model>, ApiError> {
    let ids = links
        .iter()
        .filter_map(|link| link.source_document_id.map(|id| id.0))
        .collect::<Vec<_>>();

    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    Ok(document::Entity::find()
        .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(document::Column::DeletedAt.is_null())
        .filter(document::Column::Id.is_in(ids))
        .all(&*state.db)
        .await
        .map_err(|error| ApiError::Internal {
            message: error.to_string(),
        })?
        .into_iter()
        .map(|source| (source.id, source))
        .collect())
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/checklist
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/checklist",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    responses(
        (status = 200, description = "Checklist items", body = Vec<ChecklistItemDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_checklist(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ChecklistItemDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskChecklistRepo::new((*state.db).clone());

    let items = repo
        .list_for_task(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(items.into_iter().map(checklist_item_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/tasks/{readable_id}/checklist
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/checklist",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = CreateChecklistItemRequest,
    responses(
        (status = 201, description = "Checklist item created", body = ChecklistItemDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn create_checklist_item(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<CreateChecklistItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    validate_name("title", &body.title)?;

    let item = state
        .task_service()
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: auth.resource.0.id,
                title: body.title,
                position: PositionBetween {
                    before: body.before,
                    after: body.after,
                },
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(checklist_item_to_dto(item))))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("item_id" = String, Path, description = "Checklist item UUID"),
    ),
    request_body = UpdateChecklistItemRequest,
    responses(
        (status = 200, description = "Checklist item updated", body = ChecklistItemDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Item not found"),
        (status = 409, description = "Position exhausted — retry"),
    )
)]
pub(crate) async fn update_checklist_item(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    Path(p): Path<ChecklistItemPath>,
    State(state): State<AppState>,
    Json(body): Json<UpdateChecklistItemRequest>,
) -> Result<Json<ChecklistItemDto>, ApiError> {
    use atlas_acta::entities::boards_tasks::TaskChecklistItemPatch;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    if let Some(ref title) = body.title {
        validate_name("title", title)?;
    }
    let item_id = ChecklistItemId(p.item_id);

    let position = if body.before.is_some() || body.after.is_some() {
        Some(PositionBetween {
            before: body.before,
            after: body.after,
        })
    } else {
        None
    };

    let item = state
        .task_service()
        .patch_checklist_item(
            &ctx,
            auth.resource.0.id,
            item_id,
            TaskChecklistItemPatch {
                title: body.title,
                checked: body.checked,
                position,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(checklist_item_to_dto(item)))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("item_id" = String, Path, description = "Checklist item UUID"),
    ),
    responses(
        (status = 204, description = "Checklist item deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Item not found"),
    )
)]
pub(crate) async fn delete_checklist_item(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    Path(p): Path<ChecklistItemPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    state
        .task_service()
        .remove_checklist_item(&ctx, auth.resource.0.id, ChecklistItemId(p.item_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("item_id" = String, Path, description = "Checklist item UUID"),
    ),
    request_body = PromoteChecklistItemRequest,
    responses(
        (status = 201, description = "Checklist item promoted to task", body = PromotionDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Item not found"),
        (status = 409, description = "Item already promoted"),
    )
)]
pub(crate) async fn promote_checklist_item(
    auth: Authorized<TaskRes, EditorMin, TasksCreate>,
    Path(p): Path<ChecklistItemPath>,
    State(state): State<AppState>,
    Json(body): Json<PromoteChecklistItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let parent_task = &auth.resource.0;

    if body.board_id != parent_task.board_id.0 {
        return Err(ApiError::InvalidInput {
            message: "promoted task must stay on the parent task's board".into(),
        });
    }

    let result = state
        .task_service()
        .promote_checklist_item(
            &ctx,
            parent_task.id,
            ChecklistItemId(p.item_id),
            parent_task.project_id,
            BoardId(body.board_id),
            ColumnId(body.column_id),
        )
        .await
        .map_err(|e| match e {
            atlas_core::error::DomainError::Forbidden { message }
                if message.contains("already been promoted") =>
            {
                ApiError::Conflict
            }
            other => ApiError::Domain(other),
        })?;

    let promoted_readable_id = result.task.readable_id.clone();
    let dto = PromotionDto {
        task: task_to_dto(result.task, String::new(), String::new()),
        parent_reference: Some(reference_to_dto(
            result.parent_reference,
            true,
            Some(promoted_readable_id),
            None,
        )),
        checklist_item: checklist_item_to_dto(result.checklist_item),
    };

    Ok((StatusCode::CREATED, Json(dto)))
}

// ---------------------------------------------------------------------------
// Sub-tasks (child tasks)
// ---------------------------------------------------------------------------

/// Builds `TaskSummaryDto`s for the given tasks, resolving assignees and board/
/// column names in a fixed number of batch queries.
async fn tasks_to_summaries(
    state: &AppState,
    ctx: &WorkspaceCtx,
    tasks: Vec<Task>,
) -> Result<Vec<TaskSummaryDto>, ApiError> {
    let mut assignees_by_task = board_assignees_by_task(state, ctx, &tasks).await?;
    let board_column_names = board_column_names_by_task(state, ctx, &tasks).await?;
    let subtask_counts = subtask_counts_by_task(state, ctx, &tasks).await?;

    Ok(tasks
        .into_iter()
        .map(|t| {
            let (board_id, board_name, column_name) = board_column_names
                .get(&t.column_id.0)
                .cloned()
                .unwrap_or_default();

            TaskSummaryDto {
                id: t.id.0,
                readable_id: t.readable_id,
                board_id,
                column_id: t.column_id.0,
                title: t.title,
                priority: t.priority.map(|p| p.as_str().to_string()),
                estimate: t.estimate,
                labels: t.labels,
                assignees: assignees_by_task.remove(&t.id.0).unwrap_or_default(),
                board_name,
                column_name,
                subtask_count: subtask_counts.get(&t.id.0).copied().unwrap_or(0),
                updated_at: t.updated_at,
            }
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/subtasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Parent task readable ID"),
    ),
    responses(
        (status = 200, description = "Sub-tasks of the task", body = Vec<TaskSummaryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_subtasks(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskSummaryDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTaskRepo::new((*state.db).clone());

    let children = repo
        .list_children(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    let dtos = tasks_to_summaries(&state, &ctx, children).await?;
    Ok(Json(dtos))
}

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/subtasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Parent task readable ID"),
    ),
    request_body = CreateSubtaskRequest,
    responses(
        (status = 201, description = "Sub-task created", body = CreateTaskResponseDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn create_subtask(
    auth: Authorized<TaskRes, EditorMin, TasksCreate>,
    State(state): State<AppState>,
    Json(body): Json<CreateSubtaskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let parent = &auth.resource.0;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let input = validate_task_input(
        &state,
        &auth.principal,
        auth.membership.clone(),
        &auth.workspace,
        &ctx,
        TaskInput {
            title: body.title,
            description: body.description,
            properties: body.properties,
            references: body.references,
        },
    )
    .await?;

    create_task_response(
        &state,
        &ctx,
        NewTask {
            project_id: parent.project_id,
            board_id: parent.board_id,
            column_id: body.column_id.map_or(parent.column_id, ColumnId),
            title: input.title,
            description: input.description,
            priority: input.priority,
            due_date: input.due_date,
            estimate: input.estimate,
            labels: input.labels,
            properties: input.custom,
            position: PositionBetween {
                before: body.before,
                after: body.after,
            },
        },
        Some(parent.id),
        input.references,
    )
    .await
}

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/parent",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Readable ID of the task to convert"),
    ),
    request_body = SetTaskParentRequest,
    responses(
        (status = 200, description = "Task converted into a sub-task", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task or parent not found"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn set_task_parent(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<SetTaskParentRequest>,
) -> Result<Json<TaskDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    // Resolved through the same visibility check as a reference target, so a
    // caller cannot attach a task to a parent on a board they cannot see.
    let parent = resolve_reference_target(
        &state,
        &auth.principal,
        auth.membership.clone(),
        &auth.workspace,
        &CreateReferenceRequest {
            kind: "parent".into(),
            target_task_readable_id: Some(body.parent_readable_id),
            target_document_id: None,
        },
    )
    .await?;

    let parent_id = parent.target_task_id.ok_or(ApiError::NotFound)?;

    let task = state
        .task_service()
        .set_parent(&ctx, auth.resource.0.id, parent_id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_to_dto(task, String::new(), String::new())))
}

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/promote",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Sub-task readable ID"),
    ),
    responses(
        (status = 200, description = "Sub-task promoted to a board task", body = TaskDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn promote_subtask(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
) -> Result<Json<TaskDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let task = state
        .task_service()
        .promote_subtask(&ctx, auth.resource.0.id)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(task_to_dto(task, String::new(), String::new())))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/activity
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/activity",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
    ),
    responses(
        (status = 200, description = "Activity feed", body = Page<ActivityEntryDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_activity(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Json<Page<ActivityEntryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let after_id = q
        .cursor
        .as_deref()
        .and_then(Cursor::decode)
        .map(|c| TaskActivityId(c.0));

    let task = &auth.resource.0;
    let task_id = task.id;
    let task_readable_id = task.readable_id.clone();

    let mut entries = state
        .task_service()
        .list_activity(&ctx, task_id, after_id, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = entries.len() > limit as usize;
    if has_more {
        entries.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        entries.last().map(|a| Cursor(a.id.0))
    } else {
        None
    };

    let dtos = enrich_activity_entries(&state, &ctx, entries, &task_readable_id).await?;
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks/{readable_id}/comments
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks/{readable_id}/comments",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200)"),
        ("feed" = Option<String>, Query, description = "Set to `full` for authorized links and retained events"),
    ),
    responses(
        (status = 200, description = "Task comments, oldest first. `feed=full` returns authorized links and retained events.", body = CommentListResponseDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
    )
)]
pub(crate) async fn list_comments(
    auth: Authorized<TaskRes, ViewerMin, TasksRead>,
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Result<Response, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let task_id = auth.resource.0.id;

    if q.feed.as_deref() == Some("full") {
        let after = decode_feed_cursor(q.cursor.as_deref())?;
        let (entries, next_cursor, has_more) = project_comment_feed(
            &state,
            &ctx,
            CommentOwner::Task(task_id),
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
        .task_service()
        .list_comments(&ctx, task_id, after_id, limit + 1)
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

    let dtos = enrich_comment_entries(&state, CommentOwner::Task(task_id), entries).await?;

    Ok(Json(Page::new(dtos, next_cursor, has_more)).into_response())
}

// ---------------------------------------------------------------------------
// POST /api/workspaces/{ws}/tasks/{readable_id}/comments
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/workspaces/{ws}/tasks/{readable_id}/comments",
    operation_id = "create_task_comment",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
    ),
    request_body = CreateCommentRequest,
    responses(
        (status = 201, description = "Comment created", body = CommentDto),
        (status = 200, description = "Draft comment finalization replay", body = CommentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Draft finalization conflict"),
        (status = 410, description = "Draft is terminal"),
        (status = 422, description = "Comment body is blank or exceeds the maximum length"),
    )
)]
pub(crate) async fn create_comment(
    auth: Authorized<TaskRes, EditorMin, TasksUpdate>,
    State(state): State<AppState>,
    Json(body): Json<CreateCommentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    validate_comment_body(&body.body)?;

    let task_id = auth.resource.0.id;

    let (comment, status) = if let Some(draft_id) = body.draft_id {
        let result = state
            .task_service()
            .finalize_comment_draft(&ctx, task_id, CommentDraftId(draft_id), body.body)
            .await
            .map_err(ApiError::Domain)?;
        (
            result.comment,
            if result.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            },
        )
    } else {
        (
            state
                .task_service()
                .add_comment(&ctx, task_id, body.body)
                .await
                .map_err(ApiError::Domain)?,
            StatusCode::CREATED,
        )
    };

    let dto = comment_to_dto(&state, &ctx, CommentOwner::Task(task_id), comment).await;
    Ok((status, Json(dto)))
}

// ---------------------------------------------------------------------------
// PATCH /api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("comment_id" = String, Path, description = "Comment UUID"),
    ),
    request_body = UpdateCommentRequest,
    responses(
        (status = 200, description = "Comment updated", body = CommentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Only the comment's author may edit it"),
        (status = 404, description = "Task or comment not found"),
        (status = 422, description = "Comment body is blank or exceeds the maximum length"),
    )
)]
pub(crate) async fn update_comment(
    auth: Authorized<TaskRes, ViewerMin, TasksUpdate>,
    Path(p): Path<CommentPath>,
    State(state): State<AppState>,
    Json(body): Json<UpdateCommentRequest>,
) -> Result<Json<CommentDto>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    validate_comment_body(&body.body)?;

    let task_id = auth.resource.0.id;

    let comment = state
        .task_service()
        .update_comment(&ctx, task_id, CommentId(p.comment_id), body.body)
        .await
        .map_err(ApiError::Domain)?;

    let dto = comment_to_dto(&state, &ctx, CommentOwner::Task(task_id), comment).await;
    Ok(Json(dto))
}

// ---------------------------------------------------------------------------
// DELETE /api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("readable_id" = String, Path, description = "Task readable ID"),
        ("comment_id" = String, Path, description = "Comment UUID"),
    ),
    responses(
        (status = 204, description = "Comment deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Neither the comment's author nor a workspace admin/owner"),
        (status = 404, description = "Task or comment not found"),
    )
)]
pub(crate) async fn delete_comment(
    auth: Authorized<TaskRes, ViewerMin, TasksUpdate>,
    Path(p): Path<CommentPath>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);

    let can_moderate = matches!(
        auth.membership,
        Some(MemberRole::Owner) | Some(MemberRole::Admin)
    );

    state
        .task_service()
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

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/activity
// ---------------------------------------------------------------------------

/// Query parameters for the workspace activity feed.
#[derive(Deserialize)]
pub(crate) struct WorkspaceActivityQuery {
    pub actor: Option<String>,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/activity",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("actor" = Option<String>, Query, description = "Actor type filter: 'user' or 'api_key'"),
        ("from" = Option<String>, Query, description = "Lower bound (inclusive) on created_at (ISO 8601)"),
        ("to" = Option<String>, Query, description = "Upper bound (inclusive) on created_at (ISO 8601)"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200, default 50)"),
    ),
    responses(
        (status = 200, description = "Access-filtered workspace activity feed", body = Page<ActivityEntryDto>),
        (status = 400, description = "Invalid query parameter"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
    )
)]
pub(crate) async fn list_workspace_activity(
    member: WorkspaceMember,
    State(state): State<AppState>,
    Query(q): Query<WorkspaceActivityQuery>,
) -> Result<Json<Page<ActivityEntryDto>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;

    let actor = match (&member.user, &member.api_key_id) {
        (Some(user), _) => Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
        (None, Some(kid)) => Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(kid.0)),
        _ => return Err(ApiError::Unauthorized),
    };

    // The workspace activity feed is entirely task-family. Gate an API-key
    // principal on `tasks:read` so a scope-restricted agent cannot observe task
    // activity it lacks read capability for. Humans and root pass unchanged.
    if let Some(key_id) = member.api_key_id {
        enforce_api_key_scope(
            &state.db,
            key_id,
            Capability {
                family: CapabilityFamily::Tasks,
                action: CapabilityAction::Read,
            },
        )
        .await?;
    }

    let actor_type = q
        .actor
        .as_deref()
        .map(|s| match s {
            "user" => Ok(ActorTypeFilter::User),
            "api_key" => Ok(ActorTypeFilter::ApiKey),
            other => Err(ApiError::InvalidInput {
                message: format!("invalid actor filter '{other}'; must be 'user' or 'api_key'"),
            }),
        })
        .transpose()?;

    // Keyset cursor: "(created_at_micros,id)" encoded as a SearchCursor.
    let after = q
        .cursor
        .as_deref()
        .map(|s| {
            SearchCursor::decode(s).ok_or_else(|| ApiError::InvalidInput {
                message: "invalid cursor".to_string(),
            })
        })
        .transpose()?
        .map(|sc| {
            let micros = match sc.key {
                SortKey::Updated(m) => m,
                SortKey::Relevance(_) => {
                    return Err(ApiError::InvalidInput {
                        message: "cursor is not compatible with activity listing".to_string(),
                    });
                }
            };
            let ts = chrono::DateTime::from_timestamp_micros(micros).ok_or_else(|| {
                ApiError::InvalidInput {
                    message: "invalid cursor timestamp".to_string(),
                }
            })?;
            Ok((ts, TaskActivityId(sc.id)))
        })
        .transpose()?;

    let ctx = WorkspaceCtx::new(member.workspace.id, actor);

    // The access-filter invariant: compute which projects/boards the caller can see
    // by calling the real permissions::resolve() per project, NOT by re-encoding
    // authz rules in SQL. The SQL filter uses the resulting id sets.
    let scope = compute_workspace_activity_scope(&state, &member, &ctx).await?;

    let filters = WorkspaceActivityFilters {
        actor_type,
        from: q.from,
        to: q.to,
    };

    let repo = PgTaskActivityRepo {
        conn: (*state.db).clone(),
    };
    let mut rows = repo
        .list_for_workspace(&ctx, scope, filters, after, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        rows.last().map(|r| {
            let micros = r.activity.created_at.timestamp_micros();
            SearchCursor {
                key: SortKey::Updated(micros),
                id: r.activity.id.0,
            }
        })
    } else {
        None
    };

    let (activities, readable_ids): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .map(|r| (r.activity, r.task_readable_id))
        .unzip();

    let dtos = enrich_workspace_activity_entries(&state, &ctx, activities, readable_ids).await?;
    Ok(Json(Page::new_search(dtos, next_cursor, has_more)))
}

/// Determines which projects and boards the caller can access within the workspace.
///
/// This is the load-bearing security boundary for the workspace activity feed.
/// Access is computed by calling the real `permissions::resolve()` per project,
/// using a single grant load from `list_all_for_principal_in_workspace`. This
/// approach is intentional: we do NOT re-encode permission rules in SQL. The
/// SQL filter uses the resulting id sets as a pure data filter.
///
/// Admin bypass (Owner/Admin membership, or root/system_admin) skips the per-project
/// loop and returns `is_admin = true`, which causes the SQL to omit id-set filtering.
async fn compute_workspace_activity_scope(
    state: &AppState,
    member: &WorkspaceMember,
    ctx: &WorkspaceCtx,
) -> Result<WorkspaceActivityScope, ApiError> {
    let is_admin = match (&member.user, &member.membership) {
        (Some(user), _) if user.is_root || user.is_system_admin => true,
        (_, Some(m)) if matches!(m.role, MemberRole::Owner | MemberRole::Admin) => true,
        _ => false,
    };

    if is_admin {
        return Ok(WorkspaceActivityScope {
            is_admin: true,
            project_ids: vec![],
            board_ids: vec![],
        });
    }

    let (principal, user_id, api_key_id) = match (&member.user, &member.api_key_id) {
        (Some(user), _) => (Principal::User(user.id), Some(user.id), None),
        (None, Some(kid)) => (Principal::ApiKey(*kid), None, Some(*kid)),
        _ => {
            return Err(ApiError::Unauthorized);
        }
    };

    let membership_role = member.membership.as_ref().map(|m| m.role.clone());

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    // Load all grants for this principal across the workspace once. The port
    // returns the opaque `atlas_core::ids::ResourceRef`; decode back to the
    // Acta enum `resolve()` compares against `ChainSegment.resource`.
    let raw_grants = grant_repo
        .list_all_for_principal_in_workspace(
            atlas_custos::WorkspaceScope(ctx.workspace_id.0),
            user_id,
            api_key_id,
        )
        .await
        .map_err(ApiError::Domain)?;

    let mut all_grants = Vec::with_capacity(raw_grants.len());
    for (resource_ref, role) in raw_grants {
        let resource =
            atlas_acta::permissions::resource_ref_codec::from_core(&resource_ref, ctx.workspace_id)
                .map_err(ApiError::Domain)?;
        all_grants.push((resource, role));
    }

    // Enumerate all projects and resolve access per project.
    let project_repo = PgProjectRepo {
        conn: (*state.db).clone(),
    };
    let all_projects = project_repo.list(ctx).await.map_err(ApiError::Domain)?;

    let mut accessible_project_ids = Vec::new();
    let mut accessible_board_ids: Vec<BoardId> = Vec::new();

    // Extract board-only grants from the flat grant list.
    for (resource, _role) in &all_grants {
        if let ResourceRef::Board(bid) = resource {
            accessible_board_ids.push(*bid);
        }
    }

    for project in &all_projects {
        let vis = project.visibility.clone();
        let chain = ResourceChain {
            segments: vec![
                ChainSegment {
                    resource: ResourceRef::Project(project.id),
                    visibility: Some(vis),
                },
                ChainSegment {
                    resource: ResourceRef::Workspace,
                    visibility: None,
                },
            ],
        };

        let input = ResolutionInput {
            principal: &principal,
            membership: membership_role.clone(),
            chain: &chain,
            grants: &all_grants,
        };

        if resolve(&input).is_some() {
            accessible_project_ids.push(project.id);
        }
    }

    Ok(WorkspaceActivityScope {
        is_admin: false,
        project_ids: accessible_project_ids,
        board_ids: accessible_board_ids,
    })
}

/// Batch-loads actor enrichment for workspace activity rows where each entry may
/// have a different task_readable_id.
async fn enrich_workspace_activity_entries(
    state: &AppState,
    ctx: &WorkspaceCtx,
    activities: Vec<TaskActivity>,
    readable_ids: Vec<String>,
) -> Result<Vec<ActivityEntryDto>, ApiError> {
    use std::collections::HashMap;

    let mut user_ids: Vec<UserId> = Vec::new();
    let mut needs_keys = false;

    for a in &activities {
        match &a.actor {
            Actor::User(uid) => user_ids.push(UserId(uid.0)),
            Actor::ApiKey(_) => needs_keys = true,
        }
    }

    user_ids.sort_by_key(|u| u.0);
    user_ids.dedup_by_key(|u| u.0);

    let user_map: HashMap<uuid::Uuid, atlas_custos::entities::identity::User> = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .list_by_ids(&user_ids)
    .await
    .map_err(ApiError::Domain)?
    .into_iter()
    .map(|u| (u.id.0, u))
    .collect();

    let key_info: HashMap<uuid::Uuid, (String, String)> = if needs_keys {
        PgApiKeyRepo {
            conn: (*state.db).clone(),
        }
        .list_granted_in_workspace(atlas_custos::WorkspaceScope(ctx.workspace_id.0))
        .await
        .map_err(ApiError::Domain)?
        .into_iter()
        .map(|k| (k.id.0, (k.name, k.type_.as_str().to_string())))
        .collect()
    } else {
        HashMap::new()
    };

    let dtos = activities
        .into_iter()
        .zip(readable_ids)
        .map(|(a, task_readable_id)| {
            let actor_dto = match &a.actor {
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
            ActivityEntryDto {
                id: a.id.0,
                kind: a.kind.as_str().to_string(),
                actor: actor_dto,
                payload: serde_json::to_value(&a.payload).unwrap_or(serde_json::Value::Null),
                created_at: a.created_at,
                task_id: a.task_id.0,
                task_readable_id,
            }
        })
        .collect();

    Ok(dtos)
}

// ---------------------------------------------------------------------------
// GET /api/workspaces/{ws}/tasks
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/workspaces/{ws}/tasks",
    tag = "tasks",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("assignee" = Option<String>, Query, description = "Assignee filter: 'me', 'user:{uuid}', or 'api_key:{uuid}'"),
        ("actor" = Option<String>, Query, description = "Actor type filter: 'user' or 'api_key'"),
        ("column_id" = Option<String>, Query, description = "Filter by column id (repeat for multiple)"),
        ("priority" = Option<String>, Query, description = "Filter by priority (repeat for multiple)"),
        ("label" = Option<String>, Query, description = "Filter by label (repeat for multiple)"),
        ("board_id" = Option<String>, Query, description = "Filter by board id"),
        ("sort" = Option<String>, Query, description = "Sort order: updated_at_desc (default), updated_at_asc, created_at_desc, created_at_asc, priority_desc, title_asc"),
        ("cursor" = Option<String>, Query, description = "Pagination cursor (34-char base64url)"),
        ("limit" = Option<u32>, Query, description = "Page size (max 200, default 50)"),
    ),
    responses(
        (status = 200, description = "Paginated workspace task list", body = Page<TaskSummaryDto>),
        (status = 400, description = "Invalid query parameter (e.g. unknown sort)"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
    )
)]
pub(crate) async fn list_workspace_tasks(
    member: WorkspaceMember,
    State(state): State<AppState>,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
) -> Result<Json<Page<TaskSummaryDto>>, ApiError> {
    let q = parse_workspace_task_query(raw_query.as_deref().unwrap_or(""))?;

    let actor = match (&member.user, &member.api_key_id) {
        (Some(user), _) => Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
        (None, Some(key_id)) => Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(key_id.0)),
        (None, None) => return Err(ApiError::Unauthorized),
    };

    if let Some(key_id) = member.api_key_id {
        enforce_api_key_scope(
            &state.db,
            key_id,
            Capability {
                family: CapabilityFamily::Tasks,
                action: CapabilityAction::Read,
            },
        )
        .await?;
    }

    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;

    let sort = q
        .sort
        .as_deref()
        .map(|s| {
            TaskSort::from_param_str(s).ok_or_else(|| ApiError::BadRequest {
                message: format!("invalid sort key '{s}'"),
            })
        })
        .transpose()?;

    let priorities = q
        .priorities
        .iter()
        .map(|p| {
            p.parse::<Priority>().map_err(|_| ApiError::InvalidInput {
                message: format!("invalid priority '{p}'"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let column_ids = q
        .column_ids
        .iter()
        .map(|s| {
            s.parse::<uuid::Uuid>()
                .map(atlas_acta::ids::ColumnId)
                .map_err(|_| ApiError::InvalidInput {
                    message: format!("invalid column_id '{s}'"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let board_id = q
        .board_id
        .as_deref()
        .map(|s| {
            s.parse::<uuid::Uuid>()
                .map(atlas_acta::ids::BoardId)
                .map_err(|_| ApiError::InvalidInput {
                    message: format!("invalid board_id '{s}'"),
                })
        })
        .transpose()?;

    let assignee = q
        .assignee
        .as_deref()
        .map(parse_workspace_assignee_filter)
        .transpose()?;

    let actor_type = q
        .actor
        .as_deref()
        .map(|s| match s {
            "user" => Ok(ActorTypeFilter::User),
            "api_key" => Ok(ActorTypeFilter::ApiKey),
            other => Err(ApiError::InvalidInput {
                message: format!("invalid actor filter '{other}'; must be 'user' or 'api_key'"),
            }),
        })
        .transpose()?;

    let after = q
        .cursor
        .as_deref()
        .map(|s| {
            SearchCursor::decode(s).ok_or_else(|| ApiError::InvalidInput {
                message: "invalid cursor".to_string(),
            })
        })
        .transpose()?
        .map(|sc| {
            let micros = match sc.key {
                SortKey::Updated(m) => m,
                SortKey::Relevance(_) => {
                    return Err(ApiError::InvalidInput {
                        message: "cursor sort key is not compatible with task listing".to_string(),
                    });
                }
            };
            Ok(atlas_acta::ports::boards_tasks::TaskListCursor {
                sort_value: serde_json::Value::Number(micros.into()),
                id: atlas_acta::ids::TaskId(sc.id),
            })
        })
        .transpose()?;

    let filters = TaskViewFilters {
        sort,
        priorities,
        column_ids,
        labels: q.labels,
        board_id,
        assignee,
        actor_type,
    };

    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgTaskRepo::new((*state.db).clone());

    let mut tasks = repo
        .list_by_workspace_filtered(&ctx, &filters, after, limit + 1)
        .await
        .map_err(ApiError::Domain)?;

    let has_more = tasks.len() > limit as usize;
    if has_more {
        tasks.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        tasks.last().map(|t| SearchCursor {
            key: SortKey::Updated(t.updated_at.timestamp_micros()),
            id: t.id.0,
        })
    } else {
        None
    };

    let mut assignees_by_task = board_assignees_by_task(&state, &ctx, &tasks).await?;
    let board_column_names = board_column_names_by_task(&state, &ctx, &tasks).await?;
    let subtask_counts = subtask_counts_by_task(&state, &ctx, &tasks).await?;

    let dtos = tasks
        .into_iter()
        .map(|t| {
            let (board_id, board_name, column_name) = board_column_names
                .get(&t.column_id.0)
                .cloned()
                .unwrap_or_default();

            TaskSummaryDto {
                id: t.id.0,
                readable_id: t.readable_id,
                board_id,
                column_id: t.column_id.0,
                title: t.title,
                priority: t.priority.map(|p| p.as_str().to_string()),
                estimate: t.estimate,
                labels: t.labels,
                assignees: assignees_by_task.remove(&t.id.0).unwrap_or_default(),
                board_name,
                column_name,
                subtask_count: subtask_counts.get(&t.id.0).copied().unwrap_or(0),
                updated_at: t.updated_at,
            }
        })
        .collect();

    Ok(Json(Page::new_search(dtos, next_cursor, has_more)))
}

/// Parses the raw query string for `GET /api/workspaces/{ws}/tasks`.
///
/// Uses `form_urlencoded` directly to support repeated params (e.g.
/// `?column_id=x&column_id=y`) which `serde_urlencoded` does not handle for
/// `Vec<T>` fields.
fn parse_workspace_task_query(raw: &str) -> Result<WorkspaceTaskQueryParams, ApiError> {
    let mut q = WorkspaceTaskQueryParams::default();
    for pair in raw.split('&').filter(|s| !s.is_empty()) {
        let (key, val) = pair.split_once('=').unwrap_or((pair, ""));
        // Decode `+` as space (form-urlencoded convention). Full percent-decoding
        // is not needed here because all expected values are ASCII-safe (UUIDs,
        // slugs, sort keys, simple labels). A label with non-ASCII chars would
        // arrive as a percent-encoded value; those cases are handled server-side
        // by treating the raw encoded form as the label string.
        let val = val.replace('+', " ");
        match key {
            "assignee" => q.assignee = Some(val),
            "actor" => q.actor = Some(val),
            "column_id" => q.column_ids.push(val),
            "priority" => q.priorities.push(val),
            "label" => q.labels.push(val),
            "board_id" => q.board_id = Some(val),
            "sort" => q.sort = Some(val),
            "cursor" => q.cursor = Some(val),
            "limit" => {
                q.limit = val.parse::<u32>().ok();
            }
            _ => {}
        }
    }
    Ok(q)
}

fn parse_workspace_assignee_filter(s: &str) -> Result<AssigneeFilter, ApiError> {
    if s == "me" {
        return Ok(AssigneeFilter::Me);
    }
    if let Some(rest) = s.strip_prefix("user:") {
        let id = rest
            .parse::<uuid::Uuid>()
            .map_err(|_| ApiError::InvalidInput {
                message: format!("invalid assignee user uuid: {rest}"),
            })?;
        return Ok(AssigneeFilter::User(UserId(id)));
    }
    if let Some(rest) = s.strip_prefix("api_key:") {
        let id = rest
            .parse::<uuid::Uuid>()
            .map_err(|_| ApiError::InvalidInput {
                message: format!("invalid assignee api_key uuid: {rest}"),
            })?;
        return Ok(AssigneeFilter::ApiKey(ApiKeyId(id)));
    }
    Err(ApiError::InvalidInput {
        message: format!(
            "invalid assignee filter '{s}'; must be 'me', 'user:{{uuid}}', or 'api_key:{{uuid}}'"
        ),
    })
}
