//! `acta` component router (design D1/D5/D6, `v2-e3-s2-router-audit` PR4).
//!
//! Acta's 169 routes split the same way they do in today's `lib.rs`: almost
//! everything sits behind `require_authn` → `require_rate_limit` →
//! CSRF-for-cookie-mutations (`lib.rs`'s `protected` router, pre-PR4); one
//! route (`ingest_github_event`, the public GitHub webhook ingest, A8) is
//! unauthenticated and carries its own per-route rate limiter, matching
//! `lib.rs`'s `public` router (pre-PR4: `lib.rs:649-676`).
//!
//! ## T4.1 findings: acta is NOT layer-uniform
//!
//! Reading `lib.rs`'s pre-PR4 protected block route by route (not inferred
//! from route count) surfaces seven routes carrying a per-route
//! `DefaultBodyLimit` layer on top of the router-wide stack, in addition to
//! the one route-specific governor already known from the design (mirroring
//! custos's `login`/`activate`, D6):
//!
//! | Path | Method(s) | Layer |
//! |---|---|---|
//! | `/api/workspaces/{ws}/tasks/{readable_id}/references/batch` | POST | `DefaultBodyLimit::max(1 MiB)` |
//! | `/api/workspaces/{ws}/tasks/{readable_id}/attachments` | POST, GET | `DefaultBodyLimit::max(attachment_body_limit)` |
//! | `/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments` | POST, GET | `DefaultBodyLimit::max(attachment_body_limit)` |
//! | `/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments` | POST | `DefaultBodyLimit::max(attachment_body_limit)` |
//! | `/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments` | POST, GET | `DefaultBodyLimit::max(attachment_body_limit)` |
//! | `/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments` | POST | `DefaultBodyLimit::max(attachment_body_limit)` |
//! | `/api/workspaces/{ws}/documents/moves/batch` | POST | `DefaultBodyLimit::max(1 MiB)` |
//! | `/api/workspaces/{ws}/integrations/{integration}/events` | POST | `GovernorLayer` (public, unauthenticated) |
//!
//! `component_routes!` has no grammar for a per-route `.layer(...)` (see
//! `routes::custos`'s module doc for why the macro was not extended for a
//! minority case). All eight routes above sit outside the macro in the
//! `layered`/`public` sub-modules below, hand-declaring their `AuditedRoute`
//! entries exactly the way custos hand-declares `login`/`activate` — same
//! tradeoff, same coverage (declared-vs-registry, never declared-vs-router;
//! see `routes::custos`'s module doc for the full argument). Every other
//! acta route follows the plain two-tier `require_authn` →
//! `require_rate_limit` → CSRF-for-mutations pattern with no per-route
//! exception, confirmed by reading all 169 route declarations directly, not
//! assumed from the component's size.
//!
//! `attachment_body_limit` is computed from `state.max_attachment_bytes` at
//! `router(state)` call time (identical construction to pre-PR4
//! `lib.rs:143-149`), so it cannot be a macro-time constant either way — one
//! more reason these seven routes stay outside `component_routes!`.
//!
//! ## `/openapi.json` and `/scalar`
//!
//! Both are owned by `platform` in `docs/registry-route-ownership.md` (line
//! 57: "baseline (explicit)") and excluded from every per-component audit
//! entirely (D4: `RoutePath::new` rejects `/openapi.json`'s literal `.`;
//! `/scalar` is a `.merge()`, never a `.route()`, with no operation
//! contract) — so neither ever gets a `declared_routes()` entry on ANY
//! component, and no audit in this slice can tell which component's router
//! module physically mounts them. PR2 left both inline in `lib.rs` rather
//! than moving them into `platform::router()`, since they were not among
//! platform's six audited routes.
//!
//! This PR's own gate (design D6, restated by the orchestrator for PR4)
//! requires `lib.rs::app()` to contain zero inline `.route()`/`.merge()`
//! calls once acta's conversion lands, which forces relocating these two.
//! They are mounted in `acta::router()`'s `public` sub-module below —
//! alongside `ingest_github_event`, exactly where they already lived
//! together in `lib.rs`'s own `public` router (pre-PR4: `lib.rs:666-676`) —
//! rather than in `platform::router()`, to avoid touching `platform.rs` in a
//! PR scoped to acta. This is a placement choice only: neither route is
//! declared, audited, or capability-checked as an "acta route" by doing so,
//! since D4 already excludes both from every audit regardless of which
//! module mounts them.
#![allow(
    unreachable_pub,
    reason = "component_routes! always emits `pub fn router` to match the real \
              per-component contract (D1); this module's own path is `pub(crate)` \
              (`routes::acta` in `routes/mod.rs`), so the `pub` here is never \
              reachable from outside the crate, which is by design, not an oversight"
)]

use axum::Router;

use crate::router_audit::AuditedRoute;
use crate::state::AppState;

