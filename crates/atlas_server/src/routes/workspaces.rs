use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::{
    AdminUpdateWorkspaceRequest, CreateWorkspaceRequest, UpdateWorkspaceRequest, WorkspaceDto,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::NewBoard,
    entities::identity::{MemberRole, NewWorkspace, Workspace},
    entities::status_templates::NewStatusTemplate,
    entities::workspace_core::NewProject,
    ids::{UserId, WorkspaceId},
    permissions::{Capability, CapabilityAction, CapabilityFamily, Visibility, VisibilityRole},
    position, resolve_collision, slugify,
};

use crate::{
    auth::middleware::Principal,
    authz::{RequireUserAdmin, WorkspaceMember, enforce_api_key_scope},
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, BoardRepo, MembershipRepo, PgApiKeyRepo, PgBoardRepo, PgMembershipRepo,
        PgProjectRepo, PgStatusTemplateRepo, PgUserRepo, PgWorkspaceRepo, ProjectRepo,
        StatusTemplateRepo, UserRepo, WorkspaceRepo,
    },
    routes::validation::{validate_name, validate_slug},
    state::AppState,
};

/// Status columns every new workspace starts with, in board order. The default
/// board derives its columns from these, so a freshly created workspace has a
/// usable kanban out of the box instead of an empty, column-less board.
const DEFAULT_STATUSES: &[(&str, &str)] = &[
    ("To Do", "neutral"),
    ("In Progress", "blue"),
    ("Done", "green"),
];

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

    seed_default_content(&state, workspace.id, user_id).await?;

    Ok((StatusCode::CREATED, Json(workspace_to_dto(&workspace))))
}

