use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;

use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
};
use serde::de::DeserializeOwned;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::{Board, Task},
    entities::documents::Document,
    entities::identity::{ApiKey, MemberRole, Workspace},
    entities::workspace_core::Project,
    ids::{ApiKeyId, FolderId, UserId},
    permissions::{
        Capability, CapabilityAction, CapabilityFamily, ChainSegment, Principal, ResolutionInput,
        ResourceChain, ResourceRef, ResourceRole, Visibility,
    },
    ports::permission_grant_repo::ResolutionQuery,
};

use super::batch_authorization::ProjectionAuthContext;
use crate::{
    auth::middleware::Principal as MiddlewarePrincipal,
    error::ApiError,
    persistence::{
        entities::{
            boards_tasks::{board, board_from, task, task_from},
            documents::{document, document_from},
            workspace_core::folder,
        },
        repos::{
            ApiKeyRepo, MembershipRepo, PermissionGrantRepo, PgApiKeyRepo, PgMembershipRepo,
            PgPermissionGrantRepo, PgProjectRepo, PgUserRepo, PgWorkspaceRepo, ProjectRepo,
            UserRepo, WorkspaceRepo,
        },
    },
    state::AppState,
};

pub trait MinRole: Send + Sync {
    /// The minimum effective role a human (or group) principal must hold on the
    /// resource. This is the floor applied to every principal that is not an
    /// `ApiKey`.
    const ROLE: ResourceRole;

    /// The minimum effective role an `ApiKey` (agent) principal must hold. It
    /// defaults to `ROLE`, so agents are held to the same floor as humans unless
    /// a marker explicitly lowers it. Overriding this is how a route admits
    /// agents below the human floor while still enforcing a capability scope.
    const AGENT_ROLE: ResourceRole = Self::ROLE;
}

pub struct ViewerMin;
pub struct EditorMin;
pub struct AdminMin;

impl MinRole for ViewerMin {
    const ROLE: ResourceRole = ResourceRole::Viewer;
}
impl MinRole for EditorMin {
    const ROLE: ResourceRole = ResourceRole::Editor;
}
impl MinRole for AdminMin {
    const ROLE: ResourceRole = ResourceRole::Admin;
}

/// Role floor for the webhook routes: a human principal must be `Admin` (the
/// shipped floor, preserved byte-for-byte), while an `ApiKey` principal is
/// admitted at `Editor` and then gated on the matching `webhooks:{action}`
/// capability. This is the only marker that diverges `AGENT_ROLE` from `ROLE`.
pub struct AdminMinAgentEditor;

impl MinRole for AdminMinAgentEditor {
    const ROLE: ResourceRole = ResourceRole::Admin;
    const AGENT_ROLE: ResourceRole = ResourceRole::Editor;
}

/// Declares the API-key capability a route requires, as the third generic
/// marker parameter on `Authorized<R, M, S>`. `NoScope` (the default) means the
/// route has no capability gate; the twenty-four `{Family}{Action}` markers
/// below each pin `CAPABILITY` to one entry of the closed catalog.
///
/// Only `Principal::ApiKey` requests are checked against `CAPABILITY`; `User`
/// and `Group` principals have no scope concept and always bypass the gate.
pub trait RequiredScope: Send + Sync {
    const CAPABILITY: Option<Capability>;
}

/// The default scope marker: no capability required. Every extractor call
/// site outside the five gated families uses this (implicitly, via the
/// default type parameter), so their signature does not change.
pub struct NoScope;

impl RequiredScope for NoScope {
    const CAPABILITY: Option<Capability> = None;
}

macro_rules! capability_marker {
    ($name:ident, $family:ident, $action:ident) => {
        /// Scope marker requiring a single capability from the closed catalog.
        pub struct $name;

        impl RequiredScope for $name {
            const CAPABILITY: Option<Capability> = Some(Capability {
                family: CapabilityFamily::$family,
                action: CapabilityAction::$action,
            });
        }
    };
}

capability_marker!(TasksRead, Tasks, Read);
capability_marker!(TasksCreate, Tasks, Create);
capability_marker!(TasksUpdate, Tasks, Update);
capability_marker!(TasksDelete, Tasks, Delete);
capability_marker!(DocsRead, Docs, Read);
capability_marker!(DocsCreate, Docs, Create);
capability_marker!(DocsUpdate, Docs, Update);
capability_marker!(DocsDelete, Docs, Delete);
capability_marker!(BoardsRead, Boards, Read);
capability_marker!(BoardsCreate, Boards, Create);
capability_marker!(BoardsUpdate, Boards, Update);
capability_marker!(BoardsDelete, Boards, Delete);
capability_marker!(FoldersRead, Folders, Read);
capability_marker!(FoldersCreate, Folders, Create);
capability_marker!(FoldersUpdate, Folders, Update);
capability_marker!(FoldersDelete, Folders, Delete);
capability_marker!(ProjectsRead, Projects, Read);
capability_marker!(ProjectsCreate, Projects, Create);
capability_marker!(ProjectsUpdate, Projects, Update);
capability_marker!(ProjectsDelete, Projects, Delete);
capability_marker!(WebhooksRead, Webhooks, Read);
capability_marker!(WebhooksCreate, Webhooks, Create);
capability_marker!(WebhooksUpdate, Webhooks, Update);
capability_marker!(WebhooksDelete, Webhooks, Delete);
capability_marker!(ConfigRead, Config, Read);
capability_marker!(ConfigCreate, Config, Create);
capability_marker!(ConfigUpdate, Config, Update);
capability_marker!(ConfigDelete, Config, Delete);
capability_marker!(GrantsRead, Grants, Read);
capability_marker!(SavedSearchesRead, SavedSearches, Read);
capability_marker!(SavedSearchesCreate, SavedSearches, Create);
capability_marker!(SavedSearchesUpdate, SavedSearches, Update);
capability_marker!(SavedSearchesDelete, SavedSearches, Delete);
capability_marker!(TaskViewsRead, TaskViews, Read);
capability_marker!(TaskViewsCreate, TaskViews, Create);
capability_marker!(TaskViewsUpdate, TaskViews, Update);
capability_marker!(TaskViewsDelete, TaskViews, Delete);

