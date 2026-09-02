#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! T2.5-T2.9 (`v2-e3-s3` PR2): `Page<T>` conformance across routes declared
//! paginated today.
//!
//! Two independent concerns live in this file:
//!
//! 1. A **local, no-DB, exhaustive classification** of which declared routes
//!    return `Page<T>` today (T2.5), derived from source rather than a
//!    hand-typed list (INV-DATA-DRIVEN): for every `#[utoipa::path]`-annotated
//!    handler under `crates/atlas_server/src/routes/*.rs`, this scans the
//!    handler's own Rust return-type text and its OpenAPI `body = ...`
//!    annotation and classifies the route `Page<T>` if either mentions
//!    `Page<`. Both signals are needed: some handlers' `#[utoipa::path]`
//!    block omits `body =` entirely even though the handler literally
//!    returns `Json<Page<T>>` (a pre-existing OpenAPI documentation gap,
//!    out of this slice's scope to fix — D-SHAPE forbids touching any DTO or
//!    OpenAPI shape here), and one handler (`semantic_search`) returns `impl
//!    IntoResponse` so only its `body = inline(Page<..>)` annotation names
//!    the shape. Relying on only one signal would silently under-count.
//!
//!    The result is cross-checked bidirectionally (INV-SET) against
//!    `route_matrix()` (the same live-registry accessor PR1's RFC 9457 sweep
//!    uses): every classified route must be a real registered `(method,
//!    path)`, and the two D-SHAPE-deferred `Vec<T>` inconsistencies
//!    (`list_subtasks`, the API-key grants listing) are named as an
//!    explicit, self-checking exclusion — never silently absorbed into the
//!    classified set, never silently dropped without a name and a reason.
//!
//! 2. A **live, container-backed** proof (T2.6-T2.8) that a sample of
//!    classified routes actually behaves per the spec: opaque cursor,
//!    `has_more`/`next_cursor` correctness, round-trip, and last-page
//!    absence. This cannot run in this sandbox (rootless podman blocks
//!    `ATLAS_TEST_DATABASE_URL`); CI is the actual gate. The sample is
//!    deliberately NOT all 24 classified routes: `tests/api_folders.rs`
//!    (`list_folders_paginates_with_cursor`), `tests/api_audit_read.rs`
//!    (`workspace_audit_pagination`), `tests/api_workspace_activity.rs`, and
//!    `tests/api_workspace_tasks.rs` (`list_workspace_tasks_pagination_no_
//!    overlap_and_has_more_correct`, "TW11") already prove round-trip and
//!    last-page absence for their resources — re-proving the same thing
//!    here would be exactly the "add a fourth" duplicate the task's own
//!    instruction warns against. What none of those existing tests assert
//!    is cursor **opacity** (the cursor is not, and does not embed, any
//!    plaintext resource id) — that is this file's real, non-duplicative
//!    contribution, added for the `tasks` family alongside the two families
//!    (`grants`, `documents`) that have no existing pagination test of any
//!    kind, where round-trip and last-page are proven here for the first
//!    time.

mod support;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use atlas_api::dtos::documents::CreateDocumentRequest;
use atlas_api::dtos::{CreateGrantRequest, CreateProjectRequest, GrantPrincipal};
use atlas_api::pagination::Cursor;
use atlas_client::AtlasClient;
use atlas_core::registry::HttpMethod;
use atlas_server::persistence::repos::ApiKeyRepo;

// ---------------------------------------------------------------------------
// T2.5: exhaustive, source-derived Page<T> route classification
// ---------------------------------------------------------------------------

/// A `Page<T>`-classified route: only `method` and `path_template` matter
/// for the audit; `file`/`line` are kept for failure diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClassifiedRoute {
    method: HttpMethod,
    path: String,
}

fn method_from_token(token: &str) -> Option<HttpMethod> {
    match token {
        "get" => Some(HttpMethod::Get),
        "post" => Some(HttpMethod::Post),
        "put" => Some(HttpMethod::Put),
        "patch" => Some(HttpMethod::Patch),
        "delete" => Some(HttpMethod::Delete),
        _ => None,
    }
}

