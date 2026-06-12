use axum::{Json, response::IntoResponse};
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable as _};

use atlas_api::{
    dtos::{
        ApiKeyCreated, ApiKeyDto, CreateApiKeyRequest, CreateGrantRequest, CreateProjectRequest,
        CreateUserRequest, GrantDto, GrantPrincipal, LoginRequest, LoginResponse, MeResponse,
        ProjectDto, UpdateProjectRequest, UserDto, WorkspaceDto,
    },
    problem::ProblemDetails,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Atlas API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Atlas knowledge and project-management platform REST API"
    ),
    components(schemas(
        LoginRequest,
        LoginResponse,
        MeResponse,
        CreateUserRequest,
        UserDto,
        CreateApiKeyRequest,
        ApiKeyCreated,
        ApiKeyDto,
        CreateProjectRequest,
        UpdateProjectRequest,
        ProjectDto,
        CreateGrantRequest,
        GrantPrincipal,
        GrantDto,
        WorkspaceDto,
        ProblemDetails,
    )),
    tags(
        (name = "auth", description = "Authentication and session management"),
        (name = "users", description = "User management (root-only)"),
        (name = "api-keys", description = "Workspace API key management"),
        (name = "projects", description = "Project CRUD"),
        (name = "grants", description = "Permission grant management"),
        (name = "workspaces", description = "Workspace metadata"),
    )
)]
pub(crate) struct ApiDoc;

pub(crate) async fn openapi_json() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

pub(crate) fn scalar_router<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    axum::Router::from(Scalar::with_url("/scalar", ApiDoc::openapi()))
}

/// Expose the assembled `OpenApi` document for test assertions.
pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