pub trait ResolvedResource: Sized + Send {
    type PathParams: DeserializeOwned + Send;

    fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        params: Self::PathParams,
    ) -> impl std::future::Future<Output = Result<(Self, ResourceChain), ApiError>> + Send;
}

pub struct ProjectRes(pub Project);
pub struct WorkspaceRes(pub Workspace);

impl ResolvedResource for ProjectRes {
    type PathParams = HashMap<String, String>;

    async fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        params: Self::PathParams,
    ) -> Result<(Self, ResourceChain), ApiError> {
        let slug = params.get("project_slug").ok_or(ApiError::NotFound)?;

        let ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(UserId::new()));
        let repo = PgProjectRepo { conn: db.clone() };
        let project = repo
            .find_by_slug(&ctx, slug)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let vis = project_visibility(&project.visibility);
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
        Ok((ProjectRes(project), chain))
    }
}

impl ResolvedResource for WorkspaceRes {
    type PathParams = ();

    async fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        _params: (),
    ) -> Result<(Self, ResourceChain), ApiError> {
        let _ = db;
        let chain = ResourceChain {
            segments: vec![ChainSegment {
                resource: ResourceRef::Workspace,
                visibility: None,
            }],
        };
        Ok((WorkspaceRes(ws.clone()), chain))
    }
}

/// Proof of identity for a folder resource, resolved via `{folder_id}` path param.
///
/// Builds the `folder → project → workspace` permission chain. Cross-tenant or
/// deleted folders surface as `ApiError::NotFound` (no existence disclosure).
pub struct FolderRes(pub atlas_domain::entities::workspace_core::Folder);

impl ResolvedResource for FolderRes {
    type PathParams = HashMap<String, String>;

    async fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        params: Self::PathParams,
    ) -> Result<(Self, ResourceChain), ApiError> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let folder_id_str = params.get("folder_id").ok_or(ApiError::NotFound)?;
        let folder_uuid =
            folder_id_str
                .parse::<uuid::Uuid>()
                .map_err(|_| ApiError::InvalidInput {
                    message: "folder_id must be a valid UUID".into(),
                })?;

        let row = folder::Entity::find_by_id(folder_uuid)
            .filter(folder::Column::WorkspaceId.eq(ws.id.0))
            .filter(folder::Column::DeletedAt.is_null())
            .one(db)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let f = crate::persistence::entities::workspace_core::folder_from(row);
        let chain = build_folder_chain(db, ws, &f).await?;
        Ok((FolderRes(f), chain))
    }
}

/// Builds the `folder → project → workspace` permission chain for a resolved folder.
pub async fn build_folder_chain(
    db: &sea_orm::DatabaseConnection,
    ws: &Workspace,
    folder: &atlas_domain::entities::workspace_core::Folder,
) -> Result<ResourceChain, ApiError> {
    let ancestry = resolve_folder_ancestry(db, ws.id, folder.id).await?;

    let mut segments = Vec::new();

    for f in &ancestry {
        segments.push(ChainSegment {
            resource: ResourceRef::Folder(f.id),
            visibility: None,
        });
    }

    let effective_project_id = ancestry.last().and_then(|f| f.project_id);
    if let Some(project_id) = effective_project_id {
        let repo = PgProjectRepo { conn: db.clone() };
        let ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(UserId::new()));
        let project = repo
            .find(&ctx, project_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        if let Some(p) = project {
            segments.push(ChainSegment {
                resource: ResourceRef::Project(project_id),
                visibility: Some(project_visibility(&p.visibility)),
            });
        }
    }

    segments.push(ChainSegment {
        resource: ResourceRef::Workspace,
        visibility: None,
    });

    Ok(ResourceChain { segments })
}

/// Proof of identity for a board resource, resolved via `{board_id}` path param.
///
/// Resolves the board → project → workspace permission chain. Cross-tenant or
/// deleted boards surface as `ApiError::NotFound` (no existence disclosure).
pub struct BoardRes(pub Board);

/// Proof of identity for a task resource, resolved via `{readable_id}` path param.
///
/// Resolves the task → board → project → workspace permission chain via the
/// task's owning board. Cross-tenant or deleted tasks surface as `ApiError::NotFound`.
pub struct TaskRes(pub Task);

impl ResolvedResource for BoardRes {
    type PathParams = HashMap<String, String>;