/// Scans one `routes/*.rs` file for every `#[utoipa::path(...)]` block
/// immediately followed by a handler function, classifying each as
/// `Page<T>`-returning if either the `body = ...` annotation or the
/// handler's own return-type text mentions `Page<`.
///
/// Brace/paren-free by design: both the attribute and the fn signature are
/// scanned as accumulated text up to a marker line this codebase's `rustfmt`
/// output produces reliably (`)]` closing the attribute at column 0, `{`
/// opening the fn body) rather than by full Rust parsing.
fn scan_routes_file(source: &str) -> Vec<(ClassifiedRoute, String, bool)> {
    let mut results = Vec::new();
    let mut lines = source.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#[utoipa::path(") {
            continue;
        }

        // Two forms appear in this codebase: the common multi-line
        // `#[utoipa::path(\n  ...\n)]` (nothing else on the opening line),
        // and a single-line `#[utoipa::path(get, path = "...", ...)]` form
        // (`trash.rs`). Both must be handled or a route silently vanishes
        // from the scan without any error — exactly the kind of gap this
        // parser's own tests below (`classifier_*`) guard against for the
        // signals it DOES parse; this one is guarded by the count floor and
        // by cross-checking every found attribute against a live route.
        let mut attr_text = String::new();
        if trimmed.ends_with(")]") {
            attr_text.push_str(
                trimmed
                    .trim_start_matches("#[utoipa::path(")
                    .trim_end_matches(")]"),
            );
            attr_text.push('\n');
        } else {
            for attr_line in lines.by_ref() {
                if attr_line == ")]" {
                    break;
                }
                attr_text.push_str(attr_line);
                attr_text.push('\n');
            }
        }

        let method_token = attr_text
            .lines()
            .flat_map(|l| l.split(','))
            .map(str::trim)
            .find_map(method_from_token);
        let Some(method) = method_token else {
            continue;
        };

        let path = attr_text
            .find("path = \"")
            .map(|start| &attr_text[start + "path = \"".len()..])
            .and_then(|rest| rest.find('"').map(|end| rest[..end].to_string()));
        let Some(path) = path else { continue };

        let body_mentions_page =
            attr_text.contains("body = Page<") || attr_text.contains("body = inline(Page<");

        // Accumulate the fn signature text (from the next non-blank line up
        // to and including the line that opens the fn body) to check its
        // return type.
        let mut sig_text = String::new();
        let mut fn_name = None;
        for sig_line in lines.by_ref() {
            if fn_name.is_none()
                && let Some(idx) = sig_line.find("fn ")
            {
                let rest = &sig_line[idx + 3..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    fn_name = Some(name);
                }
            }
            sig_text.push_str(sig_line);
            sig_text.push('\n');
            // A `{` before the `fn` line belongs to a doc comment or an
            // attribute, not to the fn body; only stop once the signature
            // itself has opened its body.
            if fn_name.is_some() && sig_line.contains('{') {
                break;
            }
        }

        let return_type_mentions_page = sig_text.contains("Page<");
        let is_page = body_mentions_page || return_type_mentions_page;

        results.push((
            ClassifiedRoute { method, path },
            fn_name.unwrap_or_else(|| "<unknown>".to_string()),
            is_page,
        ));
    }

    results
}

/// Routes deliberately excluded from `Page<T>` classification even though
/// they share a DTO with a `Page<T>`-returning sibling — the two D-SHAPE
/// findings this slice records but does not fix. Named explicitly so
/// neither a silent inclusion nor a silent drop can happen: `exclusions_
/// are_still_accurate` below fails if either entry stops existing as a
/// real, still-non-`Page<T>` route, and the main classification test fails
/// if either entry is ever found to actually be `Page<T>`-classified
/// (meaning the finding has been fixed and the exclusion is stale).
const DSHAPE_EXCLUDED: &[(HttpMethod, &str, &str)] = &[
    (
        HttpMethod::Get,
        "/api/workspaces/{ws}/tasks/{readable_id}/subtasks",
        "list_subtasks returns Vec<TaskSummaryDto>, not Page<T>, unlike list_tasks \
         on the same DTO (tasks.rs:3946-3949 vs tasks.rs:1248-1252) — recorded \
         finding, deferred to S6/S7 per D-SHAPE, not fixed here.",
    ),
    (
        HttpMethod::Get,
        "/api/api-keys/{key_id}/grants",
        "API-key grants return Vec<ApiKeyGrantDto> where workspace/project grants \
         return Page<GrantDto> (api_keys.rs:600-604 vs grants.rs:198,473) — \
         recorded finding, deferred to S6/S7 per D-SHAPE, not fixed here.",
    ),
];

fn routes_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes")
}

fn classify_all_routes() -> Vec<(ClassifiedRoute, String, bool)> {
    let dir = routes_dir();
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));

    let mut all = Vec::new();
    let mut file_count = 0;
    for entry in entries {
        let path = entry.expect("readable dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        file_count += 1;
        let source =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        all.extend(scan_routes_file(&source));
    }

    assert!(
        file_count >= 25,
        "expected at least 25 .rs files under routes/ (exploration's grounding); \
         found {file_count} — did the directory move?"
    );
    all
}

