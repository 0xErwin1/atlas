use axum::{Json, extract::State};

use atlas_api::dtos::PrincipalDto;
use atlas_domain::Actor;

use crate::{
    authz::WorkspaceMember,
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