    async fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        params: Self::PathParams,
    ) -> Result<(Self, ResourceChain), ApiError> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let board_id_str = params.get("board_id").ok_or(ApiError::NotFound)?;
        let board_uuid = board_id_str
            .parse::<uuid::Uuid>()
            .map_err(|_| ApiError::NotFound)?;

        let row = board::Entity::find_by_id(board_uuid)
            .filter(board::Column::WorkspaceId.eq(ws.id.0))
            .filter(board::Column::DeletedAt.is_null())
            .one(db)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let board = board_from(row);
        let chain = build_board_chain(db, ws, board.id, board.project_id).await?;
        Ok((BoardRes(board), chain))
    }
}

impl ResolvedResource for TaskRes {
    type PathParams = HashMap<String, String>;

    async fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        params: Self::PathParams,
    ) -> Result<(Self, ResourceChain), ApiError> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let readable_id = params.get("readable_id").ok_or(ApiError::NotFound)?;

        let row = task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(ws.id.0))
            .filter(task::Column::ReadableId.eq(readable_id.as_str()))
            .filter(task::Column::DeletedAt.is_null())
            .one(db)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let task = task_from(row);
        let chain = build_board_chain(db, ws, task.board_id, task.project_id).await?;
        Ok((TaskRes(task), chain))
    }
}

/// Builds the `board → folder ancestry → project → workspace` permission chain for a
/// board-scoped resource.
///
/// Chain order is most-specific-first: Board → Folder(s) → Project → Workspace. The Board
/// segment enables explicit board-level grants (`permission_grants.board_id`) to be resolved
/// by the permission engine and the grant-loader SQL. The Folder segments mirror
/// `build_document_chain`, so a folder-scoped grant reaches boards filed under that folder
/// exactly like it already reaches documents.
pub async fn build_board_chain(
    db: &sea_orm::DatabaseConnection,
    ws: &Workspace,
    board_id: atlas_domain::ids::BoardId,
    project_id: atlas_domain::ids::ProjectId,
) -> Result<ResourceChain, ApiError> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let mut segments = Vec::new();

    segments.push(ChainSegment {
        resource: ResourceRef::Board(board_id),
        visibility: None,
    });

    let board_row = board::Entity::find_by_id(board_id.0)
        .filter(board::Column::WorkspaceId.eq(ws.id.0))
        .one(db)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let board_folder_id = board_row.and_then(|row| row.folder_id).map(FolderId);

    if let Some(leaf_folder_id) = board_folder_id {
        let folder_ancestors = resolve_folder_ancestry(db, ws.id, leaf_folder_id).await?;

        for f in folder_ancestors {
            segments.push(ChainSegment {
                resource: ResourceRef::Folder(f.id),
                visibility: None,
            });
        }
    }

    let repo = PgProjectRepo { conn: db.clone() };
    let ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(UserId::new()));
    let project = repo
        .find(&ctx, project_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if let Some(p) = project {
        segments.push(ChainSegment {
            resource: ResourceRef::Project(project_id),
            visibility: Some(project_visibility(&p.visibility)),
        });
    }

    segments.push(ChainSegment {
        resource: ResourceRef::Workspace,
        visibility: None,
    });

    Ok(ResourceChain { segments })
}

/// Proof of identity for a document resource, resolved via `{doc_id}` path param.
pub struct DocumentRes(pub Document);

/// Proof of identity for a document resource, resolved via `{slug}` path param.
///
/// This is the primary extractor for document routes that use the human-readable slug
/// in the URL. The full folder→project→workspace permission chain is identical to
/// `DocumentRes`; only the path param name differs.
pub struct DocumentSlugRes(pub Document);

impl ResolvedResource for DocumentRes {
    type PathParams = HashMap<String, String>;

    async fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        params: Self::PathParams,
    ) -> Result<(Self, ResourceChain), ApiError> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let doc_id_str = params.get("doc_id").ok_or(ApiError::NotFound)?;
        let doc_uuid = doc_id_str
            .parse::<uuid::Uuid>()
            .map_err(|_| ApiError::NotFound)?;

        let row = document::Entity::find_by_id(doc_uuid)
            .filter(document::Column::WorkspaceId.eq(ws.id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(db)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let doc = document_from(row).map_err(|msg| ApiError::Internal { message: msg })?;

        let chain = build_document_chain(db, ws, &doc).await?;
        Ok((DocumentRes(doc), chain))
    }
}

impl ResolvedResource for DocumentSlugRes {
    type PathParams = HashMap<String, String>;

    async fn resolve(
        db: &sea_orm::DatabaseConnection,
        ws: &Workspace,
        params: Self::PathParams,
    ) -> Result<(Self, ResourceChain), ApiError> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let ident = params.get("slug").ok_or(ApiError::NotFound)?;

        // The `{slug}` segment accepts either the document's stable UUID (used by
        // search, wikilinks and any programmatic link — rename-proof) or its
        // human-readable slug. UUID is the canonical identity; slug is sugar.
        let base = document::Entity::find()
            .filter(document::Column::WorkspaceId.eq(ws.id.0))
            .filter(document::Column::DeletedAt.is_null());

