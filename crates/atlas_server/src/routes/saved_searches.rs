use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::saved_searches::{
    CreateSavedSearchRequest, RenameSavedSearchRequest, SavedSearchDto,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::saved_searches::{NewSavedSearch, SavedSearch},
};

use crate::{
    authz::WorkspaceMember,
    error::ApiError,
    persistence::repos::{PgSavedSearchRepo, SavedSearchRepo},
    routes::validation::{validate_name, validate_query},
    state::AppState,
};

fn actor_from_member(member: &WorkspaceMember) -> Result<Actor, ApiError> {
    match (&member.user, &member.api_key_id) {
        (Some(user), _) => Ok(Actor::User(user.id)),
        (None, Some(key_id)) => Ok(Actor::ApiKey(*key_id)),
        (None, None) => Err(ApiError::Unauthorized),
    }
}

fn saved_search_to_dto(ss: SavedSearch) -> SavedSearchDto {
    SavedSearchDto {
        id: ss.id.0,
        workspace_id: ss.workspace_id.0,
        name: ss.name,
        query: ss.query,
        created_at: ss.created_at,
        updated_at: ss.updated_at,
    }
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/saved-searches
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/saved-searches",
    tag = "saved-searches",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 200, description = "Caller's saved searches sorted by name (case-insensitive)", body = [SavedSearchDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
    )
)]
pub(crate) async fn list_saved_searches(
    member: WorkspaceMember,
    State(state): State<AppState>,
) -> Result<Json<Vec<SavedSearchDto>>, ApiError> {
    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgSavedSearchRepo::new((*state.db).clone());

    let searches = repo.list_for_owner(&ctx).await.map_err(ApiError::Domain)?;

    Ok(Json(
        searches.into_iter().map(saved_search_to_dto).collect(),
    ))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/saved-searches
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/saved-searches",
    tag = "saved-searches",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateSavedSearchRequest,
    responses(
        (status = 201, description = "Saved search created", body = SavedSearchDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Workspace not found or caller is not a member"),
        (status = 409, description = "A saved search with this name already exists for this owner"),
        (status = 422, description = "Validation error or per-owner cap exceeded"),
    )
)]
pub(crate) async fn create_saved_search(
    member: WorkspaceMember,
    State(state): State<AppState>,
    Json(body): Json<CreateSavedSearchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;
    validate_query(&body.query)?;

    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgSavedSearchRepo::new((*state.db).clone());

    let name = body.name.trim().to_string();

    let ss = repo
        .create(
            &ctx,
            NewSavedSearch {
                name,
                query: body.query,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(saved_search_to_dto(ss))))
}

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/saved-searches/{id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/saved-searches/{id}",
    tag = "saved-searches",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("id" = uuid::Uuid, Path, description = "Saved search id"),
    ),
    request_body = RenameSavedSearchRequest,
    responses(
        (status = 200, description = "Saved search renamed", body = SavedSearchDto),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Saved search not found, not owned by caller, or caller is not a member"),
        (status = 409, description = "A saved search with this name already exists for this owner"),
        (status = 422, description = "Validation error"),
    )
)]
pub(crate) async fn rename_saved_search(
    member: WorkspaceMember,
    Path((_ws, id)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
    Json(body): Json<RenameSavedSearchRequest>,
) -> Result<Json<SavedSearchDto>, ApiError> {
    validate_name("name", &body.name)?;

    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgSavedSearchRepo::new((*state.db).clone());

    let new_name = body.name.trim().to_string();

    let ss = repo
        .rename(&ctx, id.into(), new_name)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(saved_search_to_dto(ss)))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/saved-searches/{id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/saved-searches/{id}",
    tag = "saved-searches",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("id" = uuid::Uuid, Path, description = "Saved search id"),
    ),
    responses(
        (status = 204, description = "Saved search deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 404, description = "Saved search not found, not owned by caller, or caller is not a member"),
    )
)]
pub(crate) async fn delete_saved_search(
    member: WorkspaceMember,
    Path((_ws, id)): Path<(String, uuid::Uuid)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let actor = actor_from_member(&member)?;
    let ctx = WorkspaceCtx::new(member.workspace.id, actor);
    let repo = PgSavedSearchRepo::new((*state.db).clone());

    repo.delete(&ctx, id.into())
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
