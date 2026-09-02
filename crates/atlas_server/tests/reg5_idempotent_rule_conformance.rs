#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! T4.9/T4.14 (`v2-e3-s3` PR4, D8): exhaustive rule-conformance test for
//! `RouteDeclaration.idempotent`'s re-derivation. Every one of the 212
//! `reg5.rs` entries is checked against the written rule
//! (`crates/atlas_core/src/registry/route.rs`'s `idempotent` doc comment),
//! not a sample — a mismatch names the exact offending `(method, path)`
//! (INV-SET-style, never a count comparison).
//!
//! `EXPECTED_IDEMPOTENT` below encodes the same 212 decisions as the
//! judgment file (`docs/reg5-idempotent-judgment.md`):
//! every non-`POST` entry is `false` (T4.10, mechanical); every `POST`
//! entry's value is either a mechanical `create_*` name match or one of the
//! 39 judged decisions the judgment file explains (including the six
//! streamed-upload routes, F4). This table
//! is cross-checked for completeness against `reg5.rs`'s own 212-entry
//! enumeration below (`table_is_exhaustive_over_reg5`), the same
//! "one source, checked exhaustively" shape D4's exclusion-list
//! completeness check uses.

use atlas_core::registry::{HttpMethod, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

const EXPECTED_IDEMPOTENT: &[(HttpMethod, &str, bool)] = &[
    (HttpMethod::Get, "/me/ui-state", false),
    (HttpMethod::Put, "/me/ui-state", false),
    (HttpMethod::Get, "/meta", false),
    (HttpMethod::Get, "/health", false),
    (HttpMethod::Get, "/ready", false),
    (HttpMethod::Get, "/version", false),
    (HttpMethod::Get, "/openapi.json", false),
    (HttpMethod::Get, "/scalar", false),
    (HttpMethod::Post, "/auth/logout", false),
    (HttpMethod::Get, "/auth/me", false),
    (HttpMethod::Post, "/auth/change-password", false),
    (HttpMethod::Patch, "/users/me", false),
    (HttpMethod::Post, "/users", false),
    (HttpMethod::Get, "/users", false),
    (HttpMethod::Post, "/users/{user_id}/disable", false),
    (HttpMethod::Post, "/users/{user_id}/enable", false),
    (HttpMethod::Post, "/users/{user_id}/reset-password", false),
    (HttpMethod::Post, "/users/{user_id}/activation-link", false),
    (HttpMethod::Post, "/users/{user_id}/system-admin", false),
    (HttpMethod::Get, "/users/{user_id}/memberships", false),
    (HttpMethod::Get, "/admin/audit", false),
    (HttpMethod::Post, "/api-keys", false),
    (HttpMethod::Get, "/api-keys", false),
    (HttpMethod::Delete, "/api-keys/{key_id}", false),
    (HttpMethod::Patch, "/api-keys/{key_id}", false),
    (HttpMethod::Get, "/api-keys/{key_id}/grants", false),
    (
        HttpMethod::Delete,
        "/api-keys/{key_id}/grants/{grant_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/projects/{project_slug}/grants",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/projects/{project_slug}/grants",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}",
        false,
    ),
    (HttpMethod::Post, "/workspaces/{ws}/grants", true),
    (HttpMethod::Get, "/workspaces/{ws}/grants", false),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/grants/{grant_id}",
        false,
    ),
    (HttpMethod::Post, "/workspaces/{ws}/groups", true),
    (HttpMethod::Get, "/workspaces/{ws}/groups", false),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/groups/{group_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/groups/{group_id}/members",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/groups/{group_id}/members",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/groups/{group_id}/members/{user_id}",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/audit", false),
    (HttpMethod::Post, "/auth/login", false),
    (HttpMethod::Get, "/activate/{token}", false),
    (HttpMethod::Post, "/activate/{token}", false),
    (HttpMethod::Get, "/admin/trash", false),
    (HttpMethod::Post, "/admin/trash/restore", false),
    (HttpMethod::Post, "/admin/trash/purge", true),
    (HttpMethod::Get, "/admin/trash/purges/{operation_id}", false),
    (HttpMethod::Get, "/workspaces", false),
    (HttpMethod::Post, "/workspaces", true),
    (HttpMethod::Get, "/workspaces/{ws}", false),
    (HttpMethod::Patch, "/workspaces/{ws}", false),
    (HttpMethod::Get, "/admin/workspaces", false),
    (HttpMethod::Patch, "/admin/workspaces/{ws}", false),
    (HttpMethod::Delete, "/admin/workspaces/{ws}", false),
    (HttpMethod::Get, "/admin/status-templates", false),
    (HttpMethod::Post, "/admin/status-templates", true),
    (
        HttpMethod::Patch,
        "/admin/status-templates/{template_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/admin/status-templates/{template_id}",
        false,
    ),
    (HttpMethod::Post, "/workspaces/{ws}/projects", true),
    (HttpMethod::Get, "/workspaces/{ws}/projects", false),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/projects/{project_slug}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/projects/{project_slug}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/projects/{project_slug}",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/members", false),
    (HttpMethod::Post, "/workspaces/{ws}/members", true),
    (HttpMethod::Get, "/workspaces/{ws}/assignable-users", false),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/members/{user_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/members/{user_id}",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/tags", false),
    (HttpMethod::Post, "/workspaces/{ws}/tags", true),
    (HttpMethod::Get, "/workspaces/{ws}/tags/used", false),
    (HttpMethod::Patch, "/workspaces/{ws}/tags/{tag_id}", false),
    (HttpMethod::Delete, "/workspaces/{ws}/tags/{tag_id}", false),
    (HttpMethod::Get, "/workspaces/{ws}/status-templates", false),
    (HttpMethod::Post, "/workspaces/{ws}/status-templates", true),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/status-templates/{template_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/status-templates/{template_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/apply-status-templates",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/property-definitions",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/property-definitions",
        true,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/property-definitions/{property_definition_id}",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/saved-searches", false),
    (HttpMethod::Post, "/workspaces/{ws}/saved-searches", true),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/saved-searches/{id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/saved-searches/{id}",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/task-views", false),
    (HttpMethod::Post, "/workspaces/{ws}/task-views", true),
    (HttpMethod::Get, "/workspaces/{ws}/task-views/{id}", false),
    (HttpMethod::Patch, "/workspaces/{ws}/task-views/{id}", false),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/task-views/{id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/projects/{project_slug}/boards",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/projects/{project_slug}/boards",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/boards/{board_id}", false),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/boards/{board_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/boards/{board_id}/move",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/archive",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/unarchive",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/columns",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/boards/{board_id}/columns",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}/columns/{column_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/tasks",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/boards/{board_id}/tasks",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/presence",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}/presence",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/presence",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/presence",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/tasks", false),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/tasks/{readable_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/move",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/assignees",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/assignees",
        true,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/references",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/references",
        true,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/references/batch",
        true,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/attachments",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/attachments",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts",
        true,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/backlinks",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/graph",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/checklist",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/checklist",
        true,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/subtasks",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/subtasks",
        true,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/promote",
        true,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/parent",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/activity",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/tasks/{readable_id}/comments",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/tasks/{readable_id}/comments",
        true,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/activity", false),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/projects/{project_slug}/documents",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/projects/{project_slug}/documents",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/documents/{slug}", false),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/documents/{slug}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}",
        false,
    ),
    (
        HttpMethod::Put,
        "/workspaces/{ws}/documents/{slug}/content",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/compact",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/content/range",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/documents/{slug}/content/range",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/content/search",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/history",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/revisions/{seq}",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/backlinks",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/frontmatter",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/attachments",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/attachments",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/attachments", false),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comment-drafts",
        true,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/documents/{slug}/move",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/moves/batch",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/copy",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/documents/{slug}/comments",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/comments",
        true,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/projects/{project_slug}/folders",
        true,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/projects/{project_slug}/folders",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/folders/{folder_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/folders/{folder_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/folders/{folder_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/folders/{folder_id}/move",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/folders/{folder_id}/copy",
        true,
    ),
    (HttpMethod::Post, "/workspaces/{ws}/webhooks", false),
    (HttpMethod::Get, "/workspaces/{ws}/webhooks", false),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/webhooks/{webhook_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/webhooks/{webhook_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/webhooks/{webhook_id}",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/webhooks/{webhook_id}/deliveries",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/integration-configs",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs",
        false,
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs/{config_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/integration-configs/{config_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/integration-configs/{config_id}",
        false,
    ),
    (HttpMethod::Post, "/workspaces/{ws}/automation-rules", true),
    (HttpMethod::Get, "/workspaces/{ws}/automation-rules", false),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        false,
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        false,
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        false,
    ),
    (HttpMethod::Get, "/workspaces/{ws}/events", false),
    (HttpMethod::Get, "/workspaces/{ws}/search", false),
    (HttpMethod::Get, "/workspaces/{ws}/semantic-search", false),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/semantic-search/reindex",
        false,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/semantic-search/reindex",
        true,
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/integrations/{integration}/events",
        false,
    ),
];