        let query = match ident.parse::<uuid::Uuid>() {
            Ok(uuid) => base.filter(document::Column::Id.eq(uuid)),
            Err(_) => base.filter(document::Column::Slug.eq(ident.as_str())),
        };

        let row = query
            .one(db)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        let doc = document_from(row).map_err(|msg| ApiError::Internal { message: msg })?;

        let chain = build_document_chain(db, ws, &doc).await?;
        Ok((DocumentSlugRes(doc), chain))
    }
}

/// Builds the full `document → folder ancestry → project → workspace` permission
/// chain for a resolved document. Shared by `DocumentRes`, `DocumentSlugRes`, and
/// the attachment routes, which authorize against the attachment's owning document.
pub async fn build_document_chain(
    db: &sea_orm::DatabaseConnection,
    ws: &Workspace,
    doc: &Document,
) -> Result<ResourceChain, ApiError> {
    let mut segments = Vec::new();

    segments.push(ChainSegment {
        resource: ResourceRef::Document(doc.id),
        visibility: None,
    });

    // Walk folder ancestry most-specific-first, bounded to guard against cycles.
    let mut inherited_project_id = None;
    if let Some(leaf_folder_id) = doc.folder_id {
        let folder_ancestors = resolve_folder_ancestry(db, ws.id, leaf_folder_id).await?;

        // The last folder in the ancestry list carries the project_id that should
        // act as the document's project when the document itself has no project_id.
        if let Some(root) = folder_ancestors.last() {
            inherited_project_id = root.project_id;
        }

        for f in folder_ancestors {
            segments.push(ChainSegment {
                resource: ResourceRef::Folder(f.id),
                visibility: None,
            });
        }
    }

    let effective_project_id = doc.project_id.or(inherited_project_id);
    if let Some(project_id) = effective_project_id {
        let repo = PgProjectRepo { conn: db.clone() };
        let ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(UserId::new()));
        let project = repo
            .find(&ctx, project_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        if let Some(p) = project {
            segments.push(ChainSegment {
                resource: ResourceRef::Project(project_id),
                visibility: Some(project_visibility(&p.visibility)),
            });
        }
    }

    segments.push(ChainSegment {
        resource: ResourceRef::Workspace,
        visibility: None,
    });

    Ok(ResourceChain { segments })
}

/// Walks the folder hierarchy from `leaf_id` upward, returning folders in
/// most-specific-first order (leaf → root). Bounded at 32 levels to guard
/// against cycles or pathologically deep trees.
pub async fn resolve_folder_ancestry(
    db: &sea_orm::DatabaseConnection,
    workspace_id: atlas_domain::ids::WorkspaceId,
    leaf_id: FolderId,
) -> Result<Vec<atlas_domain::entities::workspace_core::Folder>, ApiError> {
    use crate::persistence::entities::workspace_core::folder_from;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    const MAX_DEPTH: usize = 32;

    let mut ancestry: Vec<atlas_domain::entities::workspace_core::Folder> = Vec::new();
    let mut current_id = Some(leaf_id);

    while let Some(fid) = current_id {
        if ancestry.len() >= MAX_DEPTH {
            break;
        }

        let row = folder::Entity::find_by_id(fid.0)
            .filter(folder::Column::WorkspaceId.eq(workspace_id.0))
            .filter(folder::Column::DeletedAt.is_null())
            .one(db)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        match row {
            None => break,
            Some(m) => {
                let f = folder_from(m);
                current_id = f.parent_folder_id;
                ancestry.push(f);
            }
        }
    }

    Ok(ancestry)
}

