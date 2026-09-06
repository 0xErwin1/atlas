#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `v2-e11-s5` PR6a (design D7, D7.2, D7.4 direction 1 only; PR6b adds
//! directions 2 and 3 to this same file).
//!
//! Composes three derivations, declaring nothing by hand (spec requirement
//! "A coverage test compares the composed OpenAPI document against CLI and
//! MCP"; SHELL-REG-4, design D7):
//!
//! 1. `support::route_matrix::route_matrix()` — the 217 live REG-5 routes,
//!    each carrying its own `(component, method, path_template)`.
//! 2. `support::client_routes::client_routes()` — `AtlasClient` method name
//!    ⇒ `(component, method, path_template)`, cross-checked by cardinality
//!    against the route contract's pinned extracted-call count (D7.4).
//! 3. This file's own CLI and MCP walks — command/operation ⇒ set of
//!    `AtlasClient` method names — built on the same
//!    `support::source_walk` helpers `cli_mcp_component_derivation.rs`
//!    (PR1/PR5) carries privately until PR6b, extended to resolve the
//!    *method name* following `.acta()/.custos()/.platform()` rather than
//!    only the namespace token. Both walks are pinned to PR1/PR5's own
//!    cardinalities (28 commands, 112 operations) before any coverage
//!    assertion, so a dropped command or operation fails loudly instead of
//!    silently shrinking coverage.
//!
//! **Per-surface, never a union (D7.2).** A single union exclusion list
//! would be vacuous: MCP covers what CLI does not (users/grants/api-keys)
//! and CLI covers what MCP does not (comments/attachments/webhooks), so
//! "uncovered by both" would be nearly empty. `CLI_UNCOVERED` and
//! `MCP_UNCOVERED` are two separate, closed, route-keyed lists.
//!
//! **Three failure directions (design D7.3), all live.** Direction 1
//! (unnamed gap): a registry route reachable through neither surface and
//! absent from that surface's exclusion list. Direction 2 (stale
//! exclusion): a listed route the surface now covers. Direction 3
//! (exclusion creep, epic R5): a `Category` used on a surface it is not
//! valid for, or a `Category` with zero rows on every surface (dead).
//! `Category`'s own closure — no value outside the enum — is a compile-time
//! property: [`Category::valid_for_surface`]'s match has no wildcard arm, so
//! adding a thirteenth variant without updating it fails to compile, in a
//! diff a reviewer sees, exactly as design D7.3 asks.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use atlas_core::registry::HttpMethod;
use regex::Regex;

use support::client_routes::{ClientRoute, client_routes};
use support::route_matrix::{RouteMatrixEntry, route_matrix};
use support::scan::scan;
use support::source_walk::{
    McpFn, extract_array_body, find_fn_body, find_verb_match_body, module_source_files,
    normalize_template, parse_impl_fn_boundaries, parse_resource_handlers, read_production_source,
    repo_root,
};

// ---------------------------------------------------------------------------
// Category (design D7.2, D7.3 direction 3) — closed, per-surface.
// ---------------------------------------------------------------------------