fn all_declared_routes() -> Vec<(HttpMethod, String, bool)> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut routes = Vec::new();
    for component in registry.entries() {
        for route in &component.api.routes {
            routes.push((
                route.method,
                route.path.as_str().to_string(),
                route.idempotent,
            ));
        }
    }
    routes
}

#[test]
fn table_is_exhaustive_over_reg5() {
    let live = all_declared_routes();
    assert_eq!(
        live.len(),
        212,
        "reg5.rs must declare exactly 212 routes; the classification table below assumes this"
    );
    assert_eq!(
        EXPECTED_IDEMPOTENT.len(),
        212,
        "EXPECTED_IDEMPOTENT must cover all 212 reg5.rs entries, not a sample"
    );

    let live_keys: std::collections::HashSet<(HttpMethod, &str)> = live
        .iter()
        .map(|(method, path, _)| (*method, path.as_str()))
        .collect();
    let table_keys: std::collections::HashSet<(HttpMethod, &str)> = EXPECTED_IDEMPOTENT
        .iter()
        .map(|(method, path, _)| (*method, *path))
        .collect();

    let missing_from_table: Vec<_> = live_keys.difference(&table_keys).collect();
    let missing_from_reg5: Vec<_> = table_keys.difference(&live_keys).collect();

    assert!(
        missing_from_table.is_empty(),
        "reg5.rs declares routes absent from EXPECTED_IDEMPOTENT: {missing_from_table:?}"
    );
    assert!(
        missing_from_reg5.is_empty(),
        "EXPECTED_IDEMPOTENT names routes reg5.rs no longer declares: {missing_from_reg5:?}"
    );
}