/// Unauthenticated routes (`lib.rs`'s `public` router, pre-PR4:
/// `lib.rs:649-676`): the GitHub webhook ingest (A8, carrying its own
/// per-IP governor) plus `/openapi.json` and `/scalar` (platform-owned,
/// D4-excluded from every audit — see the module doc for why they live
/// here).
mod public {
    use std::sync::Arc;

    use atlas_core::registry::HttpMethod;
    use axum::Router;
    use axum::routing::get;
    use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};

    use crate::router_audit::{AuditedRoute, DeclaredScope};
    use crate::routes::{integrations_ingest, openapi};
    use crate::state::AppState;

    pub fn router(state: AppState) -> Router {
        // per_second/burst_size are non-zero, so finish() always returns
        // Some here (identical construction to pre-PR4 `lib.rs:656-664`). A
        // single GitHub source IP fans out deliveries for many
        // repos/workspaces, and a rejected delivery is retried by GitHub, so
        // the quota is a little higher than login/activate's.
        #[allow(clippy::expect_used)]
        let ingest_config = {
            let mut b = GovernorConfigBuilder::default();
            let cfg = b
                .per_second(5)
                .burst_size(20)
                .finish()
                .expect("governor config");
            Arc::new(cfg)
        };

        Router::new()
            .route(
                "/api/workspaces/{ws}/integrations/{integration}/events",
                axum::routing::post(integrations_ingest::ingest_github_event)
                    .layer(GovernorLayer::new(ingest_config)),
            )
            .route("/openapi.json", get(openapi::openapi_json))
            .merge(openapi::scalar_router())
            .with_state(state)
    }

    /// Hand-declared (see module doc): `ingest_github_event` takes no
    /// `Authorized<...>` extractor at all (HMAC-verified by its own
    /// extractor instead), so it is capability-extraction exempt (D5),
    /// matching the registry's `action: None`. `/openapi.json` and
    /// `/scalar` carry no entry at all — see the module doc for why.
    pub(crate) fn declared_routes() -> Vec<AuditedRoute> {
        vec![AuditedRoute {
            method: HttpMethod::Post,
            path: "/api/workspaces/{ws}/integrations/{integration}/events",
            scope: DeclaredScope::Unauthenticated,
        }]
    }
}

/// Workspace/project administration, tags, status templates, property
/// definitions, saved searches, and task views: 47 routes, none carrying a
/// per-route layer.
///
/// Exemption scaffold (D5) applies to every route here whose handler takes
/// a gate OTHER than `Authorized<R, M, S>` — `RequireUserAdmin`,
/// `WorkspaceMember`, `WorkspaceOwnerOrAdmin`, or a bare
/// `Extension<Principal>` — verified by reading each handler's real
/// signature directly (`routes::trash`, `routes::workspaces`,
/// `routes::platform_status_templates`, `routes::projects`,
/// `routes::members`, `routes::saved_searches`, `routes::task_views`), the
/// same exemption category custos established for its own non-`Authorized`
/// gates. `tags`, `status_templates`, and `property_definitions` all take
/// real `Authorized<...>` extractors and go through capability extraction.
mod workspace_admin {
    use crate::routes::{
        members, platform_status_templates, projects, property_definitions, saved_searches,
        status_templates, tags, task_views, trash, workspaces,
    };
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/api/admin/trash" => [ get(trash::list_trash, exempt) ];
        "/api/admin/trash/restore" => [ post(trash::restore_trash, exempt) ];
        "/api/admin/trash/purge" => [ post(trash::purge_trash, exempt) ];
        "/api/admin/trash/purges/{operation_id}" => [ get(trash::get_purge_status, exempt) ];
        "/api/workspaces" => [
            post(workspaces::create_workspace, exempt),
            get(workspaces::list_workspaces, exempt)
        ];
        "/api/workspaces/{ws}" => [
            get(workspaces::get_workspace, exempt),
            patch(workspaces::update_workspace, exempt)
        ];
        "/api/admin/workspaces" => [ get(workspaces::admin_list_workspaces, exempt) ];
        "/api/admin/workspaces/{ws}" => [
            patch(workspaces::admin_update_workspace, exempt),
            delete(workspaces::admin_delete_workspace, exempt)
        ];
        "/api/admin/status-templates" => [
            get(platform_status_templates::list_platform_status_templates, exempt),
            post(platform_status_templates::create_platform_status_template, exempt)
        ];
        "/api/admin/status-templates/{template_id}" => [
            patch(platform_status_templates::update_platform_status_template, exempt),
            delete(platform_status_templates::delete_platform_status_template, exempt)
        ];
        "/api/workspaces/{ws}/projects" => [
            post(projects::create_project),
            get(projects::list_projects, exempt)
        ];
        "/api/workspaces/{ws}/projects/{project_slug}" => [
            get(projects::get_project),
            patch(projects::update_project),
            delete(projects::delete_project)
        ];
        "/api/workspaces/{ws}/members" => [
            get(members::list_workspace_members, exempt),
            post(members::add_member, exempt)
        ];
        "/api/workspaces/{ws}/assignable-users" => [
            get(members::list_assignable_users, exempt)
        ];
        "/api/workspaces/{ws}/members/{user_id}" => [
            patch(members::update_member_role, exempt),
            delete(members::remove_member, exempt)
        ];
        "/api/workspaces/{ws}/tags" => [
            get(tags::list_tags),
            post(tags::create_tag)
        ];
        "/api/workspaces/{ws}/tags/used" => [ get(tags::list_used_labels) ];
        "/api/workspaces/{ws}/tags/{tag_id}" => [
            patch(tags::patch_tag),
            delete(tags::delete_tag)
        ];
        "/api/workspaces/{ws}/status-templates" => [
            get(status_templates::list_status_templates),
            post(status_templates::create_status_template)
        ];
        "/api/workspaces/{ws}/status-templates/{template_id}" => [
            patch(status_templates::update_status_template),
            delete(status_templates::delete_status_template)
        ];
        "/api/workspaces/{ws}/boards/{board_id}/apply-status-templates" => [
            post(status_templates::apply_status_templates)
        ];
        "/api/workspaces/{ws}/property-definitions" => [
            get(property_definitions::list_property_definitions),
            post(property_definitions::create_property_definition)
        ];
        "/api/workspaces/{ws}/property-definitions/{property_definition_id}" => [
            delete(property_definitions::delete_property_definition)
        ];
        "/api/workspaces/{ws}/saved-searches" => [
            get(saved_searches::list_saved_searches, exempt),
            post(saved_searches::create_saved_search, exempt)
        ];
        "/api/workspaces/{ws}/saved-searches/{id}" => [
            patch(saved_searches::rename_saved_search, exempt),
            delete(saved_searches::delete_saved_search, exempt)
        ];
        "/api/workspaces/{ws}/task-views" => [
            get(task_views::list_task_views, exempt),
            post(task_views::create_task_view, exempt)
        ];
        "/api/workspaces/{ws}/task-views/{id}" => [
            get(task_views::get_task_view, exempt),
            patch(task_views::update_task_view, exempt),
            delete(task_views::delete_task_view, exempt)
        ];
    }
}

