#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_server::routes::openapi::openapi;

/// All schema component names that must be present in the generated OpenAPI document.
///
/// If a new DTO is added to the `ApiDoc` components list, it must appear here too —
/// and vice versa. This test is the guard against silent drift.
const EXPECTED_SCHEMAS: &[&str] = &[
    "LoginRequest",
    "LoginResponse",
    "MeResponse",
    "CreateUserRequest",
    "UserDto",
    "CreateApiKeyRequest",
    "ApiKeyCreated",
    "ApiKeyDto",
    "CreateProjectRequest",
    "UpdateProjectRequest",
    "ProjectDto",
    "CreateGrantRequest",
    "GrantPrincipal",
    "GrantDto",
    "WorkspaceDto",
    "ProblemDetails",
];

#[test]
fn openapi_document_contains_required_schemas() {
    let doc = openapi();

    let components = doc
        .components
        .as_ref()
        .expect("OpenAPI document must have a components section");

    let schemas = &components.schemas;

    for name in EXPECTED_SCHEMAS {
        assert!(
            schemas.contains_key(*name),
            "expected schema '{name}' is missing from OpenAPI components"
        );
    }

    assert_eq!(
        schemas.len(),
        EXPECTED_SCHEMAS.len(),
        "OpenAPI component schema count mismatch: expected {}, got {}. \
         Update EXPECTED_SCHEMAS in openapi_drift.rs when adding or removing DTOs.",
        EXPECTED_SCHEMAS.len(),
        schemas.len()
    );
}

/// Every shipped route path must appear in the OpenAPI document.
///
/// The path strings mirror the axum route definitions in lib.rs. When a route is added
/// or removed, this list must be updated — and if the OpenAPI `#[utoipa::path]`
/// annotation is missing, the test fails.
const EXPECTED_PATHS: &[&str] = &[
    "/health",
    "/version",
    "/v1/auth/login",
    "/v1/auth/logout",
    "/v1/auth/me",
    "/v1/users",
    "/v1/users/{user_id}/disable",
    "/v1/users/{user_id}/enable",
    "/v1/workspaces/{ws}",
    "/v1/workspaces/{ws}/api-keys",
    "/v1/workspaces/{ws}/api-keys/{key_id}/revoke",
    "/v1/workspaces/{ws}/projects",
    "/v1/workspaces/{ws}/projects/{project_slug}",
    "/v1/workspaces/{ws}/projects/{project_slug}/grants",
    "/v1/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}",
    "/v1/workspaces/{ws}/grants",
    "/v1/workspaces/{ws}/grants/{grant_id}",
];

#[test]
fn openapi_document_paths_match_router() {
    let doc = openapi();
    let doc_paths = &doc.paths.paths;

    for path in EXPECTED_PATHS {
        assert!(
            doc_paths.contains_key(*path),
            "route '{path}' is missing from the OpenAPI paths; add a #[utoipa::path] annotation \
             and register it in ApiDoc paths()"
        );
    }

    assert_eq!(
        doc_paths.len(),
        EXPECTED_PATHS.len(),
        "OpenAPI path count mismatch: expected {}, got {}. \
         Update EXPECTED_PATHS in openapi_drift.rs when adding or removing routes.",
        EXPECTED_PATHS.len(),
        doc_paths.len()
    );
}

#[test]
fn openapi_document_has_correct_info() {
    let doc = openapi();

    assert_eq!(doc.info.title, "Atlas API");
    assert!(!doc.info.version.is_empty(), "version must not be empty");
}
