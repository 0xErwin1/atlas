use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use atlas_api::dtos::tags::{CreateTagRequest, TagDto};
use atlas_domain::{
    Actor, WorkspaceCtx, entities::tags::NewTag, entities::tags::Tag, permissions::Principal,
};

use crate::{
    authz::{Authorized, EditorMin, ViewerMin, authorized::WorkspaceRes},
    error::ApiError,
    persistence::repos::{PgTagRepo, TagRepo},
    routes::validation::validate_name,
    state::AppState,
};

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
    }
}

fn tag_to_dto(t: Tag) -> TagDto {
    TagDto {
        id: t.id.0,
        workspace_id: t.workspace_id.0,
        name: t.name,
        created_at: t.created_at,
        updated_at: t.updated_at,
    }
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tags
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tags",
    tag = "tags",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 200, description = "Workspace tags sorted by name", body = [TagDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_tags(
    auth: Authorized<WorkspaceRes, ViewerMin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TagDto>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTagRepo::new((*state.db).clone());

    let tags = repo.list(&ctx).await.map_err(ApiError::Domain)?;

    Ok(Json(tags.into_iter().map(tag_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/tags
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/tags",
    tag = "tags",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreateTagRequest,
    responses(
        (status = 201, description = "Tag created", body = TagDto),
        (status = 200, description = "Existing tag returned (idempotent)", body = TagDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn create_tag(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreateTagRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTagRepo::new((*state.db).clone());

    let name = body.name.trim();

    if let Some(existing) = repo
        .find_by_name(&ctx, name)
        .await
        .map_err(ApiError::Domain)?
    {
        return Ok((StatusCode::OK, Json(tag_to_dto(existing))));
    }

    let tag = repo
        .create(
            &ctx,
            NewTag {
                name: name.to_string(),
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(tag_to_dto(tag))))
}
