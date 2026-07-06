use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use atlas_api::dtos::tags::{CreateTagRequest, TagDto, UpdateTagRequest};
use atlas_domain::{
    Actor, WorkspaceCtx, entities::tags::NewTag, entities::tags::Tag, ids::TagId,
    permissions::Principal,
};

use crate::{
    authz::{
        Authorized, ConfigCreate, ConfigDelete, ConfigRead, ConfigUpdate, EditorMin, ViewerMin,
        authorized::WorkspaceRes,
    },
    error::ApiError,
    persistence::repos::{PgTagRepo, TagRepo},
    routes::validation::{validate_name, validate_swatch},
    state::AppState,
};

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => Actor::User(atlas_domain::ids::UserId(uuid::Uuid::nil())),
    }
}

fn tag_to_dto(t: Tag) -> TagDto {
    TagDto {
        id: t.id.0,
        workspace_id: t.workspace_id.0,
        name: t.name,
        color: t.color,
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
    auth: Authorized<WorkspaceRes, ViewerMin, ConfigRead>,
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
    auth: Authorized<WorkspaceRes, EditorMin, ConfigCreate>,
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

// ---------------------------------------------------------------------------
// PATCH /v1/workspaces/{ws}/tags/{tag_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/v1/workspaces/{ws}/tags/{tag_id}",
    tag = "tags",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("tag_id" = uuid::Uuid, Path, description = "Tag ID"),
    ),
    request_body = UpdateTagRequest,
    responses(
        (status = 200, description = "Tag updated", body = TagDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Tag not found or not in workspace"),
        (status = 409, description = "Name already taken by another tag"),
        (status = 422, description = "Invalid input (blank name or unknown color swatch)"),
    )
)]
pub(crate) async fn patch_tag(
    auth: Authorized<WorkspaceRes, EditorMin, ConfigUpdate>,
    State(state): State<AppState>,
    Path((_ws, tag_id)): Path<(String, uuid::Uuid)>,
    Json(body): Json<UpdateTagRequest>,
) -> Result<Json<TagDto>, ApiError> {
    if let Some(ref name) = body.name {
        validate_name("name", name)?;
    }

    if let Some(ref color) = body.color {
        validate_swatch("color", color)?;
    }

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTagRepo::new((*state.db).clone());

    let tag = repo
        .update(&ctx, TagId(tag_id), body.name, body.color)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(tag_to_dto(tag)))
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/tags/used
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/tags/used",
    tag = "tags",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    responses(
        (status = 200, description = "Distinct label strings used by non-deleted tasks", body = [String]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
    )
)]
pub(crate) async fn list_used_labels(
    auth: Authorized<WorkspaceRes, ViewerMin, ConfigRead>,
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTagRepo::new((*state.db).clone());

    let labels = repo
        .list_used_labels(&ctx)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(labels))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/tags/{tag_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/tags/{tag_id}",
    tag = "tags",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("tag_id" = uuid::Uuid, Path, description = "Tag ID"),
    ),
    responses(
        (status = 204, description = "Tag soft-deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Tag not found or not in workspace"),
    )
)]
pub(crate) async fn delete_tag(
    auth: Authorized<WorkspaceRes, EditorMin, ConfigDelete>,
    State(state): State<AppState>,
    Path((_ws, tag_id)): Path<(String, uuid::Uuid)>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgTagRepo::new((*state.db).clone());

    repo.soft_delete(&ctx, TagId(tag_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