/// Names every `(method, path)` whose live `reg5.rs` value disagrees with
/// `EXPECTED_IDEMPOTENT`. Extracted from the test body so T4.17's adversarial
/// proof can call it against a deliberately-mutated copy of `routes` without
/// touching `reg5.rs` itself.
fn find_rule_mismatches(routes: &[(HttpMethod, String, bool)]) -> Vec<String> {
    let mut offenders = Vec::new();
    for (method, path, actual) in routes {
        let expected = EXPECTED_IDEMPOTENT
            .iter()
            .find(|(m, p, _)| m == method && *p == path)
            .map(|(_, _, expected)| *expected)
            .unwrap_or_else(|| panic!("no EXPECTED_IDEMPOTENT entry for {method:?} {path}"));

        if expected != *actual {
            offenders.push(format!(
                "{method:?} {path}: reg5.rs has idempotent={actual}, rule says {expected}"
            ));
        }
    }
    offenders
}

#[test]
fn every_reg5_idempotent_value_matches_the_written_rule() {
    let live = all_declared_routes();
    let offenders = find_rule_mismatches(&live);

    assert!(
        offenders.is_empty(),
        "the following entries disagree with the written D8 rule:\n{}",
        offenders.join("\n")
    );
}

/// T4.17: the exhaustive test has teeth. Flipping one route's value in a
/// local copy of the live data (never `reg5.rs` itself — no revert needed)
/// must produce exactly one named offender, not a silent pass.
#[test]
fn a_deliberate_mismatch_is_caught_and_named() {
    let mut live = all_declared_routes();

    let target = live
        .iter_mut()
        .find(|(method, path, _)| {
            *method == HttpMethod::Post && path == "/workspaces" // create_workspace, idempotent: true
        })
        .expect("create_workspace must be present in reg5.rs");
    assert!(target.2, "create_workspace must be true before the flip");
    target.2 = false;

    let offenders = find_rule_mismatches(&live);

    assert_eq!(
        offenders.len(),
        1,
        "flipping exactly one entry must produce exactly one offender: {offenders:?}"
    );
    assert!(
        offenders[0].contains("/workspaces") && offenders[0].contains("Post"),
        "the offender must name the exact flipped route: {offenders:?}"
    );
}

#[test]
fn true_and_false_counts_match_the_pr4_grounding() {
    let live = all_declared_routes();
    let true_count = live.iter().filter(|(_, _, idempotent)| *idempotent).count();
    let false_count = live.len() - true_count;

    // `v2-e3-s4` D3 added two more `idempotent: false` entries
    // (`/openapi.json`, `/scalar`), so the pre-S4 176 grew to 178; the
    // `true` count (34) is unaffected — neither new route is a POST.
    assert_eq!(true_count, 34, "expected exactly 34 idempotent:true routes");
    assert_eq!(
        false_count, 178,
        "expected exactly 178 idempotent:false routes"
    );
}
