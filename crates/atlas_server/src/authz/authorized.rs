use std::collections::HashMap;
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
    entities::identity::{MemberRole, Workspace},
    entities::workspace_core::Project,
    ids::{ApiKeyId, FolderId, UserId},
    permissions::{
        ChainSegment, Principal, ResolutionInput, ResourceChain, ResourceRef, ResourceRole,
        Visibility,
    },
    ports::permission_grant_repo::ResolutionQuery,
};

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
    const ROLE: ResourceRole;
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

/// Builds the `board → project → workspace` permission chain for a board-scoped resource.
///
/// Chain order is most-specific-first: Board → Project → Workspace. The Board segment
/// enables explicit board-level grants (`permission_grants.board_id`) to be resolved by
/// the permission engine and the grant-loader SQL.
pub async fn build_board_chain(
    db: &sea_orm::DatabaseConnection,
    ws: &Workspace,
    board_id: atlas_domain::ids::BoardId,
    project_id: atlas_domain::ids::ProjectId,
) -> Result<ResourceChain, ApiError> {
    let repo = PgProjectRepo { conn: db.clone() };
    let ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(UserId::new()));
    let project = repo
        .find(&ctx, project_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let mut segments = Vec::new();

    segments.push(ChainSegment {
        resource: ResourceRef::Board(board_id),
        visibility: None,
    });

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

/// Proof that the request's principal has at least `M::ROLE` on resource `R`.
pub struct Authorized<R: ResolvedResource, M: MinRole> {
    pub principal: Principal,
    pub workspace: Workspace,
    pub resource: R,
    pub effective: ResourceRole,
    pub membership: Option<atlas_domain::entities::identity::MemberRole>,
    _min: PhantomData<M>,
}

impl<R, M> FromRequestParts<AppState> for Authorized<R, M>
where
    R: ResolvedResource,
    R::PathParams: DeserializeOwned + Send,
    M: MinRole + 'static,
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

        let effective = resolve_effective_role(
            &state.db,
            &domain_principal,
            membership_role.clone(),
            &workspace,
            &chain,
        )
        .await?
        .ok_or(ApiError::NotFound)?;

        if effective < M::ROLE {
            tracing::warn!(
                required = ?M::ROLE,
                effective = ?effective,
                workspace = %ws_slug,
                "authorization denied: insufficient role for resource"
            );
            return Err(ApiError::Forbidden {
                message: "insufficient permissions for this resource".into(),
            });
        }

        Ok(Authorized {
            principal: domain_principal,
            workspace,
            resource,
            effective,
            membership: membership_role,
            _min: PhantomData,
        })
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
    let Some(key) = key_repo.get_by_id(kid).await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?
    else {
        return Ok(None);
    };

    let creator_membership =
        effective_membership_for_user(db, key.created_by_user_id, workspace).await?;
    let creator_principal = Principal::User(key.created_by_user_id);
    let creator_role =
        resolve_grant_role(db, &creator_principal, creator_membership, workspace, chain).await?;

    let base = if key.is_global {
        creator_role
    } else {
        let grant_role =
            resolve_grant_role(db, &Principal::ApiKey(kid), None, workspace, chain).await?;
        min_role(grant_role, creator_role)
    };

    Ok(base.map(|r| r.min(ResourceRole::Editor)))
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
    let Some(user) = user_repo.find_by_id(user_id).await.map_err(|e| ApiError::Internal {
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

    let key_repo = PgApiKeyRepo { conn: db.clone() };
    let Some(key) = key_repo.get_by_id(key_id).await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?
    else {
        return Ok(false);
    };

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
