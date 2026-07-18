use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
};
use std::collections::HashMap;

use atlas_domain::{
    entities::identity::{MemberRole, User, WorkspaceMembership},
    ids::{ApiKeyId, UserId},
};

use crate::{
    auth::middleware::Principal,
    authz::authorized::ReadScopeSet,
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, MembershipRepo, PgApiKeyRepo, PgMembershipRepo, PgUserRepo, PgWorkspaceRepo,
        UserRepo, Workspace, WorkspaceRepo,
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

                // is_root and is_system_admin get global admin access to every workspace
                // without being a member. This is a security-load-bearing short-circuit:
                // weakening this check would silently remove global-admin visibility.
                if user.is_root || user.is_system_admin {
                    return Ok(WorkspaceMember {
                        workspace,
                        user: Some(user),
                        api_key_id: None,
                        membership: None,
                    });
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
                if !crate::authz::authorized::api_key_can_access_workspace(
                    &state.db, key_id, &workspace,
                )
                .await?
                {
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
    /// True only when a `is_root || is_system_admin` user bypasses the normal
    /// membership/grant gate. Never true for an ApiKey principal.
    /// Consumed by the search route to short-circuit the SQL permission predicate.
    pub bypass: bool,
    /// The API key's read-capability set, present ONLY for an `ApiKey` principal.
    /// `None` for users (and groups): humans have no scope axis and read every
    /// family. The search route uses this to gate cross-family read feeds so an
    /// agent only sees families it holds `{family}:read` on.
    pub read_scopes: Option<ReadScopeSet>,
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

                // is_root and is_system_admin bypass membership and grant checks.
                // bypass = true signals the search SQL to short-circuit the permission predicate.
                if user.is_root || user.is_system_admin {
                    return Ok(WorkspaceAccess {
                        principal: atlas_domain::permissions::Principal::User(user_id),
                        workspace,
                        membership: Some(atlas_domain::entities::identity::MemberRole::Admin),
                        bypass: true,
                        read_scopes: None,
                    });
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
                        .principal_has_any_grant_in_workspace(workspace.id, Some(user_id), None)
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
                    bypass: false,
                    read_scopes: None,
                })
            }
            Principal::ApiKey(key_id) => {
                if !crate::authz::authorized::api_key_can_access_workspace(
                    &state.db, key_id, &workspace,
                )
                .await?
                {
                    return Err(ApiError::NotFound);
                }

                // Load the key once to derive its read-capability set; the search
                // route gates cross-family read feeds on this so an agent only sees
                // families it holds `{family}:read` on.
                let key_repo = PgApiKeyRepo {
                    conn: (*state.db).clone(),
                };
                let key = key_repo
                    .get_by_id(key_id)
                    .await
                    .map_err(|e| ApiError::Internal {
                        message: e.to_string(),
                    })?
                    .ok_or(ApiError::NotFound)?;

                Ok(WorkspaceAccess {
                    principal: atlas_domain::permissions::Principal::ApiKey(key_id),
                    workspace,
                    membership: None,
                    bypass: false,
                    read_scopes: Some(ReadScopeSet::from_scopes(&key.scopes)),
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

        if !(user.is_root || user.is_system_admin) {
            return Err(ApiError::Forbidden {
                message: "Admin access required".into(),
            });
        }

        Ok(RequireUserAdmin { user })
    }
}

/// Proof that the request's principal is an authenticated user with `is_root = true`.
///
/// This is the break-glass guard for operations that only the single root account
/// may perform — specifically, promoting and demoting system-admins. A system-admin
/// cannot satisfy this extractor; only `is_root` passes.
pub struct RequireRoot {
    pub user: User,
}

impl FromRequestParts<AppState> for RequireRoot {
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
                    message: "API keys cannot perform root-only actions".into(),
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

        Ok(RequireRoot { user })
    }
}

/// Which class of caller the `WorkspaceOwnerOrAdmin` extractor resolved.
///
/// Break-glass (`is_root || is_system_admin`) can manage any workspace regardless of
/// membership. Owner and Admin are workspace-role–bound. The handler uses this to enforce
/// the per-action permission matrix without re-loading the caller's membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerClass {
    BreakGlass,
    Owner,
    Admin,
}

/// Proof that the caller may perform member-management actions in the workspace.
///
/// Structural gate for `PATCH` and `DELETE` on `/api/workspaces/{ws}/members/{user_id}`.
/// It resolves one of three caller classes:
/// - `BreakGlass`: `user.is_root || user.is_system_admin` — no membership required in
///   this workspace; can act on any workspace.
/// - `Owner`: caller has `MemberRole::Owner` in this workspace.
/// - `Admin`: caller has `MemberRole::Admin` in this workspace.
///
/// Rejected cases:
/// - No `Principal` extension → 401 Unauthorized.
/// - `Principal::ApiKey` → 403 (api keys cannot manage members).
/// - User with `MemberRole::Member` or no membership (and not break-glass) → 403.
pub struct WorkspaceOwnerOrAdmin {
    pub workspace: Workspace,
    pub caller_user_id: UserId,
    pub caller_class: CallerClass,
    /// Whether the caller is the root user. `CallerClass::BreakGlass` covers both
    /// root and system-admins, so this is the only way to distinguish the two;
    /// member-management routes use it to protect the root user from a non-root
    /// caller. A workspace-role caller (`Owner`/`Admin`) is never root.
    pub caller_is_root: bool,
}

impl FromRequestParts<AppState> for WorkspaceOwnerOrAdmin {
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
                    message: "API keys cannot manage workspace members".into(),
                });
            }
        };

        let Path(params): Path<HashMap<String, String>> = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiError::NotFound)?;

        let ws_slug = params.get("ws").ok_or(ApiError::NotFound)?.clone();

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

        if user.is_root || user.is_system_admin {
            return Ok(WorkspaceOwnerOrAdmin {
                workspace,
                caller_user_id: user_id,
                caller_class: CallerClass::BreakGlass,
                caller_is_root: user.is_root,
            });
        }

        let membership_repo = PgMembershipRepo {
            conn: (*state.db).clone(),
        };
        let ctx = atlas_domain::WorkspaceCtx::new(workspace.id, atlas_domain::Actor::User(user_id));
        let membership =
            membership_repo
                .find(&ctx, user_id)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?;

        let caller_class = match membership.map(|m| m.role) {
            Some(MemberRole::Owner) => CallerClass::Owner,
            Some(MemberRole::Admin) => CallerClass::Admin,
            Some(MemberRole::Member) | None => {
                return Err(ApiError::Forbidden {
                    message: "Only workspace owners and admins can manage members".into(),
                });
            }
        };

        Ok(WorkspaceOwnerOrAdmin {
            workspace,
            caller_user_id: user_id,
            caller_class,
            caller_is_root: false,
        })
    }
}

#[allow(dead_code)]
fn user_id_unused(_: UserId) {}
