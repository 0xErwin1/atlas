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
