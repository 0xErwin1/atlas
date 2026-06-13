use axum::{Json, response::IntoResponse};
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable as _};

use atlas_api::{
    dtos::{
        ApiKeyCreated, ApiKeyDto, CreateApiKeyRequest, CreateGrantRequest, CreateProjectRequest,
        CreateUserRequest, GrantDto, GrantPrincipal, LoginRequest, LoginResponse, MeResponse,
        ProjectDto, UpdateProjectRequest, UserDto, WorkspaceDto,
        documents::{
            ActorDto, AttachmentDto, BacklinkDto, ConflictProblemDto, CreateDocumentRequest,
            DocumentDto, DocumentSummaryDto, FrontmatterDto, MoveDocumentRequest,
            RevisionContentDto, RevisionMetaDto, UpdateContentRequest, UpdateDocumentRequest,
        },
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
    paths(
        crate::routes::health::health,
        crate::routes::health::version,
        crate::routes::auth::login,
        crate::routes::auth::logout,
        crate::routes::auth::me,
        crate::routes::users::create_user,
        crate::routes::users::disable_user,
        crate::routes::users::enable_user,
        crate::routes::workspaces::get_workspace,
        crate::routes::api_keys::create_api_key,
        crate::routes::api_keys::list_api_keys,
        crate::routes::api_keys::revoke_api_key,
        crate::routes::projects::create_project,
        crate::routes::projects::list_projects,
        crate::routes::projects::get_project,
        crate::routes::projects::update_project,
        crate::routes::projects::delete_project,
        crate::routes::grants::create_project_grant,
        crate::routes::grants::list_project_grants,
        crate::routes::grants::delete_project_grant,
        crate::routes::grants::create_workspace_grant,
        crate::routes::grants::list_workspace_grants,
        crate::routes::grants::delete_workspace_grant,
        crate::routes::documents::create_document,
        crate::routes::documents::list_documents,
        crate::routes::documents::get_document,
        crate::routes::documents::update_document,
        crate::routes::documents::delete_document,
        crate::routes::documents::update_content,
        crate::routes::documents::list_history,
        crate::routes::documents::list_backlinks,
        crate::routes::documents::get_frontmatter,
        crate::routes::documents::upload_attachment,
        crate::routes::documents::list_attachments,
        crate::routes::documents::get_revision_content,
        crate::routes::documents::download_attachment,
        crate::routes::documents::delete_attachment,
        crate::routes::documents::move_document,
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
        CreateDocumentRequest,
        UpdateDocumentRequest,
        UpdateContentRequest,
        MoveDocumentRequest,
        DocumentDto,
        DocumentSummaryDto,
        RevisionMetaDto,
        RevisionContentDto,
        BacklinkDto,
        FrontmatterDto,
        AttachmentDto,
        ActorDto,
        ConflictProblemDto,
    )),
    tags(
        (name = "auth", description = "Authentication and session management"),
        (name = "users", description = "User management (root-only)"),
        (name = "api-keys", description = "Workspace API key management"),
        (name = "projects", description = "Project CRUD"),
        (name = "grants", description = "Permission grant management"),
        (name = "workspaces", description = "Workspace metadata"),
        (name = "documents", description = "Document management"),
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
