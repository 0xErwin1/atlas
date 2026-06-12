/// Kind of access control a route requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteKind {
    Public,
    AuthnRequired,
    WorkspaceMember,
}

pub(crate) struct RouteEntry {
    pub(crate) method: &'static str,
    pub(crate) path_template: &'static str,
    pub(crate) kind: RouteKind,
}

pub(crate) static ROUTE_MATRIX: &[RouteEntry] = &[
    // Public
    RouteEntry {
        method: "GET",
        path_template: "/health",
        kind: RouteKind::Public,
    },
    RouteEntry {
        method: "GET",
        path_template: "/version",
        kind: RouteKind::Public,
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/auth/login",
        kind: RouteKind::Public,
    },
    // Authentication-required (any valid session)
    RouteEntry {
        method: "POST",
        path_template: "/v1/auth/logout",
        kind: RouteKind::AuthnRequired,
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/auth/me",
        kind: RouteKind::AuthnRequired,
    },
    // Users (root-only, but still require authn)
    RouteEntry {
        method: "POST",
        path_template: "/v1/users",
        kind: RouteKind::AuthnRequired,
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/users/00000000-0000-0000-0000-000000000001/disable",
        kind: RouteKind::AuthnRequired,
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/users/00000000-0000-0000-0000-000000000001/enable",
        kind: RouteKind::AuthnRequired,
    },
    // Workspace-scoped routes
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/api-keys",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/api-keys",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/projects",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/projects",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "PATCH",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "DELETE",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj/grants",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj/grants",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "DELETE",
        path_template: "/v1/workspaces/{ws}/projects/nonexistent-proj/grants/00000000-0000-0000-0000-000000000001",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "POST",
        path_template: "/v1/workspaces/{ws}/grants",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/grants",
        kind: RouteKind::WorkspaceMember,
    },
    RouteEntry {
        method: "DELETE",
        path_template: "/v1/workspaces/{ws}/grants/00000000-0000-0000-0000-000000000001",
        kind: RouteKind::WorkspaceMember,
    },
];