/// Authorizes `principal` for at least `min` on the destination `folder`'s
/// permission chain (folder ancestry → project → workspace). Used by document
/// move to prevent relocating a document into a folder the caller cannot edit.
/// Rejects with NotFound when the folder is absent from the workspace or the
/// principal lacks the role, to avoid disclosing the folder's existence.
pub async fn authorize_folder_destination(
    db: &sea_orm::DatabaseConnection,
    principal: &Principal,
    membership: Option<atlas_domain::entities::identity::MemberRole>,
    workspace: &Workspace,
    folder_id: FolderId,
    min: ResourceRole,
) -> Result<(), ApiError> {
    let ancestry = resolve_folder_ancestry(db, workspace.id, folder_id).await?;

    if ancestry.is_empty() {
        return Err(ApiError::NotFound);
    }

    let mut segments = Vec::new();

    for f in &ancestry {
        segments.push(ChainSegment {
            resource: ResourceRef::Folder(f.id),
            visibility: None,
        });
    }

    let effective_project_id = ancestry.last().and_then(|f| f.project_id);
    if let Some(project_id) = effective_project_id {
        let repo = PgProjectRepo { conn: db.clone() };
        let ctx =
            atlas_domain::WorkspaceCtx::new(workspace.id, atlas_domain::Actor::User(UserId::new()));
        let project = repo
            .find(&ctx, project_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        if let Some(p) = project {
            segments.push(ChainSegment {
                resource: ResourceRef::Project(project_id),
                visibility: Some(project_visibility(&p.visibility)),
            });
        }
    }

    segments.push(ChainSegment {
        resource: ResourceRef::Workspace,
        visibility: None,
    });

    let chain = ResourceChain { segments };

    let effective = resolve_effective_role(db, principal, membership, workspace, &chain)
        .await?
        .ok_or(ApiError::NotFound)?;

    if effective < min {
        return Err(ApiError::NotFound);
    }

    Ok(())
}

/// Authorizes `principal` for at least `min` on the board that owns the
/// destination `column`'s permission chain (board → project → workspace). Used
/// by task move to prevent relocating a task into a board the caller cannot
/// edit, even when they can edit the task's current board.
///
/// Rejects with NotFound when the column or its board is absent from the
/// workspace, or the principal lacks the role, to avoid disclosing existence.
pub async fn authorize_board_destination(
    db: &sea_orm::DatabaseConnection,
    principal: &Principal,
    membership: Option<atlas_domain::entities::identity::MemberRole>,
    workspace: &Workspace,
    column_id: atlas_domain::ids::ColumnId,
    min: ResourceRole,
) -> Result<(), ApiError> {
    use crate::persistence::entities::boards_tasks::board_column;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let column = board_column::Entity::find_by_id(column_id.0)
        .filter(board_column::Column::WorkspaceId.eq(workspace.id.0))
        .filter(board_column::Column::DeletedAt.is_null())
        .one(db)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let board = board::Entity::find_by_id(column.board_id)
        .filter(board::Column::WorkspaceId.eq(workspace.id.0))
        .filter(board::Column::DeletedAt.is_null())
        .one(db)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let board = board_from(board);
    let chain = build_board_chain(db, workspace, board.id, board.project_id).await?;

    let effective = resolve_effective_role(db, principal, membership, workspace, &chain)
        .await?
        .ok_or(ApiError::NotFound)?;

    if effective < min {
        return Err(ApiError::NotFound);
    }

    Ok(())
}

fn project_visibility(vis: &atlas_domain::permissions::Visibility) -> Visibility {
    vis.clone()
}

/// Proof that the request's principal has at least `M::ROLE` on resource `R`,
/// and — when the principal is an `ApiKey` and `S::CAPABILITY` is `Some` — that
/// the key's scope set holds that capability.
pub struct Authorized<R: ResolvedResource, M: MinRole, S: RequiredScope = NoScope> {
    pub principal: Principal,
    pub workspace: Workspace,
    pub resource: R,
    pub effective: ResourceRole,
    pub membership: Option<atlas_domain::entities::identity::MemberRole>,
    #[allow(
        dead_code,
        reason = "WU2-C1a retains request authorization state before route wiring"
    )]
    projection_context: ProjectionAuthContext,
    _min: PhantomData<(M, S)>,
}

impl<R: ResolvedResource, M: MinRole, S: RequiredScope> Authorized<R, M, S> {
    #[allow(
        dead_code,
        reason = "WU2-C1a exposes the context for the next projection slice"
    )]
    pub(crate) fn projection_context(&self) -> &ProjectionAuthContext {
        &self.projection_context
    }
}

