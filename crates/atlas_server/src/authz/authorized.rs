use std::collections::HashMap;
use std::marker::PhantomData;

use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
};
use serde::de::DeserializeOwned;

use atlas_domain::{
    entities::identity::Workspace,
    entities::workspace_core::Project,
    ids::UserId,
    permissions::{
        ChainSegment, Principal, ResolutionInput, ResourceChain, ResourceRef, ResourceRole,
        Visibility,
    },
    ports::permission_grant_repo::ResolutionQuery,
};

use crate::{
    auth::middleware::Principal as MiddlewarePrincipal,
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, MembershipRepo, PermissionGrantRepo, PgApiKeyRepo, PgMembershipRepo,
        PgPermissionGrantRepo, PgProjectRepo, PgUserRepo, PgWorkspaceRepo, ProjectRepo, UserRepo,
        WorkspaceRepo,
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

fn project_visibility(vis: &atlas_domain::permissions::Visibility) -> Visibility {
    vis.clone()
}

/// Proof that the request's principal has at least `M::ROLE` on resource `R`.
pub struct Authorized<R: ResolvedResource, M: MinRole> {
    pub principal: Principal,
    pub workspace: Workspace,
    pub resource: R,
    pub effective: ResourceRole,
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

        let (domain_principal, membership_role) = match &middleware_principal {
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

                let membership_repo = PgMembershipRepo {
                    conn: (*state.db).clone(),
                };
                let ctx =
                    atlas_domain::WorkspaceCtx::new(workspace.id, atlas_domain::Actor::User(*uid));
                let membership =
                    membership_repo
                        .find(&ctx, *uid)
                        .await
                        .map_err(|e| ApiError::Internal {
                            message: e.to_string(),
                        })?;

                if membership.is_none() {
                    return Err(ApiError::NotFound);
                }

                let role = membership.map(|m| m.role);
                (Principal::User(*uid), role)
            }
            MiddlewarePrincipal::ApiKey(kid) => {
                let api_key_repo = PgApiKeyRepo {
                    conn: (*state.db).clone(),
                };
                let ctx = atlas_domain::WorkspaceCtx::new(
                    workspace.id,
                    atlas_domain::Actor::ApiKey(*kid),
                );
                let keys = api_key_repo
                    .list(&ctx)
                    .await
                    .map_err(|e| ApiError::Internal {
                        message: e.to_string(),
                    })?;

                if !keys.iter().any(|k| k.id == *kid) {
                    return Err(ApiError::NotFound);
                }

                (Principal::ApiKey(*kid), None)
            }
        };

        let params: R::PathParams =
            serde_json::from_value(serde_json::to_value(&path_params.0).map_err(|e| {
                ApiError::Internal {
                    message: e.to_string(),
                }
            })?)
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        let (resource, chain) = R::resolve(&state.db, &workspace, params).await?;

        let grant_query = build_resolution_query(&domain_principal, &workspace, &chain);
        let grant_repo = PgPermissionGrantRepo {
            conn: (*state.db).clone(),
        };
        let grants = grant_repo
            .load_grants_for_resolution(grant_query)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        let input = ResolutionInput {
            principal: &domain_principal,
            membership: membership_role,
            chain: &chain,
            grants: &grants,
        };

        let effective = atlas_domain::permissions::resolve(&input).ok_or(ApiError::NotFound)?;

        if effective < M::ROLE {
            return Err(ApiError::Forbidden {
                message: format!(
                    "your effective role is {:?}; minimum required is {:?}",
                    effective,
                    M::ROLE
                ),
            });
        }

        Ok(Authorized {
            principal: domain_principal,
            workspace,
            resource,
            effective,
            _min: PhantomData,
        })
    }
}

fn build_resolution_query(
    principal: &Principal,
    workspace: &Workspace,
    chain: &ResourceChain,
) -> ResolutionQuery {
    let user_id = match principal {
        Principal::User(uid) => Some(uid.0),
        Principal::ApiKey(_) => None,
    };
    let api_key_id = match principal {
        Principal::ApiKey(kid) => Some(kid.0),
        Principal::User(_) => None,
    };

    let mut chain_projects = Vec::new();
    let mut doc_id = None;
    let mut board_id = None;

    for seg in &chain.segments {
        match &seg.resource {
            ResourceRef::Project(pid) => chain_projects.push(pid.0),
            ResourceRef::Document(did) => doc_id = Some(did.0),
            ResourceRef::Board(bid) => board_id = Some(bid.0),
            _ => {}
        }
    }

    ResolutionQuery {
        workspace_id: workspace.id,
        user_id,
        api_key_id,
        chain_projects,
        chain_folders: Vec::new(),
        doc_id,
        board_id,
    }
}