/// Boards, board/document presence, and tasks (comments, checklists,
/// subtasks, references): 53 routes, excluding the six task-family routes
/// carrying a per-route `DefaultBodyLimit` (`layered`, below). Every
/// handler here takes a real `Authorized<R, M, S>` extractor and goes
/// through capability extraction, except `list_workspace_tasks` and
/// `list_workspace_activity`, which take `WorkspaceMember` (D5 exemption).
mod boards_tasks {
    use crate::routes::{boards, presence, tasks};
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/api/workspaces/{ws}/projects/{project_slug}/boards" => [
            post(boards::create_board),
            get(boards::list_boards)
        ];
        "/api/workspaces/{ws}/boards/{board_id}" => [
            get(boards::get_board),
            patch(boards::update_board),
            delete(boards::delete_board)
        ];
        "/api/workspaces/{ws}/boards/{board_id}/move" => [ patch(boards::move_board) ];
        "/api/workspaces/{ws}/boards/{board_id}/archive" => [ post(boards::archive_board) ];
        "/api/workspaces/{ws}/boards/{board_id}/unarchive" => [ post(boards::unarchive_board) ];
        "/api/workspaces/{ws}/boards/{board_id}/columns" => [
            post(boards::create_column),
            get(boards::list_columns)
        ];
        "/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}" => [
            patch(boards::update_column),
            delete(boards::delete_column)
        ];
        "/api/workspaces/{ws}/boards/{board_id}/tasks" => [
            post(tasks::create_task),
            get(tasks::list_tasks)
        ];
        "/api/workspaces/{ws}/boards/{board_id}/presence" => [
            post(presence::heartbeat),
            delete(presence::leave)
        ];
        "/api/workspaces/{ws}/documents/{slug}/presence" => [
            post(presence::document_heartbeat),
            delete(presence::document_leave)
        ];
        "/api/workspaces/{ws}/tasks" => [ get(tasks::list_workspace_tasks, exempt) ];
        "/api/workspaces/{ws}/tasks/{readable_id}" => [
            get(tasks::get_task),
            patch(tasks::update_task),
            delete(tasks::delete_task)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/move" => [ post(tasks::move_task) ];
        "/api/workspaces/{ws}/tasks/{readable_id}/assignees" => [
            get(tasks::list_assignees),
            post(tasks::add_assignee)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}" => [
            delete(tasks::remove_assignee)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/references" => [
            get(tasks::list_references),
            post(tasks::create_reference)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}" => [
            delete(tasks::delete_reference)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content" => [
            get(tasks::download_attachment)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}" => [
            patch(tasks::rename_attachment),
            delete(tasks::delete_attachment)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts" => [
            post(tasks::create_comment_draft)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}" => [
            delete(tasks::cancel_comment_draft)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content" => [
            get(tasks::download_comment_attachment)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}" => [
            delete(tasks::delete_comment_attachment)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/backlinks" => [ get(tasks::list_backlinks) ];
        "/api/workspaces/{ws}/tasks/{readable_id}/graph" => [ get(tasks::get_task_graph) ];
        "/api/workspaces/{ws}/tasks/{readable_id}/checklist" => [
            get(tasks::list_checklist),
            post(tasks::create_checklist_item)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}" => [
            patch(tasks::update_checklist_item),
            delete(tasks::delete_checklist_item)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote" => [
            post(tasks::promote_checklist_item)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/subtasks" => [
            get(tasks::list_subtasks),
            post(tasks::create_subtask)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/promote" => [ post(tasks::promote_subtask) ];
        "/api/workspaces/{ws}/tasks/{readable_id}/parent" => [ post(tasks::set_task_parent) ];
        "/api/workspaces/{ws}/tasks/{readable_id}/activity" => [ get(tasks::list_activity) ];
        "/api/workspaces/{ws}/tasks/{readable_id}/comments" => [
            get(tasks::list_comments),
            post(tasks::create_comment)
        ];
        "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}" => [
            patch(tasks::update_comment),
            delete(tasks::delete_comment)
        ];
        "/api/workspaces/{ws}/activity" => [ get(tasks::list_workspace_activity, exempt) ];
    }
}