/// Seeds a new workspace with the scaffolding that makes it usable on first open:
/// a default project, the default status templates, and a default board (whose
/// columns are derived from those templates). The creator already has Owner
/// membership, which resolves to Admin on all workspace content, so no explicit
/// grant is needed here.
///
/// Best-effort consistency: each step runs in its own repository call rather than
/// one transaction, matching the surrounding create flow. A partial failure leaves
/// the workspace under-seeded but still valid; the UI renders missing scaffolding
/// as empty states, never as an error.
async fn seed_default_content(
    state: &AppState,
    workspace_id: WorkspaceId,
    creator: UserId,
) -> Result<(), ApiError> {
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(creator));

    let project = PgProjectRepo {
        conn: (*state.db).clone(),
    }
    .create(
        &ctx,
        NewProject {
            name: "General".to_string(),
            slug: "general".to_string(),
            task_prefix: "GEN".to_string(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .map_err(ApiError::Domain)?;

    let template_repo = PgStatusTemplateRepo::new((*state.db).clone());
    let mut prev_key: Option<String> = None;
    for (name, color) in DEFAULT_STATUSES {
        let position_key = position::between(prev_key.as_deref(), None);
        template_repo
            .create(
                &ctx,
                NewStatusTemplate {
                    name: (*name).to_string(),
                    color: Some((*color).to_string()),
                    position_key: position_key.clone(),
                },
            )
            .await
            .map_err(ApiError::Domain)?;
        prev_key = Some(position_key);
    }

    PgBoardRepo::new((*state.db).clone())
        .create_board(
            &ctx,
            NewBoard {
                project_id: project.id,
                name: "Board".to_string(),
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok(())
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
/// Returns the workspaces the authenticated principal can access.
///
/// For users: their member workspaces (or all workspaces for root/system_admin).
/// For api_keys: a global key mirrors its creator's reach (so it does NOT need
/// per-workspace grants); a non-global key lists the workspaces where it holds at
/// least one grant.
pub(crate) async fn list_workspaces(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<WorkspaceDto>>, ApiError> {
    let workspaces = match principal {
        Principal::ApiKey(kid) => {
            let key = PgApiKeyRepo {
                conn: (*state.db).clone(),
            }
            .get_by_id(kid)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::Unauthorized)?;

            if key.is_global {
                reachable_workspaces_for_user(&state, key.created_by_user_id).await?
            } else {
                PgWorkspaceRepo {
                    conn: (*state.db).clone(),
                }
                .list_for_api_key(kid)
                .await
                .map_err(|e| ApiError::Internal {
                    message: e.to_string(),
                })?
            }
        }

        Principal::User(user_id) => {
            let user = PgUserRepo {
                conn: (*state.db).clone(),
            }
            .find_by_id(user_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?
            .ok_or(ApiError::Unauthorized)?;

            if user.disabled_at.is_some() {
                return Err(ApiError::Unauthorized);
            }

            reachable_workspaces_for_user(&state, user_id).await?
        }
    };

    Ok(Json(workspaces.iter().map(workspace_to_dto).collect()))
}

/// The workspaces a user can reach: every workspace for a root/system-admin,
/// otherwise their member workspaces. Returns an empty list for a missing or
/// disabled user, so a global agent whose creator is gone or disabled reaches
/// nothing — keeping `list_workspaces` consistent with the per-request authz.
async fn reachable_workspaces_for_user(
    state: &AppState,
    user_id: UserId,
) -> Result<Vec<Workspace>, ApiError> {
    let Some(user) = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .find_by_id(user_id)
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?
    else {
        return Ok(Vec::new());
    };

    if user.disabled_at.is_some() {
        return Ok(Vec::new());
    }

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    if user.is_root || user.is_system_admin {
        ws_repo.list_all().await.map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })
    } else {
        ws_repo
            .list_for_user(user_id)
            .await
            .map_err(|_| ApiError::Internal {
                message: "workspace lookup failed".into(),
            })
    }
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
///
/// A human member passes unchanged; an API-key principal is additionally
/// required to hold `config:update`, so a scoped agent cannot rename the
/// workspace without that capability. `member.api_key_id` is `Some` exactly for
/// those callers.
pub(crate) async fn update_workspace(
    member: WorkspaceMember,
    State(state): State<AppState>,
    Json(body): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceDto>, ApiError> {
    if let Some(key_id) = member.api_key_id {
        enforce_api_key_scope(
            &state.db,
            key_id,
            Capability {
                family: CapabilityFamily::Config,
                action: CapabilityAction::Update,
            },
        )
        .await?;
    }

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

#[utoipa::path(
    patch,
    path = "/v1/admin/workspaces/{ws}",
    tag = "workspaces",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = AdminUpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Workspace updated", body = WorkspaceDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
        (status = 404, description = "Workspace not found"),
        (status = 422, description = "Validation error or slug already in use"),
    )
)]
/// Updates a workspace's `name` and/or `slug` on behalf of a platform admin.
///
/// Unlike the member-facing `update_workspace`, this admin path may re-slug a
/// workspace. The new slug is format-validated and checked for collisions against
/// every existing slug (a soft-deleted workspace still reserves its slug). Both
/// fields are optional; omitting both is a no-op that returns the current state.
/// Restricted to root/system-admin via `RequireUserAdmin`.
pub(crate) async fn admin_update_workspace(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Path(ws): Path<String>,
    Json(body): Json<AdminUpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceDto>, ApiError> {
    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    let workspace = ws_repo
        .find_by_slug(&ws)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let mut current = workspace;

    if let Some(name) = body.name {
        validate_name("name", &name)?;

        current = ws_repo
            .rename(current.id, name)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;
    }

    if let Some(slug) = body.slug
        && slug != current.slug
    {
        validate_slug("slug", &slug)?;

        let existing_slugs = ws_repo.list_slugs().await.map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;
        if existing_slugs.iter().any(|s| s == &slug) {
            return Err(ApiError::InvalidInput {
                message: format!("slug '{slug}' is already in use"),
            });
        }

        current = ws_repo
            .set_slug(current.id, slug)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?;
    }

    Ok(Json(workspace_to_dto(&current)))
}

#[utoipa::path(
    delete,
    path = "/v1/admin/workspaces/{ws}",
    tag = "workspaces",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 204, description = "Workspace deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Not a root/admin user"),
        (status = 404, description = "Workspace not found"),
    )
)]
/// Soft-deletes a workspace, hiding it and its content from every lookup while
/// preserving the underlying rows. Restricted to root/system-admin via
/// `RequireUserAdmin`. Deleting an unknown or already-deleted workspace is a 404.
pub(crate) async fn admin_delete_workspace(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
    Path(ws): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    let workspace = ws_repo
        .find_by_slug(&ws)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    ws_repo
        .soft_delete(workspace.id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(StatusCode::NO_CONTENT)
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