/// Declares `Category` and `Category::ALL` from one variant list, so a
/// variant cannot exist without appearing in `ALL` — [`dead_category_failures`]
/// is exhaustive by construction rather than by a hand-maintained constant
/// (design D7.3 direction 3).
macro_rules! closed_category {
    ($($(#[$attr:meta])* $variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Category {
            $($(#[$attr])* $variant,)+
        }

        impl Category {
            /// Number of closed-enum variants.
            const COUNT: usize = [$(Category::$variant),+].len();

            /// Every closed-enum variant, once — [`dead_category_failures`]'s
            /// universe. Generated from the same list as the enum itself.
            const ALL: &'static [Category] = &[$(Category::$variant),+];
        }
    };
}

closed_category! {
    // CLI-only: MCP covers this domain instead.
    Comments,
    Webhooks,
    /// Self-service session/activation endpoints — the CLI's `users` command
    /// manages accounts administratively, not through a browser session.
    /// Measured at apply time (not named by the spec's three CLI
    /// categories); see PR6a's apply-progress for the full accounting.
    Auth,
    // MCP-only: the CLI covers this domain instead.
    Users,
    Grants,
    ApiKeys,
    // Shared: reachable through neither surface, by design. Also measured
    // at apply time — the spec named only the six categories above; the
    // real per-surface gap is far larger than "everything except
    // comments/attachments/webhooks/users/grants/api-keys" (see PR6a's
    // apply-progress).
    /// A generic workspace/task/document attachment CRUD route — measured to
    /// be missing from *both* surfaces (the CLI has no attachments feature
    /// at all; the MCP `attachment` verb covers comment/task-comment
    /// attachments but not these plain workspace/task/document ones), so
    /// this is shared rather than CLI-only despite the spec naming it as a
    /// CLI-only gap.
    Attachments,
    RootProbe,
    /// Live presence and comment/task draft endpoints — a web-app-only
    /// real-time feature, not a command or catalog operation on either
    /// surface.
    RealTimeCollaboration,
    /// Integration-config, automation-rule, semantic-search, and
    /// event-stream endpoints — workspace-admin web features, not a command
    /// or catalog operation on either surface.
    IntegrationsAndAutomation,
    /// Root/system-admin workspace and trash endpoints — not a command or
    /// catalog operation on either surface.
    AdminOperations,
    /// A specialized single-resource mutation or fetch beyond a surface's
    /// basic list/get/create coverage (moves, copies, archive/unarchive,
    /// column/member/property-definition/task-view management, document
    /// content range/search/compact/graph) — not a command or catalog
    /// operation on either surface.
    SpecializedMutation,
}

impl Category {
    /// The closed set of categories a given surface's exclusion list may
    /// carry (design D7.3 direction 3: a category on the wrong surface is
    /// exclusion creep, not a valid entry). No wildcard arm: a thirteenth
    /// `Category` variant fails to compile here until this match names it.
    fn valid_for_surface(self, surface: Surface) -> bool {
        match self {
            Category::Comments | Category::Webhooks | Category::Auth => surface == Surface::Cli,
            Category::Users | Category::Grants | Category::ApiKeys => surface == Surface::Mcp,
            Category::Attachments
            | Category::RootProbe
            | Category::RealTimeCollaboration
            | Category::IntegrationsAndAutomation
            | Category::AdminOperations
            | Category::SpecializedMutation => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Cli,
    Mcp,
}

impl Surface {
    fn as_str(self) -> &'static str {
        match self {
            Surface::Cli => "CLI",
            Surface::Mcp => "MCP",
        }
    }
}

/// One exclusion-list entry: the route it names, its category, and a reason
/// (design D7.2 — a reason that only repeats the path is not a
/// justification, PR6a review posture; entries in the same category share a
/// reason describing that category's why, exactly as
/// `atlas_client_route_contract.rs::UNCOVERED_ROUTES` already reuses a
/// reason across several entries).
type UncoveredRoute = (
    &'static str,
    HttpMethod,
    &'static str,
    Category,
    &'static str,
);

// ---------------------------------------------------------------------------
// T6a.4 — measured at apply time (T6a.3's RED run, empty lists, named every
// real gap). The spec's own three-per-surface categories
// (comments/attachments/webhooks; users/grants/api-keys) turned out to cover
// only a fraction of the real gap: the CLI and the MCP catalog are each
// materially thinner than the full 217-route registry — most resources
// expose only list/get/create through either surface, with moves, copies,
// archive/unarchive, presence, drafts, integrations/automation, admin
// operations, and most custos self-service/lifecycle endpoints reachable
// through neither. See PR6a's apply-progress for the full accounting and
// the five additional categories this required.
// ---------------------------------------------------------------------------

const CLI_UNCOVERED: &[UncoveredRoute] = &[
    // -- AdminOperations
    (
        "acta",
        HttpMethod::Get,
        "/admin/workspaces",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/admin/workspaces/{ws}",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/admin/workspaces/{ws}",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    // -- Attachments
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    // -- Auth
    (
        "custos",
        HttpMethod::Get,
        "/activate/{token}",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/activate/{token}",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/auth/change-password",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/auth/login",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/auth/logout",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/auth/me",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    (
        "custos",
        HttpMethod::Patch,
        "/users/me",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users/{user_id}/system-admin",
        Category::Auth,
        "self-service session/activation endpoint; the CLI's `users` command manages accounts administratively, not through the browser-session auth flow",
    ),
    // -- Comments
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/comments",
        Category::Comments,
        "comments feature; MCP covers document/task comments directly, the CLI has no comments command",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comments",
        Category::Comments,
        "comments feature; MCP covers document/task comments directly, the CLI has no comments command",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
        Category::Comments,
        "comments feature; MCP covers document/task comments directly, the CLI has no comments command",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
        Category::Comments,
        "comments feature; MCP covers document/task comments directly, the CLI has no comments command",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}",
        Category::Comments,
        "comments feature; MCP covers document/task comments directly, the CLI has no comments command",
    ),
    // -- IntegrationsAndAutomation
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/automation-rules",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/events",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/integration-configs",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/integration-configs/{config_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs/{config_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/integration-configs/{config_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/integrations/{integration}/events",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/semantic-search",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/semantic-search/reindex",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/semantic-search/reindex",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    // -- RealTimeCollaboration
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comment-drafts",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    // -- RootProbe
    (
        "acta",
        HttpMethod::Get,
        "/health",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/ready",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/health",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/ready",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/health",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/me/ui-state",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Put,
        "/me/ui-state",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/meta",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/openapi.json",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/ready",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/scalar",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/version",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    // -- SpecializedMutation
    (
        "acta",
        HttpMethod::Post,
        "/workspaces",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/assignable-users",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/boards/{board_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/boards/{board_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/archive",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/boards/{board_id}/move",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/unarchive",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/moves/batch",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/compact",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/content/range",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/documents/{slug}/content/range",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/content/search",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/copy",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/documents/{slug}/move",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/folders/{folder_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/folders/{folder_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/folders/{folder_id}/copy",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/folders/{folder_id}/move",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/members",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/members/{user_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/members/{user_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/projects",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/projects/{project_slug}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/projects/{project_slug}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tags",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/tags/used",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/tags/{tag_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/tags/{tag_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/graph",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/references/batch",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    // -- Webhooks
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/webhooks",
        Category::Webhooks,
        "webhook feature; MCP covers webhooks directly, the CLI has no webhooks command",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/webhooks",
        Category::Webhooks,
        "webhook feature; MCP covers webhooks directly, the CLI has no webhooks command",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/webhooks/{webhook_id}",
        Category::Webhooks,
        "webhook feature; MCP covers webhooks directly, the CLI has no webhooks command",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/webhooks/{webhook_id}",
        Category::Webhooks,
        "webhook feature; MCP covers webhooks directly, the CLI has no webhooks command",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/webhooks/{webhook_id}",
        Category::Webhooks,
        "webhook feature; MCP covers webhooks directly, the CLI has no webhooks command",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/webhooks/{webhook_id}/deliveries",
        Category::Webhooks,
        "webhook feature; MCP covers webhooks directly, the CLI has no webhooks command",
    ),
];

const MCP_UNCOVERED: &[UncoveredRoute] = &[
    // -- AdminOperations
    (
        "acta",
        HttpMethod::Get,
        "/admin/trash",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/admin/trash/purge",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/admin/trash/purges/{operation_id}",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/admin/trash/restore",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/admin/workspaces",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/admin/workspaces/{ws}",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/admin/workspaces/{ws}",
        Category::AdminOperations,
        "root/system-admin workspace or trash operation; not exposed as a command or catalog operation",
    ),
    // -- ApiKeys
    (
        "custos",
        HttpMethod::Get,
        "/api-keys",
        Category::ApiKeys,
        "API key lifecycle management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/api-keys",
        Category::ApiKeys,
        "API key lifecycle management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Delete,
        "/api-keys/{key_id}",
        Category::ApiKeys,
        "API key lifecycle management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Patch,
        "/api-keys/{key_id}",
        Category::ApiKeys,
        "API key lifecycle management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/api-keys/{key_id}/grants",
        Category::ApiKeys,
        "API key lifecycle management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Delete,
        "/api-keys/{key_id}/grants/{grant_id}",
        Category::ApiKeys,
        "API key lifecycle management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    // -- Attachments
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/attachments",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
        Category::Attachments,
        "generic attachment route; MCP's `attachment` verb covers comment/task-comment attachments but not this one, and the CLI has no attachments command at all",
    ),
    // -- Grants
    (
        "custos",
        HttpMethod::Get,
        "/workspaces/{ws}/grants",
        Category::Grants,
        "workspace/project grant management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/workspaces/{ws}/grants",
        Category::Grants,
        "workspace/project grant management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Delete,
        "/workspaces/{ws}/grants/{grant_id}",
        Category::Grants,
        "workspace/project grant management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/workspaces/{ws}/projects/{project_slug}/grants",
        Category::Grants,
        "workspace/project grant management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/workspaces/{ws}/projects/{project_slug}/grants",
        Category::Grants,
        "workspace/project grant management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    (
        "custos",
        HttpMethod::Delete,
        "/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}",
        Category::Grants,
        "workspace/project grant management; not exposed as an MCP operation (MCP's custos surface is identity/audit only)",
    ),
    // -- IntegrationsAndAutomation
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/automation-rules",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/events",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/integration-configs",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/integration-configs/{config_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs/{config_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/integration-configs/{config_id}",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/integrations/{integration}/events",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/semantic-search",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/semantic-search/reindex",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/semantic-search/reindex",
        Category::IntegrationsAndAutomation,
        "integration/automation/semantic-search/event-stream configuration, a workspace-admin web feature; not exposed as a command or catalog operation",
    ),
    // -- RealTimeCollaboration
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comment-drafts",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/presence",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
        Category::RealTimeCollaboration,
        "live presence or draft workflow, a web-app-only real-time feature; not exposed as a command or catalog operation",
    ),
    // -- RootProbe
    (
        "acta",
        HttpMethod::Get,
        "/health",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/ready",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/health",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/ready",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Post,
        "/doctor",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/health",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/me/ui-state",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Put,
        "/me/ui-state",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/meta",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/openapi.json",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/ready",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/scalar",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/version",
        Category::RootProbe,
        "infrastructure/introspection probe; neither a CLI command nor an MCP catalog operation exposes it",
    ),
    // -- SpecializedMutation
    (
        "acta",
        HttpMethod::Post,
        "/workspaces",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/assignable-users",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/boards/{board_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/apply-status-templates",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/archive",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/boards/{board_id}/move",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/boards/{board_id}/tasks",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/unarchive",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/frontmatter",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/folders/{folder_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/members",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/members/{user_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/members/{user_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/projects/{project_slug}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/property-definitions",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/property-definitions",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/property-definitions/{property_definition_id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/task-views/{id}",
        Category::SpecializedMutation,
        "specialized single-resource mutation or fetch beyond this surface's basic list/get/create coverage; not exposed as a command or catalog operation",
    ),
    // -- Users
    (
        "custos",
        HttpMethod::Get,
        "/activate/{token}",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/activate/{token}",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/auth/change-password",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/auth/login",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/auth/logout",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/users",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Patch,
        "/users/me",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users/{user_id}/activation-link",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users/{user_id}/disable",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users/{user_id}/enable",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/users/{user_id}/memberships",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users/{user_id}/reset-password",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users/{user_id}/system-admin",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/workspaces/{ws}/groups",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/workspaces/{ws}/groups",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Delete,
        "/workspaces/{ws}/groups/{group_id}",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/workspaces/{ws}/groups/{group_id}/members",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/workspaces/{ws}/groups/{group_id}/members",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
    (
        "custos",
        HttpMethod::Delete,
        "/workspaces/{ws}/groups/{group_id}/members/{user_id}",
        Category::Users,
        "custos identity/account-lifecycle endpoint beyond MCP's identity/audit surface (3 custos operations total); not exposed as an MCP operation",
    ),
];

// ---------------------------------------------------------------------------
// Source locations, shared shape with cli_mcp_component_derivation.rs.
// ---------------------------------------------------------------------------

fn cli_src_root() -> PathBuf {
    repo_root().join("crates/atlas_cli/src")
}

fn mcp_src_root() -> PathBuf {
    repo_root().join("crates/atlas_mcp/src")
}

/// Every `.acta()/.custos()/.platform()`-followed-by-`.method(` call in
/// already-masked `code`, resolved to just the method name — this walk does
/// not need the namespace itself (PR1/PR5 already audit that it is exactly
/// one and matches the declaration); it needs the method name to look the
/// call up in `client_routes()`.
fn resolve_client_methods_in_source(masked_code: &str) -> BTreeSet<String> {
    let re = Regex::new(r"\.\s*(?:acta|custos|platform)\s*\(\s*\)\s*\.\s*(\w+)\s*\(")
        .expect("valid regex");
    re.captures_iter(masked_code)
        .map(|caps| caps[1].to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// CLI walk: 28 dispatch arms -> module -> set of client method names.
// ---------------------------------------------------------------------------

/// `cli_mcp_component_derivation.rs::declared_components_table_has_28_rows`
/// (PR1), copied because the two test binaries cannot share a constant.
const CLI_COMMAND_COUNT: usize = 28;

struct DispatchArm {
    variant: String,
    module: Option<String>,
    body: String,
}

fn parse_dispatch_arms() -> Vec<DispatchArm> {
    let source = read_production_source(&cli_src_root().join("commands/mod.rs"));
    let masked = scan(&source).code;

    let arm_re = Regex::new(r"Commands::(\w+)\s*(?:\([^)]*\))?\s*=>").expect("valid regex");
    let markers: Vec<(usize, usize, String)> = arm_re
        .captures_iter(&masked)
        .map(|caps| {
            let whole = caps.get(0).expect("match 0 exists");
            (whole.start(), whole.end(), caps[1].to_string())
        })
        .collect();

    let module_re = Regex::new(r"\b([a-z_][a-z0-9_]*)::\w+\s*\(").expect("valid regex");

    markers
        .iter()
        .enumerate()
        .map(|(index, (_, end, variant))| {
            let body_end = markers.get(index + 1).map_or(masked.len(), |(s, _, _)| *s);
            let body = masked[*end..body_end].to_string();
            let module = module_re
                .captures(&body)
                .map(|caps| caps[1].to_string())
                .filter(|m| m != "Commands");
            DispatchArm {
                variant: variant.clone(),
                module,
                body,
            }
        })
        .collect()
}

fn resolve_module_client_methods(module: &str) -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    for path in module_source_files(&cli_src_root().join("commands"), module) {
        let source = read_production_source(&path);
        let masked = scan(&source).code;
        methods.extend(resolve_client_methods_in_source(&masked));
    }
    methods
}

/// Every declared CLI command's resolved `AtlasClient` method-name set,
/// keyed by its dispatch-arm variant name. `Version` (no module, inline
/// body, no call site) resolves to an empty set.
fn cli_command_methods() -> BTreeMap<String, BTreeSet<String>> {
    parse_dispatch_arms()
        .into_iter()
        .map(|arm| {
            let methods = match &arm.module {
                Some(module) => resolve_module_client_methods(module),
                None => resolve_client_methods_in_source(&arm.body),
            };
            (arm.variant, methods)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// MCP walk: 112 operations -> dispatcher handler -> set of client methods.
// ---------------------------------------------------------------------------

/// `cli_mcp_component_derivation.rs::operations_table_has_112_rows_split_108_3_1`
/// (PR5), copied because the two test binaries cannot share a constant.
const MCP_OPERATION_COUNT: usize = 112;

fn mcp_lib_source() -> String {
    read_production_source(&mcp_src_root().join("lib.rs"))
}

fn parse_operations() -> Vec<(String, String)> {
    let source = read_production_source(&mcp_src_root().join("catalog.rs"));
    let table = extract_array_body(&source, "OPERATIONS");

    let row_re =
        Regex::new(r#"verb:\s*"([a-z_]+)"[\s\S]*?resource:\s*"([a-z_]+)""#).expect("valid regex");

    row_re
        .captures_iter(&table)
        .map(|caps| (caps[1].to_string(), caps[2].to_string()))
        .collect()
}

fn parse_verbs() -> Vec<String> {
    let source = read_production_source(&mcp_src_root().join("catalog.rs"));
    let table = extract_array_body(&source, "VERBS");

    let row_re = Regex::new(r#""([a-z_]+)""#).expect("valid regex");
    row_re
        .captures_iter(&table)
        .map(|caps| caps[1].to_string())
        .collect()
}

fn resolve_handler_client_methods(fns: &[McpFn], source: &str, handler: &str) -> BTreeSet<String> {
    let Some(body) = find_fn_body(fns, source, handler) else {
        return BTreeSet::new();
    };
    let masked = scan(body).code;
    resolve_client_methods_in_source(&masked)
}

/// Every catalogued `(verb, resource)` operation's resolved `AtlasClient`
/// method-name set. `identity/ping` (D3.2's zero-call-site exception)
/// resolves to an empty set.
fn mcp_operation_methods() -> BTreeMap<(String, String), BTreeSet<String>> {
    let source = mcp_lib_source();
    let fns = parse_impl_fn_boundaries(&source);
    let verbs = parse_verbs();
    let operations = parse_operations();

    let mut dispatcher: BTreeMap<(String, String), String> = BTreeMap::new();
    for verb in &verbs {
        let body = find_verb_match_body(&fns, &source, verb);
        for arm in parse_resource_handlers(body) {
            dispatcher.insert((verb.clone(), arm.resource), arm.handler);
        }
    }

    operations
        .into_iter()
        .map(|key| {
            let methods = match dispatcher.get(&key) {
                Some(handler) => resolve_handler_client_methods(&fns, &source, handler),
                None => BTreeSet::new(),
            };
            (key, methods)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Composition: method names -> routes -> per-surface covered-route sets.
// ---------------------------------------------------------------------------

type RouteKey = (String, HttpMethod, String);

fn route_key(component: &str, method: HttpMethod, path: &str) -> RouteKey {
    (component.to_string(), method, normalize_template(path))
}

fn covered_routes(
    method_sets: impl Iterator<Item = BTreeSet<String>>,
    client_routes: &[ClientRoute],
) -> BTreeSet<RouteKey> {
    let all_methods: BTreeSet<String> = method_sets.flatten().collect();
    client_routes
        .iter()
        .filter(|route| all_methods.contains(&route.method_name))
        .map(|route| route_key(&route.component, route.http_method, &route.path_template))
        .collect()
}

fn exclusion_key(entry: &UncoveredRoute) -> RouteKey {
    let (component, method, path, _category, _reason) = *entry;
    route_key(component, method, path)
}

// ---------------------------------------------------------------------------
// Direction 1 (T6a.3/T6a.4) — an unlisted, uncovered registry route fails,
// naming the route and the surface.
// ---------------------------------------------------------------------------

fn unnamed_gap_failures(
    entries: &[RouteMatrixEntry],
    covered: &BTreeSet<RouteKey>,
    exclusions: &[UncoveredRoute],
    surface: Surface,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut seen: BTreeSet<RouteKey> = BTreeSet::new();

    for entry in entries {
        let key = route_key(&entry.component, entry.method, &entry.path_template);
        if !seen.insert(key.clone()) {
            continue;
        }

        if covered.contains(&key) {
            continue;
        }

        let listed = exclusions.iter().any(|entry| exclusion_key(entry) == key);
        if !listed {
            failures.push(format!(
                "{} {} ({}) is reachable through neither a command nor a resource on the {} \
                 surface, and is not listed in its exclusion list",
                entry.method,
                entry.path_template,
                entry.component,
                surface.as_str()
            ));
        }
    }

    failures
}

// ---------------------------------------------------------------------------
// Direction 2 (T6b.1/T6b.2) — a listed exclusion the surface now covers
// fails, naming the stale entry (design D7.3, INV-BIDIRECTIONAL-COVERAGE).
// ---------------------------------------------------------------------------

fn stale_exclusion_failures(
    covered: &BTreeSet<RouteKey>,
    exclusions: &[UncoveredRoute],
    surface: Surface,
) -> Vec<String> {
    exclusions
        .iter()
        .filter(|entry| covered.contains(&exclusion_key(entry)))
        .map(|entry| {
            let (component, method, path, _category, _reason) = *entry;
            format!(
                "{method} {path} ({component}) is listed in the {} exclusion list but is now \
                 reachable through a command or resource on that surface — stale entry",
                surface.as_str()
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Direction 3 (T6b.3/T6b.4, epic R5) — exclusion creep: a category used on
// the surface it is not valid for, or a category with zero rows on every
// surface (dead — nothing exercises it, so it cannot be reviewed for creep).
// ---------------------------------------------------------------------------

fn exclusion_creep_failures(exclusions: &[UncoveredRoute], surface: Surface) -> Vec<String> {
    exclusions
        .iter()
        .filter(|entry| {
            let (_, _, _, category, _) = *entry;
            !category.valid_for_surface(surface)
        })
        .map(|entry| {
            let (component, method, path, category, _reason) = *entry;
            format!(
                "{method} {path} ({component}) is classified {category:?} in the {} exclusion \
                 list, but {category:?} is not valid for that surface — exclusion creep",
                surface.as_str()
            )
        })
        .collect()
}

fn dead_category_failures(cli: &[UncoveredRoute], mcp: &[UncoveredRoute]) -> Vec<String> {
    Category::ALL
        .iter()
        .filter(|category| {
            !cli.iter().any(|entry| entry.3 == **category)
                && !mcp.iter().any(|entry| entry.3 == **category)
        })
        .map(|category| {
            format!("{category:?} has zero rows on either exclusion list — dead category")
        })
        .collect()
}

// ---------------------------------------------------------------------------
// T6a.5 — anti-vacuity (design R4).
// ---------------------------------------------------------------------------

#[test]
fn registry_client_and_surface_walks_are_not_vacuous() {
    let entries = route_matrix();
    assert_eq!(
        entries.len(),
        217,
        "the live registry must declare 217 routes"
    );

    let cli_methods: BTreeSet<String> = cli_command_methods().into_values().flatten().collect();
    assert!(
        !cli_methods.is_empty(),
        "anti-vacuity: no CLI command resolved to any AtlasClient method"
    );

    let mcp_methods: BTreeSet<String> = mcp_operation_methods().into_values().flatten().collect();
    assert!(
        !mcp_methods.is_empty(),
        "anti-vacuity: no MCP operation resolved to any AtlasClient method"
    );

    assert!(
        !client_routes().is_empty(),
        "anti-vacuity: no AtlasClient routes resolved"
    );
}

// ---------------------------------------------------------------------------
// The composed coverage test (all three D7.3 directions).
// ---------------------------------------------------------------------------

#[test]
fn every_registry_route_is_reachable_or_explicitly_excluded_per_surface() {
    let entries = route_matrix();
    let routes = client_routes();

    let cli_commands = cli_command_methods();
    assert_eq!(
        cli_commands.len(),
        CLI_COMMAND_COUNT,
        "the CLI walk must resolve exactly the {CLI_COMMAND_COUNT} dispatch arms \
         cli_mcp_component_derivation.rs pins; a dropped or unparsed command would \
         silently shrink CLI coverage"
    );

    let mcp_operations = mcp_operation_methods();
    assert_eq!(
        mcp_operations.len(),
        MCP_OPERATION_COUNT,
        "the MCP walk must resolve exactly the {MCP_OPERATION_COUNT} catalogued \
         operations cli_mcp_component_derivation.rs pins; a dropped or unparsed \
         operation would silently shrink MCP coverage"
    );

    let cli_covered = covered_routes(cli_commands.into_values(), &routes);
    let mcp_covered = covered_routes(mcp_operations.into_values(), &routes);

    let mut failures = unnamed_gap_failures(&entries, &cli_covered, CLI_UNCOVERED, Surface::Cli);
    failures.extend(unnamed_gap_failures(
        &entries,
        &mcp_covered,
        MCP_UNCOVERED,
        Surface::Mcp,
    ));
    failures.extend(stale_exclusion_failures(
        &cli_covered,
        CLI_UNCOVERED,
        Surface::Cli,
    ));
    failures.extend(stale_exclusion_failures(
        &mcp_covered,
        MCP_UNCOVERED,
        Surface::Mcp,
    ));
    failures.extend(exclusion_creep_failures(CLI_UNCOVERED, Surface::Cli));
    failures.extend(exclusion_creep_failures(MCP_UNCOVERED, Surface::Mcp));
    failures.extend(dead_category_failures(CLI_UNCOVERED, MCP_UNCOVERED));

    assert!(
        failures.is_empty(),
        "coverage gaps (directions 1-3: unnamed gap, stale exclusion, exclusion creep):\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// T6b.1/T6b.2 probes — direction 2 (stale exclusion), synthetic fixtures.
// ---------------------------------------------------------------------------

#[test]
fn a_stale_exclusion_that_is_now_covered_is_flagged_by_name() {
    let covered: BTreeSet<RouteKey> =
        BTreeSet::from([route_key("acta", HttpMethod::Get, "/probe/{id}")]);
    let stale_entry: UncoveredRoute = (
        "acta",
        HttpMethod::Get,
        "/probe/{id}",
        Category::RootProbe,
        "probe: planted stale entry for a route the surface now covers",
    );

    let failures = stale_exclusion_failures(&covered, &[stale_entry], Surface::Cli);

    assert_eq!(
        failures.len(),
        1,
        "a stale exclusion entry (route now covered) must be flagged by name"
    );
    assert!(
        failures
            .first()
            .is_some_and(|failure| failure.contains("/probe/{id}"))
    );
}

#[test]
fn a_genuinely_uncovered_exclusion_is_not_flagged_as_stale() {
    let covered: BTreeSet<RouteKey> = BTreeSet::new();
    let entry: UncoveredRoute = (
        "acta",
        HttpMethod::Get,
        "/probe/{id}",
        Category::RootProbe,
        "probe: genuinely uncovered, must not be flagged",
    );

    let failures = stale_exclusion_failures(&covered, &[entry], Surface::Cli);

    assert!(
        failures.is_empty(),
        "an exclusion for a route no surface covers must not be flagged stale"
    );
}

// ---------------------------------------------------------------------------
// T6b.3/T6b.4 probes — direction 3 (exclusion creep), synthetic fixtures.
// ---------------------------------------------------------------------------

#[test]
fn a_category_used_on_the_wrong_surface_is_flagged_as_creep() {
    let entry: UncoveredRoute = (
        "custos",
        HttpMethod::Get,
        "/probe/{id}",
        Category::Users, // MCP-only category, planted on the CLI list.
        "probe: Users planted on the CLI exclusion list",
    );

    let failures = exclusion_creep_failures(&[entry], Surface::Cli);

    assert_eq!(
        failures.len(),
        1,
        "a category not valid for its surface must be flagged as exclusion creep"
    );
}

#[test]
fn a_category_valid_for_its_surface_is_not_flagged_as_creep() {
    let entry: UncoveredRoute = (
        "acta",
        HttpMethod::Get,
        "/probe/{id}",
        Category::Comments, // CLI-only category, correctly on the CLI list.
        "probe: correctly classified",
    );

    let failures = exclusion_creep_failures(&[entry], Surface::Cli);

    assert!(
        failures.is_empty(),
        "a category valid for its surface must not be flagged as creep"
    );
}

#[test]
fn a_category_absent_from_both_exclusion_lists_is_flagged_as_dead() {
    let cli: Vec<UncoveredRoute> = vec![(
        "acta",
        HttpMethod::Get,
        "/probe/{id}",
        Category::Comments,
        "probe: the only category this synthetic universe uses",
    )];
    let mcp: Vec<UncoveredRoute> = vec![];

    let failures = dead_category_failures(&cli, &mcp);

    assert!(
        !failures.iter().any(|f| f.contains("Comments")),
        "a category with a row on some surface must not be flagged dead"
    );
    assert!(
        failures.iter().any(|f| f.contains("Webhooks")),
        "a category with zero rows on every surface must be flagged dead"
    );
}

#[test]
fn category_all_lists_every_variant_exactly_once() {
    let distinct: BTreeSet<String> = Category::ALL.iter().map(|c| format!("{c:?}")).collect();

    assert_eq!(Category::ALL.len(), Category::COUNT);
    assert_eq!(
        distinct.len(),
        Category::ALL.len(),
        "Category::ALL must not repeat a variant"
    );
}

#[test]
fn no_real_exclusion_list_category_is_dead() {
    let failures = dead_category_failures(CLI_UNCOVERED, MCP_UNCOVERED);

    assert!(
        failures.is_empty(),
        "every closed Category variant must have at least one real row on some surface:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Per-surface reached/excluded pins — measured at apply time against the
// 217-route registry; both pairs must sum to it.
// ---------------------------------------------------------------------------

const CLI_REACHED_ROUTE_COUNT: usize = 113;
const CLI_EXCLUDED_ROUTE_COUNT: usize = 104;
const MCP_REACHED_ROUTE_COUNT: usize = 112;
const MCP_EXCLUDED_ROUTE_COUNT: usize = 105;

fn reached_route_count(entries: &[RouteMatrixEntry], covered: &BTreeSet<RouteKey>) -> usize {
    let keys: BTreeSet<RouteKey> = entries
        .iter()
        .map(|entry| route_key(&entry.component, entry.method, &entry.path_template))
        .collect();
    keys.iter().filter(|key| covered.contains(key)).count()
}

/// A route reached through a delegating wrapper (`atlas tasks create` ⇒
/// `create_task` ⇒ `create_task_with_references`) counts as reached, so a
/// regression in wrapper resolution shrinks the reached count here instead
/// of reappearing as a false exclusion row.
#[test]
fn per_surface_reached_and_excluded_counts_are_pinned() {
    let entries = route_matrix();
    let routes = client_routes();

    let cli_covered = covered_routes(cli_command_methods().into_values(), &routes);
    let mcp_covered = covered_routes(mcp_operation_methods().into_values(), &routes);

    assert_eq!(
        reached_route_count(&entries, &cli_covered),
        CLI_REACHED_ROUTE_COUNT,
        "CLI reached-route count changed; re-measure and re-pin together with CLI_UNCOVERED"
    );
    assert_eq!(CLI_UNCOVERED.len(), CLI_EXCLUDED_ROUTE_COUNT);
    assert_eq!(
        CLI_REACHED_ROUTE_COUNT + CLI_EXCLUDED_ROUTE_COUNT,
        entries.len(),
        "CLI reached + excluded must partition the registry"
    );

    assert_eq!(
        reached_route_count(&entries, &mcp_covered),
        MCP_REACHED_ROUTE_COUNT,
        "MCP reached-route count changed; re-measure and re-pin together with MCP_UNCOVERED"
    );
    assert_eq!(MCP_UNCOVERED.len(), MCP_EXCLUDED_ROUTE_COUNT);
    assert_eq!(
        MCP_REACHED_ROUTE_COUNT + MCP_EXCLUDED_ROUTE_COUNT,
        entries.len(),
        "MCP reached + excluded must partition the registry"
    );
}