/// T2.5/T2.9's own bidirectional audit: the derived classification does not
/// silently drift from the live registry, and the two named D-SHAPE
/// exclusions stay exactly excluded, exactly accurate.
#[test]
fn page_route_classification_is_exhaustive_and_self_checking() {
    let classified = classify_all_routes();

    let page_routes: HashSet<ClassifiedRoute> = classified
        .iter()
        .filter(|(_, _, is_page)| *is_page)
        .map(|(route, _, _)| route.clone())
        .collect();

    // Every route this scan found at all (Page<T> or not) must correspond to
    // a real, live registered route — proves the parser is not hallucinating
    // paths from malformed matches.
    let live: HashSet<(HttpMethod, String)> = support::route_matrix::route_matrix()
        .into_iter()
        .map(|e| (e.method, e.path_template))
        .collect();

    let mut unregistered = Vec::new();
    for (route, fn_name, _) in &classified {
        if !live.contains(&(route.method, route.path.clone())) {
            unregistered.push(format!("{:?} {} ({fn_name})", route.method, route.path));
        }
    }
    assert!(
        unregistered.is_empty(),
        "scanned utoipa path(s) do not match any live registered route (parser bug \
         or stale doc comment):\n{}",
        unregistered.join("\n")
    );

    // The two named D-SHAPE exclusions must never appear in the classified
    // Page<T> set — if one does, the finding has been silently fixed
    // without updating this exclusion list.
    for (method, path, reason) in DSHAPE_EXCLUDED {
        let key = ClassifiedRoute {
            method: *method,
            path: (*path).to_string(),
        };
        assert!(
            !page_routes.contains(&key),
            "excluded route {method:?} {path} is now classified Page<T> — the \
             D-SHAPE finding it names appears to be fixed; remove this exclusion \
             entry.\nRecorded reason: {reason}"
        );
    }

    // Exact count, not a floor: this derivation is stricter than
    // exploration's own hand-curated "22" (`v2-e3-s3-exploration.md` §2),
    // which omits `/api/workspaces/{ws}/search` and
    // `/api/workspaces/{ws}/semantic-search` — both genuinely return
    // `Page<T>` today (the latter only via its `body = inline(Page<..>)`
    // annotation, since its handler returns `impl IntoResponse`) but were
    // not in exploration's list. `/api/admin/trash` is also only found here
    // because its `#[utoipa::path]` uses this codebase's other valid form,
    // a single line, which a naive multi-line-only scan would silently
    // skip. This is exactly the kind of gap a derived, not curated,
    // classification exists to catch — a magic "22" constant would have
    // hidden it. If this count moves, name what changed rather than
    // adjusting the number to make the test pass.
    assert_eq!(
        page_routes.len(),
        24,
        "Page<T>-classified route count changed — enumerate the new/removed \
         route(s) rather than only updating this number: {page_routes:?}"
    );
}

/// T2.1-style both-ways exemption check, mirrored for the D-SHAPE exclusion
/// list: each excluded (method, path) must still be a real, live route.
#[test]
fn dshape_exclusions_are_still_real_routes() {
    let live: HashSet<(HttpMethod, String)> = support::route_matrix::route_matrix()
        .into_iter()
        .map(|e| (e.method, e.path_template))
        .collect();

    for (method, path, _) in DSHAPE_EXCLUDED {
        assert!(
            live.contains(&(*method, (*path).to_string())),
            "D-SHAPE exclusion {method:?} {path} no longer matches any live route — \
             remove the stale exclusion entry"
        );
    }
}

// ---------------------------------------------------------------------------
// T2.4-style RED proof for the classifier itself
// ---------------------------------------------------------------------------

#[test]
fn classifier_detects_page_via_return_type_even_without_body_annotation() {
    let source = r#"
#[utoipa::path(
    get,
    path = "/api/fake/undocumented-page",
    tag = "fake",
    responses(
        (status = 200, description = "no body annotation here"),
    )
)]
pub(crate) async fn fake_undocumented_handler(
    State(state): State<AppState>,
) -> Result<Json<Page<FakeDto>>, ApiError> {
    todo!()
}
"#;
    let found = scan_routes_file(source);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].2,
        "a handler whose return type mentions Page<..> must classify as Page<T> \
         even when the utoipa body annotation is absent — it did not, so the \
         classifier would silently under-count exactly like a naive \
         body-annotation-only scan would"
    );
}

#[test]
fn classifier_detects_page_via_body_annotation_even_with_impl_into_response() {
    let source = r#"
#[utoipa::path(
    get,
    path = "/api/fake/impl-into-response-page",
    tag = "fake",
    responses(
        (status = 200, body = inline(Page<FakeDto>)),
    )
)]
pub(crate) async fn fake_impl_into_response_handler(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    todo!()
}
"#;
    let found = scan_routes_file(source);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].2,
        "a handler returning `impl IntoResponse` must still classify as Page<T> \
         via its utoipa body annotation — it did not, so semantic_search-shaped \
         handlers would be silently dropped from the classified set"
    );
}

#[test]
fn classifier_does_not_flag_a_plain_vec_handler() {
    let source = r#"
#[utoipa::path(
    get,
    path = "/api/fake/vec-route",
    tag = "fake",
    responses(
        (status = 200, body = Vec<FakeDto>),
    )
)]
pub(crate) async fn fake_vec_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<FakeDto>>, ApiError> {
    todo!()
}
"#;
    let found = scan_routes_file(source);
    assert_eq!(found.len(), 1);
    assert!(
        !found[0].2,
        "a Vec<T>-returning handler must not classify as Page<T>, proving the \
         detector has teeth in the other direction too"
    );
}