/// Documents (content, revisions, comments, backlinks) and folders: 37
/// routes, excluding the four document-family routes carrying a per-route
/// `DefaultBodyLimit` (`layered`, below). Every handler takes a real
/// `Authorized<R, M, S>` extractor, except the workspace-wide attachment
/// listing/download/rename/delete routes, which take `WorkspaceAccess` or
/// `WorkspaceMember` (D5 exemption) — those routes are addressed by
/// workspace-scoped attachment id, not by document slug, so they cannot
/// resolve a `DocumentSlugRes`.
mod documents_folders {
    use crate::routes::{attachments, documents, folders};
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/api/workspaces/{ws}/projects/{project_slug}/documents" => [
            post(documents::create_document),
            get(documents::list_documents)
        ];
        "/api/workspaces/{ws}/documents/{slug}" => [
            get(documents::get_document),
            patch(documents::update_document),
            delete(documents::delete_document)
        ];
        "/api/workspaces/{ws}/documents/{slug}/content" => [ put(documents::update_content) ];
        "/api/workspaces/{ws}/documents/{slug}/compact" => [
            get(documents::get_document_compact)
        ];
        "/api/workspaces/{ws}/documents/{slug}/content/range" => [
            get(documents::get_content_range),
            patch(documents::edit_content_range)
        ];
        "/api/workspaces/{ws}/documents/{slug}/content/search" => [
            post(documents::search_content)
        ];
        "/api/workspaces/{ws}/documents/{slug}/history" => [ get(documents::list_history) ];
        "/api/workspaces/{ws}/documents/{slug}/revisions/{seq}" => [
            get(documents::get_revision_content)
        ];
        "/api/workspaces/{ws}/documents/{slug}/backlinks" => [ get(documents::list_backlinks) ];
        "/api/workspaces/{ws}/documents/{slug}/frontmatter" => [
            get(documents::get_frontmatter)
        ];
        "/api/workspaces/{ws}/documents/{slug}/attachments" => [
            post(documents::upload_attachment),
            get(documents::list_attachments)
        ];
        "/api/workspaces/{ws}/attachments" => [
            get(attachments::list_workspace_attachments, exempt)
        ];
        "/api/workspaces/{ws}/attachments/{attachment_id}" => [
            get(documents::download_attachment, exempt),
            patch(attachments::rename_attachment, exempt),
            delete(documents::delete_attachment, exempt)
        ];
        "/api/workspaces/{ws}/documents/{slug}/comment-drafts" => [
            post(documents::create_comment_draft)
        ];
        "/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}" => [
            delete(documents::cancel_comment_draft)
        ];
        "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}" => [
            get(documents::download_comment_attachment),
            delete(documents::delete_comment_attachment)
        ];
        "/api/workspaces/{ws}/documents/{slug}/move" => [ patch(documents::move_document) ];
        "/api/workspaces/{ws}/documents/{slug}/copy" => [ post(documents::copy_document) ];
        "/api/workspaces/{ws}/documents/{slug}/comments" => [
            get(documents::list_comments),
            post(documents::create_comment)
        ];
        "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}" => [
            patch(documents::update_comment),
            delete(documents::delete_comment)
        ];
        "/api/workspaces/{ws}/projects/{project_slug}/folders" => [
            post(folders::create_folder),
            get(folders::list_folders)
        ];
        "/api/workspaces/{ws}/folders/{folder_id}" => [
            get(folders::get_folder),
            patch(folders::rename_folder),
            delete(folders::delete_folder)
        ];
        "/api/workspaces/{ws}/folders/{folder_id}/move" => [ patch(folders::move_folder) ];
        "/api/workspaces/{ws}/folders/{folder_id}/copy" => [ post(folders::copy_folder) ];
    }
}