impl<R, M, S> FromRequestParts<AppState> for Authorized<R, M, S>
where
    R: ResolvedResource,
    R::PathParams: DeserializeOwned + Send,
    M: MinRole + 'static,
    S: RequiredScope + 'static,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let middleware_principal = parts
            .extensions
            .get::<MiddlewarePrincipal>()
            .cloned()
            .ok_or(ApiError::Unauthorized)?;

        let path_params: Path<HashMap<String, String>> = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::NotFound)?;

        let ws_slug = path_params.get("ws").ok_or(ApiError::NotFound)?.clone();

        let ws_repo = PgWorkspaceRepo {
            conn: (*state.db).clone(),
        };
        let workspace = ws_repo
            .find_by_slug(&ws_slug)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::NotFound)?;

        // For an api-key principal the same row is needed twice below: once to
        // resolve the agent's effective role and once to gate the required
        // capability. Load it a single time here and thread it into both steps,
        // instead of issuing two independent `get_by_id` lookups.
        let mut api_key: Option<ApiKey> = None;

        let (domain_principal, membership_role) =
            match &middleware_principal {
                MiddlewarePrincipal::User(uid) => {
                    let user_repo = PgUserRepo {
                        conn: (*state.db).clone(),
                    };
                    let user = user_repo
                        .find_by_id(*uid)
                        .await
                        .map_err(|e| ApiError::Internal {
                            message: e.to_string(),
                        })?
                        .ok_or(ApiError::Unauthorized)?;

                    if user.disabled_at.is_some() {
                        return Err(ApiError::Unauthorized);
                    }

                    // is_root and is_system_admin synthesize Admin membership to gain
                    // global admin access to every workspace's content without being a member.
                    // This is a security-load-bearing short-circuit: weakening this check
                    // would silently remove global-admin content visibility.
                    if user.is_root || user.is_system_admin {
                        let role = Some(atlas_domain::entities::identity::MemberRole::Admin);
                        (Principal::User(*uid), role)
                    } else {
                        let membership_repo = PgMembershipRepo {
                            conn: (*state.db).clone(),
                        };
                        let ctx = atlas_domain::WorkspaceCtx::new(
                            workspace.id,
                            atlas_domain::Actor::User(*uid),
                        );
                        let membership = membership_repo.find(&ctx, *uid).await.map_err(|e| {
                            ApiError::Internal {
                                message: e.to_string(),
                            }
                        })?;

                        if membership.is_none() {
                            return Err(ApiError::NotFound);
                        }

                        let role = membership.map(|m| m.role);
                        (Principal::User(*uid), role)
                    }
                }
                MiddlewarePrincipal::ApiKey(kid) => {
                    // Workspace access for an api key: it must hold a grant in this workspace, or
                    // be global with a creator that can reach it. resolve_effective_role is the
                    // downstream authority and returns None (→ 404) when the creator has no reach.
                    if !api_key_can_access_workspace(&state.db, *kid, &workspace).await? {
                        return Err(ApiError::NotFound);
                    }

                    let key_repo = PgApiKeyRepo {
                        conn: (*state.db).clone(),
                    };
                    api_key = key_repo
                        .get_by_id(*kid)
                        .await
                        .map_err(|e| ApiError::Internal {
                            message: e.to_string(),
                        })?;

                    (Principal::ApiKey(*kid), None)
                }
            };

        let path_map_value =
            serde_json::to_value(&path_params.0).map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        let params: R::PathParams = serde_json::from_value(path_map_value.clone())
            .or_else(|_| serde_json::from_value(serde_json::Value::Null))
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        let (resource, chain) = R::resolve(&state.db, &workspace, params).await?;

        let effective_role = if let Principal::ApiKey(_) = &domain_principal {
            // Reuse the key loaded above rather than letting
            // `resolve_agent_effective_role` fetch it again. A missing key here
            // means no access, mirroring that function's `Ok(None)` on a miss.
            match api_key.as_ref() {
                Some(key) => {
                    resolve_agent_effective_role_with_key(&state.db, key, &workspace, &chain)
                        .await?
                }
                None => None,
            }
        } else {
            resolve_effective_role(
                &state.db,
                &domain_principal,
                membership_role.clone(),
                &workspace,
                &chain,
            )
            .await?
        };

        let effective = effective_role.ok_or(ApiError::NotFound)?;

        // Principal-aware role floor: an ApiKey principal is held to
        // `M::AGENT_ROLE`, every other principal to `M::ROLE`. For all markers
        // except `AdminMinAgentEditor`, `AGENT_ROLE == ROLE`, so both branches
        // collapse to `M::ROLE` and this is identical to the previous check.
        let floor = if matches!(domain_principal, Principal::ApiKey(_)) {
            M::AGENT_ROLE
        } else {
            M::ROLE
        };

        if effective < floor {
            tracing::warn!(
                target: "authz.role_denied",
                event = "role_denied",
                required = ?floor,
                effective = ?effective,
                workspace = %ws_slug,
                principal = ?domain_principal,
                "authorization denied: insufficient role for resource"
            );
            return Err(ApiError::Forbidden {
                message: "insufficient permissions for this resource".into(),
            });
        }

        if let Principal::ApiKey(_) = &domain_principal
            && let Some(required) = S::CAPABILITY
        {
            // Gate against the key loaded above; `enforce_api_key_scope` would
            // otherwise re-read the identical row. A missing key denies as
            // NotFound, exactly as that helper does on a miss.
            let key = api_key.as_ref().ok_or(ApiError::NotFound)?;
            enforce_scope_on_key(key, required)?;
        }

        Ok(Authorized {
            principal: domain_principal.clone(),
            projection_context: ProjectionAuthContext::from_validated(
                workspace.id,
                domain_principal.clone(),
            ),
            workspace,
            resource,
            effective,
            membership: membership_role,
            _min: PhantomData,
        })
    }
}