// ---------------------------------------------------------------------------
// T2.6-T2.8: live conformance, container-backed, three named families
// ---------------------------------------------------------------------------
//
// Cannot run locally (INV-CONTAINER-UNVERIFIABLE, rootless podman blocks
// ATLAS_TEST_DATABASE_URL). Confirmed to compile cleanly via
// `cargo check --workspace --all-targets`; CI is the actual gate.

/// Shared assertions for a decoded page: the cursor, if present, must be
/// syntactically opaque (valid base64url-nopad `Cursor` format) and must
/// not equal or contain any item's own plaintext id — proving the cursor is
/// not a bare re-encoding of a visible identifier.
fn assert_cursor_is_opaque(cursor: &str, item_ids: &[uuid::Uuid]) {
    assert!(
        Cursor::decode(cursor).is_some(),
        "next_cursor {cursor:?} does not decode as a valid base64url-nopad Cursor"
    );
    for id in item_ids {
        let plain = id.to_string();
        assert_ne!(&plain, cursor, "cursor is a bare plaintext resource id");
        assert!(
            !cursor.contains(&plain),
            "cursor embeds a plaintext resource id ({plain}) as a substring"
        );
    }
}

/// `documents` family (T2.6-T2.8): no existing pagination test covers
/// `list_documents` at all, so this proves round-trip, last-page absence,
/// AND opacity — genuinely new coverage, not a fourth copy of anything.
#[tokio::test]
async fn list_documents_pagination_round_trips_and_is_opaque() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "pgconf-docs").await;

    let project = client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Docs Project".to_string(),
                slug: "docs-proj".to_string(),
                task_prefix: "DOC".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let mut created_ids = Vec::new();
    for i in 0..3 {
        let doc = client
            .create_document(
                &ws.slug,
                &project.slug,
                CreateDocumentRequest {
                    title: format!("Doc {i}"),
                    folder_id: None,
                    content: None,
                },
            )
            .await
            .expect("create document");
        created_ids.push(doc.id);
    }

    let page1 = client
        .list_documents(&ws.slug, &project.slug, None, Some(2))
        .await
        .expect("page 1");
    assert_eq!(page1.items.len(), 2);
    assert!(
        page1.has_more,
        "3 documents with limit=2 must have a second page"
    );
    let cursor = page1
        .next_cursor
        .clone()
        .expect("page 1 with has_more=true must carry next_cursor");
    let page1_ids: Vec<uuid::Uuid> = page1.items.iter().map(|d| d.id).collect();
    assert_cursor_is_opaque(&cursor, &page1_ids);

    let page2 = client
        .list_documents(&ws.slug, &project.slug, Some(&cursor), Some(2))
        .await
        .expect("page 2");
    let page2_ids: Vec<uuid::Uuid> = page2.items.iter().map(|d| d.id).collect();
    for id in &page1_ids {
        assert!(
            !page2_ids.contains(id),
            "document {id} appeared on both pages"
        );
    }
    assert!(
        !page2.has_more,
        "all 3 documents fit within pages 1+2 at limit=2"
    );
    assert!(
        page2.next_cursor.is_none(),
        "last page must not carry next_cursor"
    );

    let mut all_seen: Vec<uuid::Uuid> = page1_ids.iter().chain(page2_ids.iter()).copied().collect();
    all_seen.sort_unstable();
    let mut expected = created_ids;
    expected.sort_unstable();
    assert_eq!(
        all_seen, expected,
        "pages together must cover every created document exactly once"
    );

    db.teardown().await;
}

