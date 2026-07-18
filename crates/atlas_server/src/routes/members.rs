use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use sea_orm::TransactionTrait;

use atlas_api::dtos::{AddMemberRequest, PrincipalDto, UpdateMemberRoleRequest, UserDto};
use atlas_domain::{
    Actor,
    entities::{
        identity::MemberRole,
        security_audit::{NewSecurityAuditEvent, SecurityAction},
    },
    error::DomainError,
};

use crate::{
    authz::{CallerClass, WorkspaceMember, WorkspaceOwnerOrAdmin},
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, MembershipRepo, PgApiKeyRepo, PgMembershipRepo, PgSecurityAuditRepo,
        PgUserRepo, UserRepo,
    },
    routes::account_status,
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/members",
    tag = "members",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
    ),
    responses(
        (status = 200, description = "Workspace members and agents", body = [PrincipalDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or principal is not a member"),
    )
)]
pub(crate) async fn list_workspace_members(
    member: WorkspaceMember,
    State(state): State<AppState>,
) -> Result<Json<Vec<PrincipalDto>>, ApiError> {
    let actor = match (&member.user, &member.api_key_id) {
        (Some(user), _) => Actor::User(user.id),
        (None, Some(key_id)) => Actor::ApiKey(*key_id),
        (None, None) => return Err(ApiError::Unauthorized),
    };
    let ctx = atlas_domain::WorkspaceCtx::new(member.workspace.id, actor);

    let conn = (*state.db).clone();
    let membership_repo = PgMembershipRepo { conn: conn.clone() };
    let user_repo = PgUserRepo { conn: conn.clone() };
    let api_key_repo = PgApiKeyRepo { conn };

    let memberships = membership_repo
        .list(&ctx)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let mut principals = Vec::with_capacity(memberships.len());

    for membership in &memberships {
        let user = user_repo
            .find_by_id(membership.user_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;

        let display = user
            .as_ref()
            .map(|u| u.display_name.clone())
            .unwrap_or_default();

        let status = user
            .as_ref()
            .map(|u| account_status(u.disabled_at, u.activated_at).to_string());

        principals.push(PrincipalDto {
            principal_type: "user".to_string(),
            id: membership.user_id.0,
            display,
            key_type: None,
            role: Some(membership.role.as_str().to_string()),
            account_status: status,
        });
    }

    // List api keys that have at least one grant in this workspace, rather than
    // keys bound by the deprecated workspace_id FK.
    let api_keys = api_key_repo
        .list_granted_in_workspace(member.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    for key in &api_keys {
        principals.push(PrincipalDto {
            principal_type: "api_key".to_string(),
            id: key.id.0,
            display: key.name.clone(),
            key_type: Some(key.type_.as_str().to_string()),
            role: None,
            account_status: None,
        });
    }

    Ok(Json(principals))
}

/// Adds an existing user to the workspace at the requested role.
///
/// Mirrors the authorization discipline of `update_member_role`:
/// 1. `WorkspaceOwnerOrAdmin` gates structural access (resolves caller class).
/// 2. Role string is parsed; an unknown value → 422.
/// 3. Role-grant matrix: an admin may add `member` or `admin`, never `owner`;
///    only an owner (or break-glass) may add an `owner`.
/// 4. The target user must exist (404) and must not be disabled (422 — a
///    deactivated account cannot be assigned).
/// 5. The user must not already be a member → 409 Conflict.
/// 6. The membership insert and its audit row share one transaction.
#[utoipa::path(
    post,
    path = "/api/workspaces/{ws}/members",
    tag = "members",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
    ),
    request_body = AddMemberRequest,
    responses(
        (status = 201, description = "User added to the workspace", body = PrincipalDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges to grant the requested role"),
        (status = 404, description = "Workspace or target user not found"),
        (status = 409, description = "User is already a member"),
        (status = 422, description = "Unknown role string, or target user is disabled"),
    )
)]
pub(crate) async fn add_member(
    caller: WorkspaceOwnerOrAdmin,
    Path(_ws): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<AddMemberRequest>,
) -> Result<(StatusCode, Json<PrincipalDto>), ApiError> {
    let target_user_id = atlas_domain::ids::UserId(body.user_id);
    let new_role = parse_role(&body.role)?;

    check_add_permission(caller.caller_class, &new_role)?;

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    check_root_target_protection(&user_repo, caller.caller_is_root, target_user_id).await?;

    let target_user = user_repo
        .find_by_id(target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    if target_user.disabled_at.is_some() {
        return Err(ApiError::InvalidInput {
            message: "cannot add a deactivated account to a workspace; re-enable the user first"
                .into(),
        });
    }

    let ctx =
        atlas_domain::WorkspaceCtx::new(caller.workspace.id, Actor::User(caller.caller_user_id));

    let membership_repo = PgMembershipRepo {
        conn: (*state.db).clone(),
    };
    let existing = membership_repo
        .find(&ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if existing.is_some() {
        return Err(ApiError::Domain(DomainError::AlreadyExists {
            message: "user is already a member of this workspace".into(),
        }));
    }

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    // The membership row and the audit row commit or roll back together.
    let added = PgMembershipRepo::add_in(&txn, &ctx, target_user_id, new_role)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(caller.workspace.id),
            actor: Actor::User(caller.caller_user_id),
            action: SecurityAction::MembershipAdded,
            target_type: "user".to_string(),
            target_id: Some(target_user_id.0),
            metadata: serde_json::json!({
                "role": added.role.as_str(),
            }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let status = account_status(target_user.disabled_at, target_user.activated_at).to_string();

    Ok((
        StatusCode::CREATED,
        Json(PrincipalDto {
            principal_type: "user".to_string(),
            id: target_user_id.0,
            display: target_user.display_name.clone(),
            key_type: None,
            role: Some(added.role.as_str().to_string()),
            account_status: Some(status),
        }),
    ))
}

/// Lists users who can be added to the workspace.
///
/// Returns the active (non-disabled) users that are NOT already members of this
/// workspace. The `users` table holds only human accounts, so api keys are
/// excluded by construction. Mirrors the `GET /api/users` shape (`Vec<UserDto>`).
#[utoipa::path(
    get,
    path = "/api/workspaces/{ws}/assignable-users",
    tag = "members",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
    ),
    responses(
        (status = 200, description = "Users that can be added as members", body = [UserDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges"),
        (status = 404, description = "Workspace not found"),
    )
)]
pub(crate) async fn list_assignable_users(
    caller: WorkspaceOwnerOrAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let ctx =
        atlas_domain::WorkspaceCtx::new(caller.workspace.id, Actor::User(caller.caller_user_id));

    let membership_repo = PgMembershipRepo {
        conn: (*state.db).clone(),
    };
    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let memberships = membership_repo
        .list(&ctx)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let member_ids: std::collections::HashSet<_> = memberships.iter().map(|m| m.user_id).collect();

    let users = user_repo.list().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let assignable = users
        .iter()
        .filter(|u| u.disabled_at.is_none() && !member_ids.contains(&u.id))
        .map(crate::routes::users::user_to_dto)
        .collect();

    Ok(Json(assignable))
}

/// Changes a workspace member's role.
///
/// Enforces strictly ordered authorization checks:
/// 1. `WorkspaceOwnerOrAdmin` extractor gates structural access (resolves caller class).
/// 2. Target membership is loaded; 404 if the target is not a member.
/// 3. Role-permission matrix: admins cannot touch owners or promote to owner; others
///    are allowed subject to step 4.
/// 4. Last-owner-lockout (independent of step 3, applies to break-glass too): demoting
///    the sole remaining owner → 409. This is a data-integrity invariant, not a
///    permission — break-glass is NOT exempt.
/// 5. Apply `update_role`. Idempotent: same-role PATCH validates the full matrix first.
///
/// Steps 3 and 4 are independent and strictly ordered. Never collapse them.
#[utoipa::path(
    patch,
    path = "/api/workspaces/{ws}/members/{user_id}",
    tag = "members",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("user_id" = uuid::Uuid, Path, description = "Target member user ID"),
    ),
    request_body = UpdateMemberRoleRequest,
    responses(
        (status = 200, description = "Role updated", body = PrincipalDto),
        (status = 400, description = "Malformed JSON body"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges"),
        (status = 404, description = "Workspace or target member not found"),
        (status = 409, description = "Last-owner lockout"),
        (status = 422, description = "Unknown role string"),
    )
)]
pub(crate) async fn update_member_role(
    caller: WorkspaceOwnerOrAdmin,
    Path((_ws, target_user_uuid)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
    Json(body): Json<UpdateMemberRoleRequest>,
) -> Result<Json<PrincipalDto>, ApiError> {
    let target_user_id = atlas_domain::ids::UserId(target_user_uuid);
    let new_role = parse_role(&body.role)?;

    // Self-protection fires before check_patch_permission and before any
    // break-glass bypass, so even a system_admin (global admin) cannot change
    // their own workspace role. Another admin must do it.
    if caller.caller_user_id == target_user_id {
        return Err(ApiError::Forbidden {
            message: "you cannot change your own workspace role; ask another admin".into(),
        });
    }

    let conn = (*state.db).clone();
    let membership_repo = PgMembershipRepo { conn: conn.clone() };
    let user_repo = PgUserRepo { conn: conn.clone() };

    let ctx =
        atlas_domain::WorkspaceCtx::new(caller.workspace.id, Actor::User(caller.caller_user_id));

    let target_membership = membership_repo
        .find(&ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    check_root_target_protection(&user_repo, caller.caller_is_root, target_user_id).await?;

    let target_role = &target_membership.role;

    check_patch_permission(caller.caller_class, target_role, &new_role)?;

    if *target_role == MemberRole::Owner && new_role != MemberRole::Owner {
        check_last_owner_lockout(&membership_repo, &ctx, target_user_id).await?;
    }

    let user = user_repo
        .find_by_id(target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let display = user
        .as_ref()
        .map(|u| u.display_name.clone())
        .unwrap_or_default();
    let status = user
        .as_ref()
        .map(|u| account_status(u.disabled_at, u.activated_at).to_string());

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let updated = PgMembershipRepo::update_role_in(&txn, &ctx, target_user_id, new_role)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    // The audit row and the role update commit or roll back together.
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(caller.workspace.id),
            actor: Actor::User(caller.caller_user_id),
            action: SecurityAction::MembershipRoleChanged,
            target_type: "user".to_string(),
            target_id: Some(target_user_id.0),
            metadata: serde_json::json!({
                "old_role": target_role.as_str(),
                "new_role": updated.role.as_str(),
            }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(Json(PrincipalDto {
        principal_type: "user".to_string(),
        id: target_user_id.0,
        display,
        key_type: None,
        role: Some(updated.role.as_str().to_string()),
        account_status: status,
    }))
}

/// Removes a member from the workspace.
///
/// Enforces the same ordered authorization checks as `update_member_role`, adapted for
/// DELETE:
/// - Step 3: admin cannot remove an owner.
/// - Step 4: no one (including break-glass) can remove the last owner → 409.
#[utoipa::path(
    delete,
    path = "/api/workspaces/{ws}/members/{user_id}",
    tag = "members",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("user_id" = uuid::Uuid, Path, description = "Target member user ID"),
    ),
    responses(
        (status = 204, description = "Member removed"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges"),
        (status = 404, description = "Workspace or target member not found"),
        (status = 409, description = "Last-owner lockout"),
    )
)]
pub(crate) async fn remove_member(
    caller: WorkspaceOwnerOrAdmin,
    Path((_ws, target_user_uuid)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let target_user_id = atlas_domain::ids::UserId(target_user_uuid);

    let conn = (*state.db).clone();
    let membership_repo = PgMembershipRepo { conn: conn.clone() };
    let user_repo = PgUserRepo { conn };

    let ctx =
        atlas_domain::WorkspaceCtx::new(caller.workspace.id, Actor::User(caller.caller_user_id));

    let target_membership = membership_repo
        .find(&ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    check_root_target_protection(&user_repo, caller.caller_is_root, target_user_id).await?;

    let target_role = &target_membership.role;

    check_delete_permission(caller.caller_class, target_role)?;

    if *target_role == MemberRole::Owner {
        check_last_owner_lockout(&membership_repo, &ctx, target_user_id).await?;
    }

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    PgMembershipRepo::remove_in(&txn, &ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    // The audit row and the removal commit or roll back together.
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(caller.workspace.id),
            actor: Actor::User(caller.caller_user_id),
            action: SecurityAction::MembershipRemoved,
            target_type: "user".to_string(),
            target_id: Some(target_user_id.0),
            metadata: serde_json::json!({
                "role": target_role.as_str(),
            }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(StatusCode::NO_CONTENT)
}

fn parse_role(s: &str) -> Result<MemberRole, ApiError> {
    match s {
        "owner" => Ok(MemberRole::Owner),
        "admin" => Ok(MemberRole::Admin),
        "member" => Ok(MemberRole::Member),
        _ => Err(ApiError::InvalidInput {
            message: format!("unknown role '{s}'; valid values are owner, admin, member"),
        }),
    }
}

/// Authorizes adding a member at `new_role`.
///
/// An admin caller may grant `member` or `admin`, but never `owner`; only an
/// owner (or break-glass) may add an owner. This mirrors the `new_role == Owner`
/// branch of `check_patch_permission`.
fn check_add_permission(caller_class: CallerClass, new_role: &MemberRole) -> Result<(), ApiError> {
    if caller_class == CallerClass::Admin && *new_role == MemberRole::Owner {
        return Err(ApiError::Forbidden {
            message: "Only an owner can grant the owner role".into(),
        });
    }
    Ok(())
}

fn check_patch_permission(
    caller_class: CallerClass,
    target_role: &MemberRole,
    new_role: &MemberRole,
) -> Result<(), ApiError> {
    if caller_class == CallerClass::Admin {
        if *target_role == MemberRole::Owner {
            return Err(ApiError::Forbidden {
                message: "Admins cannot modify an owner's membership".into(),
            });
        }
        if *new_role == MemberRole::Owner {
            return Err(ApiError::Forbidden {
                message: "Only an owner can grant the owner role".into(),
            });
        }
    }
    Ok(())
}

fn check_delete_permission(
    caller_class: CallerClass,
    target_role: &MemberRole,
) -> Result<(), ApiError> {
    if caller_class == CallerClass::Admin && *target_role == MemberRole::Owner {
        return Err(ApiError::Forbidden {
            message: "Admins cannot modify an owner's membership".into(),
        });
    }
    Ok(())
}

/// Blocks a non-root caller from managing the root user's membership.
///
/// `WorkspaceOwnerOrAdmin` resolves a system-admin as `CallerClass::BreakGlass`,
/// so without this guard a non-root system-admin could add, re-role, or remove
/// the root user's per-workspace membership. Only root may manage the root user.
/// The guard is scoped strictly to a root target — system-admin targets stay
/// manageable — mirroring the root-target guard in `disable_user`/`reset_password`.
async fn check_root_target_protection(
    user_repo: &PgUserRepo,
    caller_is_root: bool,
    target_user_id: atlas_domain::ids::UserId,
) -> Result<(), ApiError> {
    if caller_is_root {
        return Ok(());
    }

    let target = user_repo
        .find_by_id(target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    if target.is_root {
        return Err(ApiError::Forbidden {
            message: "Only root can manage the root user".into(),
        });
    }

    Ok(())
}

/// Checks the last-owner-lockout invariant.
///
/// A workspace must always retain at least one owner. This check is independent of
/// the caller's permission level — it applies to everyone, including break-glass
/// (root/system-admin). The SELECT-then-mutate is not atomic: two simultaneous
/// demotions of the last two owners could both pass. This race is accepted at this
/// product scale (single-admin concurrency is not a realistic threat).
async fn check_last_owner_lockout(
    membership_repo: &PgMembershipRepo,
    ctx: &atlas_domain::WorkspaceCtx,
    target_user_id: atlas_domain::ids::UserId,
) -> Result<(), ApiError> {
    let all_members = membership_repo
        .list(ctx)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let owners: Vec<_> = all_members
        .iter()
        .filter(|m| m.role == MemberRole::Owner)
        .collect();

    if owners.len() == 1 && owners.first().map(|o| o.user_id) == Some(target_user_id) {
        return Err(ApiError::LastOwner {
            message: "A workspace must keep at least one owner".into(),
        });
    }

    Ok(())
}
