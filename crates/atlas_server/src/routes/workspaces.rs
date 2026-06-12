use axum::{Json, extract::State};

use atlas_api::dtos::WorkspaceDto;

use crate::{authz::WorkspaceMember, error::ApiError, state::AppState};

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