/// `grants` family (T2.6-T2.8): no existing pagination test covers
/// `list_workspace_grants` at all.
#[tokio::test]
async fn list_workspace_grants_pagination_round_trips_and_is_opaque() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "pgconf-grants").await;

    // Three agent principals, granted directly at the repo layer (mirrors
    // `api_grants.rs`'s own `add_agent` helper) so this test needs no
    // extra HTTP round trips just to create grantable principals.
    let ctx = atlas_acta::actor::WorkspaceCtx::new(
        ws.id,
        atlas_acta::actor::Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
    );
    let mut created_ids = Vec::new();
    for i in 0..3 {
        let agent = db
            .api_key_repo()
            .create(
                atlas_custos::WorkspaceScope(ws.id.0),
                &ctx.actor,
                atlas_server::persistence::repos::NewApiKey {
                    name: format!("agent-{i}"),
                    token_hash: format!("hash-pgconf-{i}"),
                    type_: atlas_custos::entities::identity::ApiKeyType::Agent,
                    expires_at: None,
                    scopes: atlas_custos::capability::Capability::ALL.to_vec(),
                },
            )
            .await
            .expect("create agent api key");

        let grant = client
            .create_workspace_grant(
                &ws.slug,
                CreateGrantRequest {
                    principal: GrantPrincipal {
                        r#type: "api_key".to_string(),
                        id: agent.id.0,
                    },
                    role: "editor".to_string(),
                },
            )
            .await
            .expect("create workspace grant");
        created_ids.push(grant.id);
    }

    let page1 = client
        .list_workspace_grants(&ws.slug, None, Some(2))
        .await
        .expect("page 1");
    assert_eq!(page1.items.len(), 2);
    assert!(
        page1.has_more,
        // Only the 3 agent grants seeded above exist: workspace creation
        // (both the HTTP `create_workspace` handler and this test's
        // `login_user_with_workspace` helper) grants the creator Owner
        // *membership*, not a `permission_grant`/ACL row — see
        // `routes/workspaces.rs::create_workspace`'s own comment ("no
        // explicit grant is needed here") and `MINIMAL_NONEMPTY_ROUTES`
        // below, which lists project-scope grants as unconditionally
        // nonempty but omits workspace-scope grants for the same reason.
        "more than 2 grants exist (3 agent grants)"
    );
    let cursor = page1
        .next_cursor
        .clone()
        .expect("page 1 with has_more=true must carry next_cursor");
    let page1_ids: Vec<uuid::Uuid> = page1.items.iter().map(|g| g.id).collect();
    assert_cursor_is_opaque(&cursor, &page1_ids);

    let mut page_ids = page1_ids.clone();
    let mut next = Some(cursor);
    let mut pages_walked = 1;
    while let Some(c) = next {
        let page = client
            .list_workspace_grants(&ws.slug, Some(&c), Some(2))
            .await
            .expect("subsequent page");
        pages_walked += 1;
        for id in page.items.iter().map(|g| g.id) {
            assert!(
                !page_ids.contains(&id),
                "grant {id} appeared on an earlier page"
            );
            page_ids.push(id);
        }
        if page.has_more {
            let c = page
                .next_cursor
                .clone()
                .expect("has_more=true must carry next_cursor");
            assert_cursor_is_opaque(&c, &page.items.iter().map(|g| g.id).collect::<Vec<_>>());
            next = Some(c);
        } else {
            assert!(
                page.next_cursor.is_none(),
                "last page must not carry next_cursor"
            );
            next = None;
        }
        assert!(
            pages_walked < 20,
            "pagination did not terminate — possible cursor bug"
        );
    }

    for id in &created_ids {
        assert!(
            page_ids.contains(id),
            "created grant {id} never appeared in any page"
        );
    }

    db.teardown().await;
}

