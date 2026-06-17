use axum::{
    Json,
    extract::{Extension, State},
};

use atlas_api::dtos::WorkspaceDto;

use crate::{
    auth::middleware::Principal,
    authz::WorkspaceMember,
    error::ApiError,
    persistence::repos::{PgWorkspaceRepo, WorkspaceRepo},
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/v1/workspaces",
    tag = "workspaces",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Workspaces accessible to the caller", body = [WorkspaceDto]),
        (status = 401, description = "Unauthenticated"),
    )
)]
/// Returns the workspaces the authenticated principal is a member of.
/// API keys are workspace-scoped and do not use this endpoint; the result is always empty for them.
pub(crate) async fn list_workspaces(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<WorkspaceDto>>, ApiError> {
    let user_id = match principal {
        Principal::User(uid) => uid,
        Principal::ApiKey(_) => return Ok(Json(Vec::new())),
    };

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    let workspaces = ws_repo
        .list_for_user(user_id)
        .await
        .map_err(|_| ApiError::Internal {
            message: "workspace lookup failed".into(),
        })?;

    let dtos = workspaces
        .into_iter()
        .map(|ws| WorkspaceDto {
            id: ws.id.0,
            name: ws.name,
            slug: ws.slug,
            created_at: ws.created_at,
            updated_at: ws.updated_at,
        })
        .collect();

    Ok(Json(dtos))
}

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}",
    tag = "workspaces",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 200, description = "Workspace details", body = WorkspaceDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or not a member"),
    )
)]
pub(crate) async fn get_workspace(
    member: WorkspaceMember,
    State(_state): State<AppState>,
) -> Result<Json<WorkspaceDto>, ApiError> {
    let ws = &member.workspace;
    Ok(Json(WorkspaceDto {
        id: ws.id.0,
        name: ws.name.clone(),
        slug: ws.slug.clone(),
        created_at: ws.created_at,
        updated_at: ws.updated_at,
    }))
}