/// Live updates (SSE) and search: 5 routes. `stream_events`, `search`, and
/// `semantic_search` take `WorkspaceAccess` (D5 exemption, matching the
/// registry's `action: None` for all three per
/// `docs/registry-route-ownership.md`); the reindex endpoints take real
/// `Authorized<...>` extractors.
mod search_family {
    use crate::routes::{events, search, semantic_search};
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/api/workspaces/{ws}/events" => [ get(events::stream_events, exempt) ];
        "/api/workspaces/{ws}/search" => [ get(search::search, exempt) ];
        "/api/workspaces/{ws}/semantic-search" => [ get(semantic_search::semantic_search, exempt) ];
        "/api/workspaces/{ws}/semantic-search/reindex" => [
            get(semantic_search::semantic_reindex_plan),
            post(semantic_search::semantic_reindex_start)
        ];
    }
}

/// Webhooks (admin-only subscription CRUD + delivery log), integration
/// configs, and automation rules: 16 routes, all admin-only and all taking
/// real `Authorized<...>` extractors.
mod webhooks_automations {
    use crate::routes::{automation_rules, integration_configs, webhooks};
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/api/workspaces/{ws}/webhooks" => [
            post(webhooks::create_webhook),
            get(webhooks::list_webhooks)
        ];
        "/api/workspaces/{ws}/webhooks/{webhook_id}" => [
            get(webhooks::get_webhook),
            patch(webhooks::update_webhook),
            delete(webhooks::delete_webhook)
        ];
        "/api/workspaces/{ws}/webhooks/{webhook_id}/deliveries" => [
            get(webhooks::list_webhook_deliveries)
        ];
        "/api/workspaces/{ws}/integration-configs" => [
            post(integration_configs::create_integration_config),
            get(integration_configs::list_integration_configs)
        ];
        "/api/workspaces/{ws}/integration-configs/{config_id}" => [
            get(integration_configs::get_integration_config),
            patch(integration_configs::patch_integration_config),
            delete(integration_configs::delete_integration_config)
        ];
        "/api/workspaces/{ws}/automation-rules" => [
            post(automation_rules::create_automation_rule),
            get(automation_rules::list_automation_rules)
        ];
        "/api/workspaces/{ws}/automation-rules/{rule_id}" => [
            get(automation_rules::get_automation_rule),
            patch(automation_rules::patch_automation_rule),
            delete(automation_rules::delete_automation_rule)
        ];
    }
}

/// Hand-declared (see the module doc's T4.1 table): seven routes carrying a
/// per-route `DefaultBodyLimit` layer that `component_routes!` cannot
/// express, mirroring how `routes::custos::public` hand-declares
/// `login`/`activate`. Every handler here takes a real `Authorized<R, M,
/// S>` extractor and goes through the same `declared_scope()` extraction
/// call `component_routes!`'s own `@entry` arm makes — this module only
/// skips the macro's `.route()`/`.layer()` grammar, never D5's extraction
/// mechanism.
mod layered {
    use atlas_core::registry::HttpMethod;
    use axum::Router;
    use axum::extract::DefaultBodyLimit;

    use crate::authz::ExtractScope;
    use crate::router_audit::{AuditedRoute, DeclaredScope};
    use crate::routes::{documents, tasks};
    use crate::state::AppState;

    /// Total-body cap for multipart attachment uploads (identical
    /// construction to pre-PR4 `lib.rs:143-149`): the per-chunk streaming
    /// cap inside each handler's `file` part is not a substitute for a hard
    /// total-body limit, since any OTHER multipart part streams unbounded
    /// without one.
    fn attachment_body_limit(state: &AppState) -> usize {
        const ATTACHMENT_BODY_SLACK: u64 = 1024 * 1024;
        usize::try_from(
            state
                .max_attachment_bytes
                .saturating_add(ATTACHMENT_BODY_SLACK),
        )
        .unwrap_or(usize::MAX)
    }

    pub fn router(state: AppState) -> Router {
        let attachment_body_limit = attachment_body_limit(&state);

        Router::new()
            .route(
                "/api/workspaces/{ws}/tasks/{readable_id}/references/batch",
                axum::routing::post(tasks::create_references_batch)
                    .layer(DefaultBodyLimit::max(1024 * 1024)),
            )
            .route(
                "/api/workspaces/{ws}/tasks/{readable_id}/attachments",
                axum::routing::post(tasks::upload_attachment)
                    .get(tasks::list_attachments)
                    .layer(DefaultBodyLimit::max(attachment_body_limit)),
            )
            .route(
                "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
                axum::routing::post(tasks::upload_comment_attachment)
                    .get(tasks::list_comment_attachments)
                    .layer(DefaultBodyLimit::max(attachment_body_limit)),
            )
            .route(
                "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
                axum::routing::post(tasks::upload_comment_draft_attachment)
                    .layer(DefaultBodyLimit::max(attachment_body_limit)),
            )
            .route(
                "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
                axum::routing::post(documents::upload_comment_attachment)
                    .get(documents::list_comment_attachments)
                    .layer(DefaultBodyLimit::max(attachment_body_limit)),
            )
            .route(
                "/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
                axum::routing::post(documents::upload_comment_draft_attachment)
                    .layer(DefaultBodyLimit::max(attachment_body_limit)),
            )
            .route(
                "/api/workspaces/{ws}/documents/moves/batch",
                axum::routing::post(documents::move_documents_batch)
                    .layer(DefaultBodyLimit::max(1024 * 1024)),
            )
            .with_state(state)
    }

