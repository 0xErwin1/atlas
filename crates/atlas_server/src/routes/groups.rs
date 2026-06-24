use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use sea_orm::TransactionTrait;

use atlas_api::dtos::groups::{
    AddGroupMemberRequest, CreateGroupRequest, GroupDto, GroupMemberDto,
};
use atlas_domain::{
    Actor,
    entities::{
        groups::NewGroup,
        security_audit::{NewSecurityAuditEvent, SecurityAction},
    },
    ids::{GroupId, UserId},
};

use atlas_domain::ports::group_repo::GroupRepo;

use crate::{
    authz::WorkspaceOwnerOrAdmin,
    error::ApiError,
    persistence::repos::{MembershipRepo, PgGroupRepo, PgMembershipRepo, PgSecurityAuditRepo},
    state::AppState,
};

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/groups",
    tag = "groups",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
    ),
    request_body = CreateGroupRequest,
    responses(
        (status = 201, description = "Group created", body = GroupDto),
        (status = 400, description = "Malformed JSON body"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges — owner or admin required"),
        (status = 409, description = "A group with this name already exists in the workspace"),
        (status = 422, description = "Missing required fields"),
    )
)]
pub(crate) async fn create_group(
    caller: WorkspaceOwnerOrAdmin,
    State(state): State<AppState>,
    Json(body): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<GroupDto>), ApiError> {
    let name = body.name.trim().to_string();

    if name.is_empty() {
        return Err(ApiError::InvalidInput {
            message: "group name cannot be empty".into(),
        });
    }

    let conn = (*state.db).clone();
    let txn = conn.begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let group = PgGroupRepo::create_in(
        &txn,
        NewGroup {
            workspace_id: caller.workspace.id,
            name,
            created_by: caller.caller_user_id,
        },
    )
    .await
    .map_err(ApiError::Domain)?;

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(caller.workspace.id),
            actor: Actor::User(caller.caller_user_id),
            action: SecurityAction::GroupCreated,
            target_type: "group".to_string(),
            target_id: Some(group.id.0),
            metadata: serde_json::json!({ "name": group.name }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok((StatusCode::CREATED, Json(group_to_dto(group))))
}

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/groups",
    tag = "groups",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
    ),
    responses(
        (status = 200, description = "Active groups in the workspace", body = [GroupDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
    )
)]
pub(crate) async fn list_groups(
    caller: WorkspaceOwnerOrAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<GroupDto>>, ApiError> {
    let conn = (*state.db).clone();
    let repo = PgGroupRepo { conn };

    let groups = repo
        .list(caller.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(groups.into_iter().map(group_to_dto).collect()))
}

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/groups/{group_id}",
    tag = "groups",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("group_id" = uuid::Uuid, Path, description = "Group ID"),
    ),
    responses(
        (status = 204, description = "Group soft-deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges"),
        (status = 404, description = "Group not found"),
    )
)]
pub(crate) async fn delete_group(
    caller: WorkspaceOwnerOrAdmin,
    Path((_ws, group_uuid)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let group_id = GroupId(group_uuid);

    let conn = (*state.db).clone();
    let txn = conn.begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let found = PgGroupRepo::soft_delete_in(&txn, group_id, caller.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if !found {
        return Err(ApiError::NotFound);
    }

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(caller.workspace.id),
            actor: Actor::User(caller.caller_user_id),
            action: SecurityAction::GroupDeleted,
            target_type: "group".to_string(),
            target_id: Some(group_id.0),
            metadata: serde_json::json!({}),
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

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/groups/{group_id}/members",
    tag = "groups",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("group_id" = uuid::Uuid, Path, description = "Group ID"),
    ),
    request_body = AddGroupMemberRequest,
    responses(
        (status = 201, description = "Member added to group", body = GroupMemberDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges"),
        (status = 404, description = "Group not found"),
        (status = 422, description = "User is not a workspace member"),
    )
)]
pub(crate) async fn add_group_member(
    caller: WorkspaceOwnerOrAdmin,
    Path((_ws, group_uuid)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
    Json(body): Json<AddGroupMemberRequest>,
) -> Result<(StatusCode, Json<GroupMemberDto>), ApiError> {
    let group_id = GroupId(group_uuid);
    let target_user_id = UserId(body.user_id);

    let conn = (*state.db).clone();

    // Verify the group exists and belongs to this workspace.
    let repo = PgGroupRepo { conn: conn.clone() };
    let group = repo
        .get(group_id, caller.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    // Verify the target user is a workspace member.
    let membership_repo = PgMembershipRepo { conn: conn.clone() };
    let ctx =
        atlas_domain::WorkspaceCtx::new(caller.workspace.id, Actor::User(caller.caller_user_id));

    let is_member = membership_repo
        .find(&ctx, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .is_some();

    if !is_member {
        return Err(ApiError::InvalidInput {
            message: "the specified user is not a member of this workspace".into(),
        });
    }

    let txn = conn.begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let member = PgGroupRepo::add_member_in(&txn, group_id, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(caller.workspace.id),
            actor: Actor::User(caller.caller_user_id),
            action: SecurityAction::GroupMemberAdded,
            target_type: "group".to_string(),
            target_id: Some(group.id.0),
            metadata: serde_json::json!({ "user_id": target_user_id.0 }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok((
        StatusCode::CREATED,
        Json(GroupMemberDto {
            group_id: group_id.0,
            user_id: target_user_id.0,
            created_at: member.created_at,
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/groups/{group_id}/members/{user_id}",
    tag = "groups",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("group_id" = uuid::Uuid, Path, description = "Group ID"),
        ("user_id" = uuid::Uuid, Path, description = "User ID to remove"),
    ),
    responses(
        (status = 204, description = "Member removed from group"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges"),
        (status = 404, description = "Group or membership not found"),
    )
)]
pub(crate) async fn remove_group_member(
    caller: WorkspaceOwnerOrAdmin,
    Path((_ws, group_uuid, user_uuid)): Path<(String, uuid::Uuid, uuid::Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let group_id = GroupId(group_uuid);
    let target_user_id = UserId(user_uuid);

    let conn = (*state.db).clone();

    // Verify the group exists.
    let repo = PgGroupRepo { conn: conn.clone() };
    let group = repo
        .get(group_id, caller.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let txn = conn.begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let removed = PgGroupRepo::remove_member_in(&txn, group_id, target_user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if !removed {
        return Err(ApiError::NotFound);
    }

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: Some(caller.workspace.id),
            actor: Actor::User(caller.caller_user_id),
            action: SecurityAction::GroupMemberRemoved,
            target_type: "group".to_string(),
            target_id: Some(group.id.0),
            metadata: serde_json::json!({ "user_id": target_user_id.0 }),
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

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/groups/{group_id}/members",
    tag = "groups",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("group_id" = uuid::Uuid, Path, description = "Group ID"),
    ),
    responses(
        (status = 200, description = "Members of the group", body = [GroupMemberDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient privileges"),
        (status = 404, description = "Group not found"),
    )
)]
pub(crate) async fn list_group_members(
    caller: WorkspaceOwnerOrAdmin,
    Path((_ws, group_uuid)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<GroupMemberDto>>, ApiError> {
    let group_id = GroupId(group_uuid);

    let conn = (*state.db).clone();
    let repo = PgGroupRepo { conn };

    // Verify the group exists and belongs to this workspace.
    repo.get(group_id, caller.workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let members = repo
        .list_members(group_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(
        members
            .into_iter()
            .map(|m| GroupMemberDto {
                group_id: m.group_id.0,
                user_id: m.user_id.0,
                created_at: m.created_at,
            })
            .collect(),
    ))
}

fn group_to_dto(g: atlas_domain::entities::groups::Group) -> GroupDto {
    GroupDto {
        id: g.id.0,
        workspace_id: g.workspace_id.0,
        name: g.name,
        created_by: g.created_by.0,
        created_at: g.created_at,
        updated_at: g.updated_at,
    }
}
