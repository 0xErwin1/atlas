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
    RouteEntry {
        method: "GET",
        path_template: "/v1/workspaces/{ws}/probe",
        kind: RouteKind::WorkspaceMember,
    },
];