    pub(crate) fn declared_routes() -> Vec<AuditedRoute> {
        vec![
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/api/workspaces/{ws}/tasks/{readable_id}/references/batch",
                scope: DeclaredScope::Extracted(tasks::create_references_batch.declared_scope()),
            },
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/api/workspaces/{ws}/tasks/{readable_id}/attachments",
                scope: DeclaredScope::Extracted(tasks::upload_attachment.declared_scope()),
            },
            AuditedRoute {
                method: HttpMethod::Get,
                path: "/api/workspaces/{ws}/tasks/{readable_id}/attachments",
                scope: DeclaredScope::Extracted(tasks::list_attachments.declared_scope()),
            },
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
                scope: DeclaredScope::Extracted(tasks::upload_comment_attachment.declared_scope()),
            },
            AuditedRoute {
                method: HttpMethod::Get,
                path: "/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
                scope: DeclaredScope::Extracted(tasks::list_comment_attachments.declared_scope()),
            },
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
                scope: DeclaredScope::Extracted(
                    tasks::upload_comment_draft_attachment.declared_scope(),
                ),
            },
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
                scope: DeclaredScope::Extracted(
                    documents::upload_comment_attachment.declared_scope(),
                ),
            },
            AuditedRoute {
                method: HttpMethod::Get,
                path: "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
                scope: DeclaredScope::Extracted(
                    documents::list_comment_attachments.declared_scope(),
                ),
            },
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
                scope: DeclaredScope::Extracted(
                    documents::upload_comment_draft_attachment.declared_scope(),
                ),
            },
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/api/workspaces/{ws}/documents/moves/batch",
                scope: DeclaredScope::Extracted(documents::move_documents_batch.declared_scope()),
            },
        ]
    }
}

/// Builds acta's router, reproducing today's exact public/protected split
/// and layer stack (D6): no layer added, removed, or reordered — only the
/// `.route()` call sites moved out of `lib.rs::app()` into this module
/// (plus `/openapi.json`/`/scalar`, relocated here per the module doc).
pub fn router(state: AppState) -> Router {
    let public = public::router(state.clone());
    let protected = workspace_admin::router(state.clone())
        .merge(boards_tasks::router(state.clone()))
        .merge(documents_folders::router(state.clone()))
        .merge(search_family::router(state.clone()))
        .merge(webhooks_automations::router(state.clone()))
        .merge(layered::router(state.clone()))
        .layer(axum::middleware::from_fn(
            crate::auth::csrf::require_csrf_for_cookie_mutations,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::rate_limit::require_rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::middleware::require_authn,
        ));

    public.merge(protected)
}

/// The union of every sub-router's declared routes. Order-independent: the
/// audits below compare sets, never sequences. Only this module's own
/// bidirectional/declare-and-verify audit tests call the full union today
/// (T4.9's mount assertion deliberately uses the narrower
/// `public_declared_routes()` below instead); PR5's T5.7 crate-wide sweep is
/// this function's next production caller.
#[allow(
    dead_code,
    reason = "only this module's own #[cfg(test)] audit tests call the full \
              declared_routes() union outside a test build; PR5's T5.7 crate-wide \
              sweep is its next production caller"
)]
pub(crate) fn declared_routes() -> Vec<AuditedRoute> {
    let mut routes = public::declared_routes();
    routes.extend(workspace_admin::declared_routes());
    routes.extend(boards_tasks::declared_routes());
    routes.extend(documents_folders::declared_routes());
    routes.extend(search_family::declared_routes());
    routes.extend(webhooks_automations::declared_routes());
    routes.extend(layered::declared_routes());
    routes
}

/// Just `ingest_github_event` (`public`, above), read by
/// `router_audit::acta_route_paths()` for T4.9's per-component mount
/// assertion: unlike the full `declared_routes()` union, this route is safe
/// to probe with an unauthenticated request and a foreign method, since it
/// does not sit behind `require_authn` — a probe against any other acta
/// route would get 401 from that layer before ever reaching the method
/// dispatch that would otherwise answer 405. `/openapi.json`/`/scalar` are
/// also mounted in `acta::public::router()` but carry no `declared_routes()`
/// entry at all (see the module doc), so they are not candidates here
/// either way.
pub(crate) fn public_declared_routes() -> Vec<AuditedRoute> {
    public::declared_routes()
}