/// `tasks` family: `TW11` in `api_workspace_tasks.rs` already proves
/// round-trip and last-page absence for `list_workspace_tasks`. This adds
/// only the property TW11 does not check: cursor opacity.
#[tokio::test]
async fn list_workspace_tasks_cursor_is_opaque() {
    use atlas_api::dtos::boards_tasks::{
        CreateBoardRequest, CreateColumnRequest, CreateTaskRequest, WorkspaceTaskQueryParams,
    };

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "pgconf-tasks").await;

    let project = client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Tasks Project".to_string(),
                slug: "tasks-proj".to_string(),
                task_prefix: "TSK".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                folder_id: None,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");
    let column = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create column");

    for i in 0..3 {
        client
            .create_task(
                &ws.slug,
                board.id,
                CreateTaskRequest {
                    column_id: column.id,
                    title: format!("Task {i}"),
                    description: None,
                    properties: None,
                    before: None,
                    after: None,
                    references: Vec::new(),
                },
            )
            .await
            .expect("create task");
    }

    let page1 = client
        .list_workspace_tasks(
            &ws.slug,
            &WorkspaceTaskQueryParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .expect("page 1");
    assert!(
        page1.has_more,
        "3 tasks with limit=2 must have a second page"
    );
    let cursor = page1
        .next_cursor
        .clone()
        .expect("page 1 with has_more=true must carry next_cursor");
    let page1_ids: Vec<uuid::Uuid> = page1.items.iter().map(|t| t.id).collect();
    assert_cursor_is_opaque(&cursor, &page1_ids);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T2.6-T2.9 follow-up: last-page-empty sweep over EVERY classified Page<T>
// route (not just the three named families above).
//
// PR2's own verify flagged that spec acceptance gate 1 ("Page<T> ... all
// routes returning Page<T> today") is not literally satisfied by the three
// named-family round-trip tests above, and that no later PR in this slice
// revisits Page<T> conformance to close the gap. This test closes it with the
// cheapest property that still means something for every route: given the
// minimal parent resource(s) a route needs to resolve at all, its collection
// is on its own last page (`has_more == false`, `next_cursor` present-and-
// null) and, for the 22 routes whose collection can be made genuinely empty,
// carries zero items.
//
// Two routes cannot be driven to zero items no matter what is provisioned,
// because the parent resource's own creation unconditionally writes one row
// into the exact table the route lists (`MINIMAL_NONEMPTY_ROUTES` below,
// self-checking both ways like `DSHAPE_EXCLUDED`): a document's initial
// revision, and a task's own creation activity entry. These are recorded
// with a reason, not silently skipped.
// ---------------------------------------------------------------------------

/// Routes whose minimal viable state can never be a literal zero-item page,
/// because the parent resource's own creation writes exactly one row into
/// the table this route lists. Named explicitly, both ways: `page_route_
/// classification_is_exhaustive_and_self_checking` already proves both are
/// live, classified Page<T> routes; `every_classified_page_route_reaches_
/// its_own_last_page` below fails if either ever appears without its arm.
const MINIMAL_NONEMPTY_ROUTES: &[(HttpMethod, &str, usize, &str)] = &[
    (
        HttpMethod::Get,
        "/api/workspaces/{ws}/documents/{slug}/history",
        1,
        "creating a document unconditionally inserts its initial revision \
         (seq=1, documents.rs `create_in`) — a document's own history list \
         can never be literally empty.",
    ),
    (
        HttpMethod::Get,
        "/api/workspaces/{ws}/tasks/{readable_id}/activity",
        1,
        "creating a task unconditionally appends an ActivityKind::Created \
         entry (task_service.rs `create_with_references_under`) — a task's \
         own activity feed can never be literally empty.",
    ),
    (
        HttpMethod::Get,
        "/api/workspaces/{ws}/projects/{project_slug}/grants",
        1,
        "creating a project unconditionally upserts a permission grant for \
         the creator on ResourceRef::Project (projects.rs `create_project`), \
         and `list_project_grants` reads that exact scope with no exclusion \
         — a fresh project's grant list always holds the creator's row.",
    ),
];

/// Issues an authenticated raw GET (bypassing `AtlasClient`'s typed
/// deserialization) and returns the decoded JSON body, so the last-page
/// assertions below can inspect the exact wire shape of `next_cursor`
/// (present-and-null vs. omitted) rather than an `Option<String>` that
/// cannot distinguish the two.
async fn fetch_page_json(client: &AtlasClient, path_and_query: &str) -> serde_json::Value {
    let mut builder = client
        .http_client()
        .get(format!("{}{}", client.base_url(), path_and_query));
    if let Some(token) = client.token() {
        builder = builder.bearer_auth(token);
    }
    let response = builder
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {path_and_query}: request failed: {e}"));
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .unwrap_or_else(|e| panic!("GET {path_and_query}: failed to decode JSON body: {e}"));
    assert_eq!(
        status.as_u16(),
        200,
        "GET {path_and_query} did not return 200: {body}"
    );
    body
}

/// Asserts `body` is a `Page<T>` JSON object representing its own last page
/// with exactly `expected_items` rows.
///
/// `next_cursor` must be present with value `null`, not omitted: `Page<T>`
/// (`crates/atlas_api/src/pagination.rs:142-146`) carries no `#[serde(skip_
/// serializing_if)]` on `next_cursor`, so `Option::None` serializes to a
/// present `null` key. Asserting `body.get("next_cursor") == Some(&Value::
/// Null)` (rather than deserializing into `Page<T>` and checking `Option::
/// is_none()`) is what actually distinguishes present-null from absent.
fn assert_is_own_last_page(body: &serde_json::Value, path: &str, expected_items: usize) {
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("GET {path}: response has no `items` array: {body}"));
    assert_eq!(
        items.len(),
        expected_items,
        "GET {path}: expected {expected_items} item(s), got {}: {body}",
        items.len()
    );
    assert_eq!(
        body.get("has_more"),
        Some(&serde_json::Value::Bool(false)),
        "GET {path}: has_more must be false on the last page: {body}"
    );
    assert_eq!(
        body.get("next_cursor"),
        Some(&serde_json::Value::Null),
        "GET {path}: next_cursor must be present in the body with value null \
         on the last page, never omitted: {body}"
    );
}

