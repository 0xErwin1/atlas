use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
};
use std::collections::HashMap;

use atlas_domain::{
    entities::identity::{User, WorkspaceMembership},
    ids::{ApiKeyId, UserId},
};

use crate::{
    auth::middleware::Principal,
    error::ApiError,
    persistence::repos::{
        MembershipRepo, PgMembershipRepo, PgUserRepo, PgWorkspaceRepo, UserRepo, Workspace,
        WorkspaceRepo,
    },
    state::AppState,
};

/// Proof that the request's principal is an authenticated, non-disabled workspace member.
///
/// Extracts the workspace from the `{ws}` path segment and verifies the principal
/// belongs to it. Resolves to the workspace row, the associated user (if a user
/// principal), and the api key id (if an api key principal).
pub struct WorkspaceMember {
    pub workspace: Workspace,
    pub user: Option<User>,
    pub api_key_id: Option<ApiKeyId>,
    pub membership: Option<WorkspaceMembership>,
}

impl FromRequestParts<AppState> for WorkspaceMember {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let principal = parts
            .extensions
            .get::<Principal>()
            .cloned()
            .ok_or(ApiError::Unauthorized)?;

        let Path(params): Path<HashMap<String, String>> = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::NotFound)?;

        let ws_slug = params.get("ws").ok_or(ApiError::NotFound)?.clone();

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

        match principal {
            Principal::User(user_id) => {
                let user_repo = PgUserRepo {
                    conn: (*state.db).clone(),
                };
                let user = user_repo
                    .find_by_id(user_id)
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
                let ctx = atlas_domain::WorkspaceCtx::new(
                    workspace.id,
                    atlas_domain::Actor::User(user_id),
                );
                let membership =
                    membership_repo
                        .find(&ctx, user_id)
                        .await
                        .map_err(|e| ApiError::Internal {
                            message: e.to_string(),
                        })?;

                if membership.is_none() {
                    return Err(ApiError::NotFound);
                }

                Ok(WorkspaceMember {
                    workspace,
                    user: Some(user),
                    api_key_id: None,
                    membership,
                })
            }
            Principal::ApiKey(key_id) => {
                let api_key_repo = crate::persistence::repos::PgApiKeyRepo {
                    conn: (*state.db).clone(),
                };
                let ctx = atlas_domain::WorkspaceCtx::new(
                    workspace.id,
                    atlas_domain::Actor::ApiKey(key_id),
                );
                let keys = crate::persistence::repos::ApiKeyRepo::list(&api_key_repo, &ctx)
                    .await
                    .map_err(|e| ApiError::Internal {
                        message: e.to_string(),
                    })?;

                let belongs = keys.iter().any(|k| k.id == key_id);
                if !belongs {
                    return Err(ApiError::NotFound);
                }

                Ok(WorkspaceMember {
                    workspace,
                    user: None,
                    api_key_id: Some(key_id),
                    membership: None,
                })
            }
        }
    }
}

/// Proof that the request's principal has access to the workspace named in the
/// `{ws}` path segment, without imposing a resource role gate.
///
/// Access is granted to any principal that is either a workspace member (any
/// `MemberRole`) or holds at least one grant anywhere in the workspace
/// (workspace-scope, project, folder, document, or board). A principal with no
/// membership and no grant — or a cross-tenant principal — surfaces as
/// `ApiError::NotFound` (concealment; never 403).
///
/// This is the entry gate for the unified search route: per-row visibility is
/// enforced by the SQL permission filter (mirroring `resolve()`), so the gate
/// only needs to confirm the principal belongs to the workspace at all.
pub struct WorkspaceAccess {
    pub principal: atlas_domain::permissions::Principal,
    pub workspace: Workspace,
    pub membership: Option<atlas_domain::entities::identity::MemberRole>,
}

impl FromRequestParts<AppState> for WorkspaceAccess {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let principal = parts
            .extensions
            .get::<Principal>()
            .cloned()
            .ok_or(ApiError::Unauthorized)?;

        let Path(params): Path<HashMap<String, String>> = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::NotFound)?;

        let ws_slug = params.get("ws").ok_or(ApiError::NotFound)?.clone();

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

        let grant_repo = crate::persistence::repos::PgPermissionGrantRepo {
            conn: (*state.db).clone(),
        };

        match principal {
            Principal::User(user_id) => {
                let user_repo = PgUserRepo {
                    conn: (*state.db).clone(),
                };
                let user = user_repo
                    .find_by_id(user_id)
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
                let ctx = atlas_domain::WorkspaceCtx::new(
                    workspace.id,
                    atlas_domain::Actor::User(user_id),
                );
                let membership =
                    membership_repo
                        .find(&ctx, user_id)
                        .await
                        .map_err(|e| ApiError::Internal {
                            message: e.to_string(),
                        })?;

                let role = membership.map(|m| m.role);

                if role.is_none() {
                    let has_grant = grant_repo
                        .principal_has_any_grant_in_workspace(
                            workspace.id,
                            Some(user_id),
                            None,
                        )
                        .await
                        .map_err(|e| ApiError::Internal {
                            message: e.to_string(),
                        })?;

                    if !has_grant {
                        return Err(ApiError::NotFound);
                    }
                }

                Ok(WorkspaceAccess {
                    principal: atlas_domain::permissions::Principal::User(user_id),
                    workspace,
                    membership: role,
                })
            }
            Principal::ApiKey(key_id) => {
                let api_key_repo = crate::persistence::repos::PgApiKeyRepo {
                    conn: (*state.db).clone(),
                };
                let ctx = atlas_domain::WorkspaceCtx::new(
                    workspace.id,
                    atlas_domain::Actor::ApiKey(key_id),
                );
                let keys = crate::persistence::repos::ApiKeyRepo::list(&api_key_repo, &ctx)
                    .await
                    .map_err(|e| ApiError::Internal {
                        message: e.to_string(),
                    })?;

                if !keys.iter().any(|k| k.id == key_id) {
                    return Err(ApiError::NotFound);
                }

                let has_grant = grant_repo
                    .principal_has_any_grant_in_workspace(workspace.id, None, Some(key_id))
                    .await
                    .map_err(|e| ApiError::Internal {
                        message: e.to_string(),
                    })?;

                if !has_grant {
                    return Err(ApiError::NotFound);
                }

                Ok(WorkspaceAccess {
                    principal: atlas_domain::permissions::Principal::ApiKey(key_id),
                    workspace,
                    membership: None,
                })
            }
        }
    }
}

/// Proof that the request's principal is an authenticated user with `is_root = true`.
pub struct RequireUserAdmin {
    pub user: User,
}

impl FromRequestParts<AppState> for RequireUserAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let principal = parts
            .extensions
            .get::<Principal>()
            .cloned()
            .ok_or(ApiError::Unauthorized)?;

        let user_id = match principal {
            Principal::User(uid) => uid,
            Principal::ApiKey(_) => {
                return Err(ApiError::Forbidden {
                    message: "API keys cannot perform admin actions".into(),
                });
            }
        };

        let user_repo = PgUserRepo {
            conn: (*state.db).clone(),
        };
        let user = user_repo
            .find_by_id(user_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::Unauthorized)?;

        if user.disabled_at.is_some() {
            return Err(ApiError::Unauthorized);
        }

        if !user.is_root {
            return Err(ApiError::Forbidden {
                message: "Root access required".into(),
            });
        }

        Ok(RequireUserAdmin { user })
    }
}

#[allow(dead_code)]
fn user_id_unused(_: UserId) {}