/// Denies (403) an API-key request whose scope set lacks `required`. Runs
/// immediately after the role-threshold check in
/// `Authorized::from_request_parts`, and is also called manually by the small
/// set of in-scope routes that bypass that extractor (see the design's
/// "bypass call sites" for the exhaustive list).
///
/// Only meaningful for `Principal::ApiKey`; `User` and `Group` principals never
/// call this — they have no scope concept and always bypass the gate.
pub async fn enforce_api_key_scope(
    db: &sea_orm::DatabaseConnection,
    key_id: ApiKeyId,
    required: Capability,
) -> Result<(), ApiError> {
    let key_repo = PgApiKeyRepo { conn: db.clone() };
    let key = key_repo
        .get_by_id(key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    enforce_scope_on_key(&key, required)
}

/// Pure capability check against an already-loaded key. Shared by
/// `enforce_api_key_scope` (which fetches the key) and the `Authorized`
/// extractor (which reuses the key it loaded for role resolution), so both
/// paths deny with an identical 403 and log identically.
fn enforce_scope_on_key(key: &ApiKey, required: Capability) -> Result<(), ApiError> {
    if !key.scopes.contains(&required) {
        tracing::warn!(
            target: "authz.scope_denied",
            event = "scope_denied",
            family = ?required.family,
            action = ?required.action,
            capability = required.as_str(),
            key_id = %key.id.0,
            "authorization denied: api key lacks required scope"
        );
        return Err(ApiError::Forbidden {
            message: format!("api key lacks required scope: {}", required.as_str()),
        });
    }

    Ok(())
}

/// The set of families an API key holds `{family}:read` on, precomputed from the
/// key's full scope list so cross-family read feeds can be gated per family.
///
/// This is the single source of the "may this principal read family X" predicate
/// for the scope-gated read feeds (search, workspace activity). Callers MUST go
/// through [`ReadScopeSet::allows`] rather than re-deriving the check from a raw
/// scope slice, so the read-gate semantics stay defined in exactly one place.
#[derive(Debug, Clone)]
pub struct ReadScopeSet {
    readable: HashSet<CapabilityFamily>,
}

impl ReadScopeSet {
    /// Builds the set from a key's full scope list, retaining only the families
    /// whose `{family}:read` capability is present. Non-read capabilities are
    /// irrelevant to read gating and are ignored.
    pub fn from_scopes(scopes: &[Capability]) -> Self {
        let readable = scopes
            .iter()
            .filter(|cap| cap.action == CapabilityAction::Read)
            .map(|cap| cap.family)
            .collect();

        Self { readable }
    }

    /// True iff the key holds `{family}:read`, i.e. the principal may read the
    /// given family through a scope-gated read feed.
    pub fn allows(&self, family: CapabilityFamily) -> bool {
        self.readable.contains(&family)
    }
}

/// Loads the principal's grants for the resource chain and resolves the effective
/// role. Returns `None` when the principal has no access at all (default deny).
///
/// Shared between the `Authorized` extractor and the attachment routes so both
/// honor the identical grant-resolution semantics.
pub async fn resolve_effective_role(
    db: &sea_orm::DatabaseConnection,
    principal: &Principal,
    membership: Option<MemberRole>,
    workspace: &Workspace,
    chain: &ResourceChain,
) -> Result<Option<ResourceRole>, ApiError> {
    // Agents resolve through their creator: an api key never exceeds the role its
    // creator holds on the same resource, and a global key inherits that reach
    // everywhere instead of needing per-workspace grants.
    if let Principal::ApiKey(kid) = principal {
        return resolve_agent_effective_role(db, *kid, workspace, chain).await;
    }

    resolve_grant_role(db, principal, membership, workspace, chain).await
}

/// Resolves a principal's role purely from its own grants, group grants,
/// visibility, and (for users) workspace membership. This is the raw grant
/// resolution, before any agent-specific ceiling is applied.
async fn resolve_grant_role(
    db: &sea_orm::DatabaseConnection,
    principal: &Principal,
    membership: Option<MemberRole>,
    workspace: &Workspace,
    chain: &ResourceChain,
) -> Result<Option<ResourceRole>, ApiError> {
    let grant_query = build_resolution_query(db, principal, workspace, chain).await?;
    let grant_repo = PgPermissionGrantRepo { conn: db.clone() };
    let grants = grant_repo
        .load_grants_for_resolution(grant_query)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let input = ResolutionInput {
        principal,
        membership,
        chain,
        grants: &grants,
    };

    Ok(atlas_domain::permissions::resolve(&input))
}

/// Resolves an api key's effective role under two invariants:
///
/// 1. An agent never exceeds its creator's effective role on the same resource.
/// 2. A `is_global` key inherits the creator's reach on every resource (no grant
///    needed); a non-global key is the intersection of its own grants and the
///    creator's reach.
///
/// The agent editor cap then applies on top, so an agent is always
/// `min(Editor, creator_role, [grant_role])`. A revoked-down or removed creator
/// drops the agent's access in lockstep, since the creator role is recomputed per
/// request rather than snapshotted.
async fn resolve_agent_effective_role(
    db: &sea_orm::DatabaseConnection,
    kid: ApiKeyId,
    workspace: &Workspace,
    chain: &ResourceChain,
) -> Result<Option<ResourceRole>, ApiError> {
    let key_repo = PgApiKeyRepo { conn: db.clone() };
    let Some(key) = key_repo
        .get_by_id(kid)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
    else {
        return Ok(None);
    };

    resolve_agent_effective_role_with_key(db, &key, workspace, chain).await
}

/// Resolves an api key's effective role from an already-loaded key row, applying
/// the creator-ceiling, global-inheritance, and editor-cap invariants documented
/// on `resolve_agent_effective_role`. Factored out so the `Authorized` extractor
/// can reuse the key it loaded for the scope gate instead of fetching it twice.
async fn resolve_agent_effective_role_with_key(
    db: &sea_orm::DatabaseConnection,
    key: &ApiKey,
    workspace: &Workspace,
    chain: &ResourceChain,
) -> Result<Option<ResourceRole>, ApiError> {
    if !user_is_active(db, key.created_by_user_id).await? {
        return Ok(None);
    }

    let creator_membership =
        effective_membership_for_user(db, key.created_by_user_id, workspace).await?;
    let creator_principal = Principal::User(key.created_by_user_id);
    let creator_role =
        resolve_grant_role(db, &creator_principal, creator_membership, workspace, chain).await?;

    let base = if key.is_global {
        creator_role
    } else {
        let grant_role =
            resolve_grant_role(db, &Principal::ApiKey(key.id), None, workspace, chain).await?;
        min_role(grant_role, creator_role)
    };

    Ok(base.map(|r| r.min(ResourceRole::Editor)))
}

async fn user_is_active(
    db: &sea_orm::DatabaseConnection,
    user_id: UserId,
) -> Result<bool, ApiError> {
    let user_repo = PgUserRepo { conn: db.clone() };
    let user = user_repo
        .find_by_id(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(user.is_some_and(|user| user.disabled_at.is_none()))
}

/// The effective workspace membership a user would resolve with: `Admin` for a
/// global superadmin (root/system-admin), `None` for a disabled or missing user,
/// otherwise their stored membership role (or `None` when they are not a member).
async fn effective_membership_for_user(
    db: &sea_orm::DatabaseConnection,
    user_id: UserId,
    workspace: &Workspace,
) -> Result<Option<MemberRole>, ApiError> {
    let user_repo = PgUserRepo { conn: db.clone() };
    let Some(user) = user_repo
        .find_by_id(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
    else {
        return Ok(None);
    };

    if user.disabled_at.is_some() {
        return Ok(None);
    }

    if user.is_root || user.is_system_admin {
        return Ok(Some(MemberRole::Admin));
    }

    let membership_repo = PgMembershipRepo { conn: db.clone() };
    let ctx = WorkspaceCtx::new(workspace.id, Actor::User(user_id));
    let membership = membership_repo
        .find(&ctx, user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(membership.map(|m| m.role))
}

/// Intersection of two optional roles: both must grant access, and the lower role
/// wins. `None` on either side means no access.
fn min_role(a: Option<ResourceRole>, b: Option<ResourceRole>) -> Option<ResourceRole> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        _ => None,
    }
}

/// Workspace-entry gate for an api key, shared by every workspace extractor: the
/// key is admitted when it holds a grant anywhere in the workspace, or when it is
/// global and its creator can reach the workspace. Per-resource role (and the
/// editor cap) is still enforced downstream by `resolve_effective_role`.
pub async fn api_key_can_access_workspace(
    db: &sea_orm::DatabaseConnection,
    key_id: ApiKeyId,
    workspace: &Workspace,
) -> Result<bool, ApiError> {
    let key_repo = PgApiKeyRepo { conn: db.clone() };
    let Some(key) = key_repo
        .get_by_id(key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
    else {
        return Ok(false);
    };

    if !user_is_active(db, key.created_by_user_id).await? {
        return Ok(false);
    }

    let grant_repo = PgPermissionGrantRepo { conn: db.clone() };
    let has_grant = grant_repo
        .principal_has_any_grant_in_workspace(workspace.id, None, Some(key_id))
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if has_grant {
        return Ok(true);
    }

    if !key.is_global {
        return Ok(false);
    }

    creator_can_reach_workspace(db, key.created_by_user_id, workspace).await
}

/// Whether a user can reach a workspace at all: a global superadmin or a member
/// (both surface via `effective_membership_for_user`), or a non-member who holds
/// at least one grant in the workspace.
async fn creator_can_reach_workspace(
    db: &sea_orm::DatabaseConnection,
    user_id: UserId,
    workspace: &Workspace,
) -> Result<bool, ApiError> {
    if effective_membership_for_user(db, user_id, workspace)
        .await?
        .is_some()
    {
        return Ok(true);
    }

    let grant_repo = PgPermissionGrantRepo { conn: db.clone() };
    grant_repo
        .principal_has_any_grant_in_workspace(workspace.id, Some(user_id), None)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })
}

async fn build_resolution_query(
    db: &sea_orm::DatabaseConnection,
    principal: &Principal,
    workspace: &Workspace,
    chain: &ResourceChain,
) -> Result<ResolutionQuery, ApiError> {
    let user_id = match principal {
        Principal::User(uid) => Some(uid.0),
        Principal::ApiKey(_) | Principal::Group(_) => None,
    };
    let api_key_id = match principal {
        Principal::ApiKey(kid) => Some(kid.0),
        Principal::User(_) | Principal::Group(_) => None,
    };

    // For user principals, gather the live group ids this user belongs to in the
    // workspace. These are merged into the grant lookup so group grants contribute
    // to the same max-role resolution as direct grants.
    // Api keys are never in groups; their resolution path gathers no group grants.
    let group_ids = if let Some(uid) = user_id {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct GroupRow {
            group_id: uuid::Uuid,
        }

        let rows = GroupRow::find_by_statement(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            r#"
            SELECT gm.group_id
            FROM group_members gm
            JOIN groups g ON g.id = gm.group_id
            WHERE gm.user_id = $1
              AND g.workspace_id = $2
              AND g.deleted_at IS NULL
            "#,
            [uid.into(), workspace.id.0.into()],
        ))
        .all(db)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

        rows.into_iter().map(|r| r.group_id).collect()
    } else {
        vec![]
    };

    let mut chain_projects = Vec::new();
    let mut chain_folders = Vec::new();
    let mut doc_id = None;
    let mut board_id = None;

    for seg in &chain.segments {
        match &seg.resource {
            ResourceRef::Project(pid) => chain_projects.push(pid.0),
            ResourceRef::Folder(fid) => chain_folders.push(fid.0),
            ResourceRef::Document(did) => doc_id = Some(did.0),
            ResourceRef::Board(bid) => board_id = Some(bid.0),
            ResourceRef::Workspace => {}
        }
    }

    Ok(ResolutionQuery {
        workspace_id: workspace.id,
        user_id,
        api_key_id,
        group_ids,
        chain_projects,
        chain_folders,
        doc_id,
        board_id,
    })
}