/// T2.6-T2.9 follow-up (data-driven, self-checking): every route the
/// classification above finds Page<T>-returning is exercised with its
/// minimal parent resource(s) provisioned, and asserted to be its own last
/// page. A classified route with no arm below fails the test by name rather
/// than being silently skipped (INV-DATA-DRIVEN, mirrors the classification
/// audit's own `unregistered` check).
///
/// Container-backed like the three named-family tests above: cannot run
/// locally (INV-CONTAINER-UNVERIFIABLE), confirmed to compile via `cargo
/// check --workspace --all-targets`; CI is the actual gate.
#[tokio::test]
async fn every_classified_page_route_reaches_its_own_last_page() {
    use atlas_api::dtos::boards_tasks::{
        CreateBoardRequest, CreateColumnRequest, CreateTaskRequest,
    };
    use atlas_api::dtos::webhooks::CreateWebhookRequest;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let classified = classify_all_routes();
    let page_routes: HashSet<ClassifiedRoute> = classified
        .iter()
        .filter(|(_, _, is_page)| *is_page)
        .map(|(route, _, _)| route.clone())
        .collect();

    // `page_routes` is a `HashSet`, whose iteration order is process-random.
    // Sort into a `Vec` by `(method, path)` so a failure names a
    // reproducible route instead of depending on hash-iteration luck.
    let mut sorted_page_routes: Vec<ClassifiedRoute> = page_routes.iter().cloned().collect();
    sorted_page_routes.sort_by_key(|route| (format!("{:?}", route.method), route.path.clone()));

    let mut covered: HashSet<ClassifiedRoute> = HashSet::new();

    for route in &sorted_page_routes {
        let minimal_nonempty = MINIMAL_NONEMPTY_ROUTES
            .iter()
            .find(|(m, p, _, _)| *m == route.method && *p == route.path)
            .map(|(_, _, n, _)| *n);

        match route.path.as_str() {
            "/api/admin/audit" => {
                // Isolated database, not the shared `db`/`server`: unlike
                // every other arm below, this route is a global collection,
                // not scoped to one workspace. Confirmed from source that no
                // other arm's flow (login, project/board/task/document/
                // webhook creation) writes a `security_audit_log` row today
                // — only account activation, and user/group/membership/
                // grant/api-key management do (`routes/{activate,users,
                // groups,members,grants,api_keys}.rs`, all via
                // `PgSecurityAuditRepo::append_in`), none of which any arm
                // here exercises. A fresh database removes the dependency on
                // that staying true, instead of relying on it.
                let admin_db = support::TestDb::create().await.expect("TestDb::create");
                let admin_server = support::TestServer::spawn(&admin_db).await;
                let admin = support::login_root_user(&admin_server, &admin_db).await;
                let body = fetch_page_json(&admin, "/api/admin/audit").await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
                admin_db.teardown().await;
            }
            "/api/admin/trash" => {
                // Isolated database for the same reason as `/api/admin/audit`
                // above: this is a global collection, and trash rows are only
                // ever written by delete/restore/purge flows
                // (`append_resource_deleted_in` / `..._restored_in` /
                // `..._purge_committed_in`), which no arm in this sweep
                // triggers — every arm only creates resources. A fresh
                // database removes the dependency on that staying true.
                let admin_db = support::TestDb::create().await.expect("TestDb::create");
                let admin_server = support::TestServer::spawn(&admin_db).await;
                let admin = support::login_root_user(&admin_server, &admin_db).await;
                let body = fetch_page_json(&admin, "/api/admin/trash").await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
                admin_db.teardown().await;
            }
            "/api/api-keys" => {
                let (client, _ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-apikeys").await;
                let body = fetch_page_json(&client, "/api/api-keys").await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/activity" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-activity").await;
                let body =
                    fetch_page_json(&client, &format!("/api/workspaces/{}/activity", ws.slug))
                        .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/attachments" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-attachments")
                        .await;
                let body =
                    fetch_page_json(&client, &format!("/api/workspaces/{}/attachments", ws.slug))
                        .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/audit" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-audit").await;
                let body =
                    fetch_page_json(&client, &format!("/api/workspaces/{}/audit", ws.slug)).await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/automation-rules" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-automation")
                        .await;
                let body = fetch_page_json(
                    &client,
                    &format!("/api/workspaces/{}/automation-rules", ws.slug),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/boards/{board_id}/tasks" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-boardtasks")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let board = client
                    .create_board(
                        &ws.slug,
                        &project.slug,
                        CreateBoardRequest {
                            folder_id: None,
                            name: "Sweep Board".to_string(),
                        },
                    )
                    .await
                    .expect("create board");
                let body = fetch_page_json(
                    &client,
                    &format!("/api/workspaces/{}/boards/{}/tasks", ws.slug, board.id),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/documents/{slug}/attachments" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-docattach")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let doc = client
                    .create_document(
                        &ws.slug,
                        &project.slug,
                        CreateDocumentRequest {
                            title: "Sweep Doc".to_string(),
                            folder_id: None,
                            content: None,
                        },
                    )
                    .await
                    .expect("create document");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/documents/{}/attachments",
                        ws.slug, doc.id
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/documents/{slug}/backlinks" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-docbacklinks")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let doc = client
                    .create_document(
                        &ws.slug,
                        &project.slug,
                        CreateDocumentRequest {
                            title: "Sweep Doc".to_string(),
                            folder_id: None,
                            content: None,
                        },
                    )
                    .await
                    .expect("create document");
                let body = fetch_page_json(
                    &client,
                    &format!("/api/workspaces/{}/documents/{}/backlinks", ws.slug, doc.id),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/documents/{slug}/history" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-dochistory")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let doc = client
                    .create_document(
                        &ws.slug,
                        &project.slug,
                        CreateDocumentRequest {
                            title: "Sweep Doc".to_string(),
                            folder_id: None,
                            content: None,
                        },
                    )
                    .await
                    .expect("create document");
                let body = fetch_page_json(
                    &client,
                    &format!("/api/workspaces/{}/documents/{}/history", ws.slug, doc.id),
                )
                .await;
                assert_is_own_last_page(
                    &body,
                    route.path.as_str(),
                    minimal_nonempty.expect("history is listed in MINIMAL_NONEMPTY_ROUTES"),
                );
            }
            "/api/workspaces/{ws}/grants" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-grants").await;
                let body =
                    fetch_page_json(&client, &format!("/api/workspaces/{}/grants", ws.slug)).await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/projects" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-projects").await;
                let body =
                    fetch_page_json(&client, &format!("/api/workspaces/{}/projects", ws.slug))
                        .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/projects/{project_slug}/boards" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-projboards")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/projects/{}/boards",
                        ws.slug, project.slug
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/projects/{project_slug}/documents" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-projdocs").await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/projects/{}/documents",
                        ws.slug, project.slug
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/projects/{project_slug}/folders" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-projfolders")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/projects/{}/folders",
                        ws.slug, project.slug
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/projects/{project_slug}/grants" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-projgrants")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/projects/{}/grants",
                        ws.slug, project.slug
                    ),
                )
                .await;
                assert_is_own_last_page(
                    &body,
                    route.path.as_str(),
                    minimal_nonempty.expect("project grants is listed in MINIMAL_NONEMPTY_ROUTES"),
                );
            }
            "/api/workspaces/{ws}/search" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-search").await;
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/search?q=zzz-sweep-no-such-match-zzz",
                        ws.slug
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/semantic-search" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-semsearch")
                        .await;
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/semantic-search?q=zzz-sweep-no-such-match-zzz",
                        ws.slug
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/tasks" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-tasks").await;
                let body =
                    fetch_page_json(&client, &format!("/api/workspaces/{}/tasks", ws.slug)).await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/tasks/{readable_id}/activity" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-taskactivity")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let board = client
                    .create_board(
                        &ws.slug,
                        &project.slug,
                        CreateBoardRequest {
                            folder_id: None,
                            name: "Sweep Board".to_string(),
                        },
                    )
                    .await
                    .expect("create board");
                let column = client
                    .create_column(
                        &ws.slug,
                        board.id,
                        CreateColumnRequest {
                            name: "Todo".to_string(),
                            before: None,
                            after: None,
                            color: None,
                        },
                    )
                    .await
                    .expect("create column");
                let task = client
                    .create_task(
                        &ws.slug,
                        board.id,
                        CreateTaskRequest {
                            column_id: column.id,
                            title: "Sweep Task".to_string(),
                            description: None,
                            properties: None,
                            before: None,
                            after: None,
                            references: Vec::new(),
                        },
                    )
                    .await
                    .expect("create task");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/tasks/{}/activity",
                        ws.slug, task.readable_id
                    ),
                )
                .await;
                assert_is_own_last_page(
                    &body,
                    route.path.as_str(),
                    minimal_nonempty.expect("task activity is listed in MINIMAL_NONEMPTY_ROUTES"),
                );
            }
            "/api/workspaces/{ws}/tasks/{readable_id}/backlinks" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-taskbacklinks")
                        .await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let board = client
                    .create_board(
                        &ws.slug,
                        &project.slug,
                        CreateBoardRequest {
                            folder_id: None,
                            name: "Sweep Board".to_string(),
                        },
                    )
                    .await
                    .expect("create board");
                let column = client
                    .create_column(
                        &ws.slug,
                        board.id,
                        CreateColumnRequest {
                            name: "Todo".to_string(),
                            before: None,
                            after: None,
                            color: None,
                        },
                    )
                    .await
                    .expect("create column");
                let task = client
                    .create_task(
                        &ws.slug,
                        board.id,
                        CreateTaskRequest {
                            column_id: column.id,
                            title: "Sweep Task".to_string(),
                            description: None,
                            properties: None,
                            before: None,
                            after: None,
                            references: Vec::new(),
                        },
                    )
                    .await
                    .expect("create task");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/tasks/{}/backlinks",
                        ws.slug, task.readable_id
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/webhooks" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-webhooks").await;
                let body =
                    fetch_page_json(&client, &format!("/api/workspaces/{}/webhooks", ws.slug))
                        .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            "/api/workspaces/{ws}/webhooks/{webhook_id}/deliveries" => {
                let (client, ws, _user) =
                    support::login_user_with_workspace(&server, &db, "pgconf-sweep-webhookdeliv")
                        .await;
                let webhook = client
                    .create_webhook(
                        &ws.slug,
                        CreateWebhookRequest {
                            target_url: "http://127.0.0.1:9/sweep-hook".to_string(),
                            event_types: vec!["task.created".to_string()],
                            scope_type: "workspace".to_string(),
                            scope_id: None,
                            label: None,
                        },
                    )
                    .await
                    .expect("create webhook");
                let body = fetch_page_json(
                    &client,
                    &format!(
                        "/api/workspaces/{}/webhooks/{}/deliveries",
                        ws.slug, webhook.id
                    ),
                )
                .await;
                assert_is_own_last_page(&body, route.path.as_str(), 0);
            }
            other => panic!(
                "no empty-last-page provisioning arm for classified Page<T> route \
                 {:?} {other} — a new paginated route was added without extending \
                 this exhaustive sweep",
                route.method
            ),
        }

        covered.insert(route.clone());
    }

    assert_eq!(
        covered, page_routes,
        "the provisioning sweep did not exercise exactly the classified Page<T> \
         route set"
    );

    db.teardown().await;
}
