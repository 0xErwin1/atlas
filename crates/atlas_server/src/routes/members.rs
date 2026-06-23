use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use atlas_api::dtos::{PrincipalDto, UpdateMemberRoleRequest};
use atlas_domain::{Actor, entities::identity::MemberRole};

use crate::{
    authz::{CallerClass, WorkspaceMember, WorkspaceOwnerOrAdmin},
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, MembershipRepo, PgApiKeyRepo, PgMembershipRepo, PgUserRepo, UserRepo,
    },
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/members",
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
        let display = user_repo
            .find_by_id(membership.user_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .map(|u| u.display_name)
            .unwrap_or_default();

        principals.push(PrincipalDto {
            principal_type: "user".to_string(),
            id: membership.user_id.0,
            display,
            key_type: None,
            role: Some(membership.role.as_str().to_string()),
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
        });
    }

    Ok(Json(principals))
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
    path = "/v1/workspaces/{ws}/members/{user_id}",
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

    let conn = (*state.db).clone();
    let membership_repo = PgMembershipRepo { conn: conn.clone() };

    let ctx =
        atlas_domain::WorkspaceCtx::new(caller.workspace.id, Actor::User(caller.caller_user_id));

    let target_membership = membership_repo
        .find(&ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let target_role = &target_membership.role;

    check_patch_permission(caller.caller_class, target_role, &new_role)?;

    if *target_role == MemberRole::Owner && new_role != MemberRole::Owner {
        check_last_owner_lockout(&membership_repo, &ctx, target_user_id).await?;
    }

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };
    let display = user_repo
        .find_by_id(target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .map(|u| u.display_name)
        .unwrap_or_default();

    let updated = membership_repo
        .update_role(&ctx, target_user_id, new_role)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(PrincipalDto {
        principal_type: "user".to_string(),
        id: target_user_id.0,
        display,
        key_type: None,
        role: Some(updated.role.as_str().to_string()),
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
    path = "/v1/workspaces/{ws}/members/{user_id}",
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
    let membership_repo = PgMembershipRepo { conn };

    let ctx =
        atlas_domain::WorkspaceCtx::new(caller.workspace.id, Actor::User(caller.caller_user_id));

    let target_membership = membership_repo
        .find(&ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let target_role = &target_membership.role;

    check_delete_permission(caller.caller_class, target_role)?;

    if *target_role == MemberRole::Owner {
        check_last_owner_lockout(&membership_repo, &ctx, target_user_id).await?;
    }

    membership_repo
        .remove(&ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
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