/// The declared routes behind `require_authn` (D6): every sub-router's
/// entries except `public`'s. Read by
/// `router_audit::acta_protected_route_paths()` for
/// `tests/api_acta_router_parity.rs`'s data-driven unauthenticated-401 proof
/// — the mirror image of `public_declared_routes()` above, which exists for
/// the opposite (safe-to-probe-unauthenticated) reason.
pub(crate) fn protected_declared_routes() -> Vec<AuditedRoute> {
    let mut routes = workspace_admin::declared_routes();
    routes.extend(boards_tasks::declared_routes());
    routes.extend(documents_folders::declared_routes());
    routes.extend(search_family::declared_routes());
    routes.extend(webhooks_automations::declared_routes());
    routes.extend(layered::declared_routes());
    routes
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use atlas_core::registry::{ComponentId, HttpMethod, build};

    use super::*;
    use crate::reg5::{StorageBackend, reg5_component_entries};
    use crate::router_audit::{
        DeclaredScope, capability_from_action_id, diff_declared_and_enforced, diff_route_sets,
    };

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    /// Builds the same registry the live server assembles at startup. See
    /// `routes::platform`'s test module for why the storage backend choice
    /// is irrelevant to this component's own routes.
    fn build_registry() -> atlas_core::registry::Registry {
        build(reg5_component_entries(StorageBackend::Filesystem))
            .expect("REG-5 entries must satisfy every registry::build() validator")
    }

    /// T4.2/T4.4 (bidirectional audit, D2/INV-SET): `acta::declared_routes()`
    /// must equal the registry's `acta.api.routes` set exactly, in both
    /// directions, across all 169 routes.
    #[test]
    fn acta_router_and_registry_route_sets_match_exactly() {
        let registry = build_registry();
        let entry = registry
            .get(&component("acta"))
            .expect("acta is a REG-5 component");

        let router_set: std::collections::HashSet<(HttpMethod, String)> = declared_routes()
            .iter()
            .map(|route| (route.method, route.path.to_string()))
            .collect();

        let registry_set: std::collections::HashSet<(HttpMethod, String)> = entry
            .api
            .routes
            .iter()
            .map(|route| (route.method, route.path.as_str().to_string()))
            .collect();

        let diff = diff_route_sets(&router_set, &registry_set);

        assert!(
            diff.is_empty(),
            "acta's router and registry route sets must match exactly: {diff:?}"
        );
        assert_eq!(
            router_set.len(),
            169,
            "acta owns exactly 169 routes per docs/registry-route-ownership.md"
        );
    }

    /// Declare-and-verify exemption list (distinct from D4's route-SET
    /// exclusion list — this one is scoped to D5's declared-vs-enforced
    /// COMPARISON only; every one of these 14 routes stays in both
    /// `declared_routes()` and the registry's set, and is still audited by
    /// `acta_router_and_registry_route_sets_match_exactly` above).
    ///
    /// Every entry here takes `WorkspaceMember` (D5's capability-extraction
    /// exemption: no `Authorized<R, M, S>` parameter to extract `S::CAPABILITY`
    /// from), yet the registry legitimately declares `action: Some(_)` for
    /// it, because the handler manually re-checks that EXACT capability for
    /// API-key/agent callers before proceeding — verified by reading each
    /// handler body directly, not inferred from the registry:
    ///
    /// - `update_workspace`, `list_projects`, `list_workspace_tasks`: an
    ///   inline `enforce_api_key_scope(...)` call gated on
    ///   `member.api_key_id.is_some()`.
    /// - `list_saved_searches`/`create_saved_search`/`rename_saved_search`/
    ///   `delete_saved_search`: `enforce_saved_searches_scope(...)`
    ///   (`routes::saved_searches`), same pattern, one shared helper.
    /// - `list_task_views`/`create_task_view`/`get_task_view`/
    ///   `update_task_view`/`delete_task_view`: `enforce_task_views_scope(...)`
    ///   (`routes::task_views`), same pattern.
    /// - `download_attachment`/`delete_attachment` (the workspace-scoped
    ///   attachment routes): `authorize_attachment(...)` +
    ///   `enforce_attachment_scope(...)` (`routes::attachments`), same
    ///   pattern.
    ///
    /// A human `WorkspaceMember` passes all fourteen unconditionally (any
    /// member may act on their own workspace's saved searches, task views,
    /// etc.); only an API-key/agent caller is additionally required to hold
    /// the declared capability. This is the exact ceiling D5's own module
    /// doc names: "this proves that the handler bound to this route in the
    /// table uses the declared S. It does not prove no other code path can
    /// reach the same business logic ungated" — here the "other code path"
    /// is a manual, principal-kind-conditional re-check inside the SAME
    /// handler, which `ExtractScope`'s compile-time, `Authorized<...>`-only
    /// extraction cannot see. Nulling the registry's `action` to make this
    /// mechanism agree would misrepresent a real, deliberate enforcement
    /// fact — confirmed against the old hand-maintained route registry's
    /// pre-existing `capability: Option<&str>` field for the same routes,
    /// which already carried the identical `Some(_)` value before this PR
    /// touched anything.
    const DECLARE_AND_VERIFY_EXEMPT: [(HttpMethod, &str); 14] = [
        (HttpMethod::Patch, "/api/workspaces/{ws}"),
        (HttpMethod::Get, "/api/workspaces/{ws}/projects"),
        (HttpMethod::Get, "/api/workspaces/{ws}/tasks"),
        (HttpMethod::Get, "/api/workspaces/{ws}/saved-searches"),
        (HttpMethod::Post, "/api/workspaces/{ws}/saved-searches"),
        (
            HttpMethod::Patch,
            "/api/workspaces/{ws}/saved-searches/{id}",
        ),
        (
            HttpMethod::Delete,
            "/api/workspaces/{ws}/saved-searches/{id}",
        ),
        (HttpMethod::Get, "/api/workspaces/{ws}/task-views"),
        (HttpMethod::Post, "/api/workspaces/{ws}/task-views"),
        (HttpMethod::Get, "/api/workspaces/{ws}/task-views/{id}"),
        (HttpMethod::Patch, "/api/workspaces/{ws}/task-views/{id}"),
        (HttpMethod::Delete, "/api/workspaces/{ws}/task-views/{id}"),
        (
            HttpMethod::Get,
            "/api/workspaces/{ws}/attachments/{attachment_id}",
        ),
        (
            HttpMethod::Delete,
            "/api/workspaces/{ws}/attachments/{attachment_id}",
        ),
    ];

    /// T4.7 (declare-and-verify, D5): exhaustive over every one of acta's
    /// 169 routes — data-driven off `declared_routes()`, never a curated
    /// subset (R4) — except the 14 routes named by
    /// `DECLARE_AND_VERIFY_EXEMPT` above, which this test asserts are
    /// EXACTLY the routes it excludes (so the exclusion list itself cannot
    /// silently grow without this test naming the new addition).
    #[test]
    fn acta_declared_actions_match_enforced_capabilities() {
        let registry = build_registry();
        let entry = registry
            .get(&component("acta"))
            .expect("acta is a REG-5 component");

        let routes = declared_routes();
        assert_eq!(
            routes.len(),
            169,
            "T4.7 must audit every acta route, not a sample"
        );

        let checked_routes: Vec<AuditedRoute> = routes
            .iter()
            .filter(|route| !DECLARE_AND_VERIFY_EXEMPT.contains(&(route.method, route.path)))
            .copied()
            .collect();
        assert_eq!(
            checked_routes.len(),
            routes.len() - DECLARE_AND_VERIFY_EXEMPT.len(),
            "DECLARE_AND_VERIFY_EXEMPT must name real, currently-declared acta routes only"
        );

        // Structural check, hardening the exemption list beyond cardinality: every
        // exempted route's handler must genuinely have no `Authorized<...>`
        // capability to extract, i.e. `ExtractScope` must have recorded `None` for
        // it (see the const's own doc: the exemption exists because these routes
        // take `WorkspaceMember`, not `Authorized<R, M, S>`). A route that DOES
        // take `Authorized<...>` with `S::CAPABILITY = Some(_)` would previously
        // pass the cardinality-only check if wrongly added here; this asserts and
        // names it instead.
        for (method, path) in DECLARE_AND_VERIFY_EXEMPT {
            let route = routes
                .iter()
                .find(|route| route.method == method && route.path == path)
                .unwrap_or_else(|| {
                    panic!(
                        "DECLARE_AND_VERIFY_EXEMPT names an unknown acta route: {method:?} {path}"
                    )
                });

            let extracted_capability = match route.scope {
                DeclaredScope::Extracted(capability) => capability,
                DeclaredScope::Unauthenticated => None,
            };

            assert!(
                extracted_capability.is_none(),
                "DECLARE_AND_VERIFY_EXEMPT lists {method:?} {path} as exempt from the \
                 declared-vs-enforced comparison, but its handler's ExtractScope-derived \
                 capability is {extracted_capability:?}, not None — this route takes a real \
                 Authorized<...> extractor and must go through the normal comparison, not the \
                 manual-recheck exemption"
            );
        }

        let declared_actions: HashMap<(HttpMethod, &'static str), Option<_>> = checked_routes
            .iter()
            .map(|route| {
                let action = entry
                    .api
                    .routes
                    .iter()
                    .find(|declared| {
                        declared.method == route.method && declared.path.as_str() == route.path
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "registry has no declaration for {:?} {}",
                            route.method, route.path
                        )
                    })
                    .action
                    .as_ref()
                    .map(capability_from_action_id);
                ((route.method, route.path), action)
            })
            .collect();

        let mismatches = diff_declared_and_enforced(&checked_routes, &declared_actions);

        assert!(
            mismatches.is_empty(),
            "declared vs enforced must agree for every acta route: {mismatches:?}"
        );
    }
}
