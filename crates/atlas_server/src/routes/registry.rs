/// Classification of a route's access control requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    /// No authentication required.
    Public,
    /// Any authenticated principal (user or api key).
    AuthnRequired,
    /// Any authenticated member of the workspace in the `{ws}` path segment.
    WorkspaceMember,
}

/// A single route entry — the shared registry that links the router, the protection
/// audit, and the OpenAPI drift test.
///
/// Every route that appears in `lib.rs` MUST have a corresponding entry here.
/// Every entry here MUST be routed in `lib.rs`. The `route_covers_all_registry_entries`
/// test enforces the second direction at runtime.
pub struct RouteEntry {
    pub method: &'static str,
    /// Path template using `{param}` placeholders. Workspace-scoped paths use `{ws}`.
    /// For the 401 sweep a concrete path is needed; use a sentinel value for params
    /// other than `{ws}` (e.g. `00000000-0000-0000-0000-000000000001` for uuid params,
    /// `nonexistent-slug` for slug params).
    pub path_template: &'static str,
    pub kind: RouteKind,
    /// Whether this route must appear in the OpenAPI document.
    pub openapi_path: Option<&'static str>,
}

pub static ROUTE_REGISTRY: &[RouteEntry] = &[
    // ---- Public routes ----
    RouteEntry {
        method: "GET",
        path_template: "/health",
        kind: RouteKind::Public,
        openapi_path: Some("/health"),
    },
    RouteEntry {
        method: "GET",
        path_template: "/version",
        kind: RouteKind::Public,
        openapi_path: Some("/version"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/auth/login",
        kind: RouteKind::Public,
        openapi_path: Some("/v1/auth/login"),
    },
    // ---- Auth (any authenticated principal) ----
    RouteEntry {
        method: "POST",
        path_template: "/v1/auth/logout",
        kind: RouteKind::AuthnRequired,
        openapi_path: Some("/v1/auth/logout"),
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/auth/me",
        kind: RouteKind::AuthnRequired,
        openapi_path: Some("/v1/auth/me"),
    },
    // ---- User management (root-only, but still require authn) ----
    RouteEntry {
        method: "POST",
        path_template: "/v1/users",
        kind: RouteKind::AuthnRequired,
        openapi_path: Some("/v1/users"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/users/00000000-0000-0000-0000-000000000001/disable",
        kind: RouteKind::AuthnRequired,
        openapi_path: Some("/v1/users/{user_id}/disable"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/users/00000000-0000-0000-0000-000000000001/enable",
        kind: RouteKind::AuthnRequired,
        openapi_path: Some("/v1/users/{user_id}/enable"),
    },
    // ---- Workspace-scoped routes ----
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/api-keys",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/api-keys"),
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/api-keys",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/api-keys"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/api-keys/00000000-0000-0000-0000-000000000001/revoke",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/api-keys/{key_id}/revoke"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/projects",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects"),
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/projects",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects"),
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects/{project_slug}"),
    },
    RouteEntry {
        method: "PATCH",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects/{project_slug}"),
    },
    RouteEntry {
        method: "DELETE",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects/{project_slug}"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj/grants",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects/{project_slug}/grants"),
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj/grants",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects/{project_slug}/grants"),
    },
    RouteEntry {
        method: "DELETE",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj/grants/00000000-0000-0000-0000-000000000001",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}"),
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/grants",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/grants"),
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/grants",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/grants"),
    },
    RouteEntry {
        method: "DELETE",
        path_template: "/v1/workspaces/{ws}/grants/00000000-0000-0000-0000-000000000001",
        kind: RouteKind::WorkspaceMember,
        openapi_path: Some("/v1/workspaces/{ws}/grants/{grant_id}"),
    },
];
