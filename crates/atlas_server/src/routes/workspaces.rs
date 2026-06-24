use axum::{
    Json,
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::{CreateWorkspaceRequest, UpdateWorkspaceRequest, WorkspaceDto};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{MemberRole, NewWorkspace},
    ids::WorkspaceId,
    resolve_collision, slugify,
};

use crate::{
    auth::middleware::Principal,
    authz::{RequireUserAdmin, WorkspaceMember},
    error::ApiError,
    persistence::repos::{
        MembershipRepo, PgMembershipRepo, PgUserRepo, PgWorkspaceRepo, UserRepo, WorkspaceRepo,
    },
    routes::validation::validate_name,
    state::AppState,
};

#[utoipa::path(
    post,
    path = "/v1/workspaces",
    tag = "workspaces",
    security(("bearer_auth" = [])),
    request_body = CreateWorkspaceRequest,
    responses(
        (status = 201, description = "Workspace created", body = WorkspaceDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot create workspaces"),
    )
)]
/// Creates a new workspace owned by the authenticated human user.
///
/// The slug is derived from the name and de-duplicated against existing
/// workspace slugs. The creating user is added as `Owner`, so the workspace
/// immediately appears in `GET /v1/workspaces`. API keys (agents) are rejected
/// with 403: agents are workspace-scoped and must not create workspaces.
pub(crate) async fn create_workspace(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<CreateWorkspaceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let Principal::User(user_id) = principal else {
        return Err(ApiError::Forbidden {
            message: "API keys cannot create workspaces".into(),
        });
    };

    validate_name("name", &body.name)?;

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: (*state.db).clone(),
    };

    let existing_slugs = ws_repo.list_slugs().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;
    let taken: Vec<&str> = existing_slugs.iter().map(String::as_str).collect();
    let slug = resolve_collision(&slugify(&body.name), &taken);

    let workspace_id = WorkspaceId::new();
    let workspace = ws_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: body.name,
            slug,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let ctx = WorkspaceCtx::new(workspace.id, Actor::User(user_id));
    membership_repo
        .add(&ctx, user_id, MemberRole::Owner)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok((StatusCode::CREATED, Json(workspace_to_dto(&workspace))))
}

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

    let workspaces = if user.is_root || user.is_system_admin {
        ws_repo.list_all().await.map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
    } else {
        ws_repo
            .list_for_user(user_id)
            .await
            .map_err(|_| ApiError::Internal {
                message: "workspace lookup failed".into(),
            })?
    };

    let dtos = workspaces.iter().map(workspace_to_dto).collect();

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
    Ok(Json(workspace_to_dto(&member.workspace)))
}

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}",
    tag = "workspaces",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = UpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Workspace renamed", body = WorkspaceDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or not a member"),
        (status = 422, description = "Validation error"),
    )
)]
/// Renames the workspace display name. The slug is never re-derived; only
/// `name` and `updated_at` change. Requires workspace membership.
pub(crate) async fn update_workspace(
    member: WorkspaceMember,
    State(state): State<AppState>,
    Json(body): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceDto>, ApiError> {
    validate_name("name", &body.name)?;

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    let updated = ws_repo
        .rename(member.workspace.id, body.name)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(Json(workspace_to_dto(&updated)))
}

#[utoipa::path(
    get,
    path = "/v1/admin/workspaces",
    tag = "workspaces",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "All workspaces (root only)", body = [WorkspaceDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
    )
)]
/// Returns every workspace in the system, ordered by creation date.
/// Restricted to root users via `RequireUserAdmin`.
pub(crate) async fn admin_list_workspaces(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<WorkspaceDto>>, ApiError> {
    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    let workspaces = ws_repo.list_all().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let dtos = workspaces.iter().map(workspace_to_dto).collect();
    Ok(Json(dtos))
}

fn workspace_to_dto(ws: &atlas_domain::entities::identity::Workspace) -> WorkspaceDto {
    WorkspaceDto {
        id: ws.id.0,
        name: ws.name.clone(),
        slug: ws.slug.clone(),
        created_at: ws.created_at,
        updated_at: ws.updated_at,
    }
}
