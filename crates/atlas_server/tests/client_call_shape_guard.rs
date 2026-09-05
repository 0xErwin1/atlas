#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `v2-e11-s4` PR2, design D5 — proves the consumer-side migration to the
//! namespaced `AtlasClient` mechanically, for every call site in
//! `atlas_cli`, `atlas_mcp`, `atlas_client::helpers`, and
//! `atlas_server/tests`, instead of by sampling a diff.
//!
//! **Why a second, independent walker.** `atlas_client_route_contract.rs`'s
//! extractor reads exactly one file, `atlas_client/src/lib.rs` — it can
//! never see a consumer crate, so it cannot gate `atlas_cli`/`atlas_mcp`
//! call sites (spec ground truth §0.2). This module derives its own
//! method-to-namespace map from the same client source, independently of
//! that extractor's code (a bug in one must not silently mirror into the
//! other), then walks every consumer.
//!
//! **Forward check**: for every method the map assigns a home namespace,
//! count occurrences of `\.\s*<method>\s*\(` in each consumer's masked
//! source (comments and string-literal contents stripped by
//! [`support::scan`], so a doc comment or a help string mentioning a method
//! name is never counted) that are *not* immediately preceded by
//! `\.\s*(acta|custos|platform)\s*\(\s*\)\s*`. `\s*` spans newlines, so a
//! chained `client\n    .<method>(` site counts identically to a
//! single-line one. Every crate/file's count is pinned by **exact
//! equality** — pre-migration, every pin is the crate's or file's full flat
//! count; a later PR flips a pin to 0 as its file migrates.
//!
//! **Reverse check**: for every *namespaced* call site found in a real
//! consumer, the namespace used must equal the method's declared home. At
//! this PR's ground truth no namespace exists anywhere in the client's
//! public surface yet (PR3 introduces the first one), so this check runs
//! over real sources and finds zero namespaced sites — asserted explicitly,
//! not left as an absence of assertions.
//!
//! **Two named exceptions**: `login` and `health` never move to a
//! sub-client (design D1.2, D3, R10 — `login` mutates `self.token`, which a
//! `&self` sub-client cannot do; `health` targets no owning component on the
//! wire). Both are excluded from the derived map, so a flat
//! `client.login(...)`/`client.health()` call site is correct forever and
//! is never counted as a pending migration.
//!
//! The migration itself runs through an uncommitted throwaway rewriter
//! (design D5's closing paragraph); this file is the only committed proof.

mod support;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use support::scan::scan;

// ---------------------------------------------------------------------------
// Method -> namespace map, derived from `atlas_client`'s own source
// ---------------------------------------------------------------------------

/// The three sub-client namespaces a mapped method can call home.
const NAMESPACES: &[&str] = &["acta", "custos", "platform"];

/// Methods that keep their current flat call shape forever and are
/// therefore excluded from the derived map (design D1.2, D3, R10):
/// - `login` resolves to `Component::Custos` on the wire but mutates
///   `self.token`, an effect a `&self` sub-client cannot perform.
/// - `health` (`root_get`) targets no owning component on the wire at all;
///   it is naturally excluded because [`resolve_component`] never assigns
///   it a namespace, not because of this list, but it is named here too so
///   every named exception is visible in one place.
const NEVER_NAMESPACED: &[(&str, &str)] = &[(
    "login",
    "mutates self.token; a &self sub-client cannot perform that effect (D1.2)",
)];

fn client_lib_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../atlas_client/src/lib.rs")
}

fn client_src_path(file_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../atlas_client/src")
        .join(file_name)
}

/// `atlas_client/src/<file_name>`'s production source, with `#[cfg(test)]
/// mod tests { .. }` truncated off — the same scope
/// `atlas_client_route_contract.rs::production_source` uses, computed here
/// independently rather than shared, per design D5's "not a shared
/// function" instruction.
fn client_production_source(file_name: &str) -> String {
    let path = if file_name == "lib.rs" {
        client_lib_path()
    } else {
        client_src_path(file_name)
    };
    let content =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    match content.find("#[cfg(test)]") {
        Some(index) => content[..index].to_string(),
        None => content,
    }
}

/// `crates/atlas_client/src/*.rs` files that carry real method
/// implementations outside `lib.rs`, populated one at a time as PR3-PR5
/// split the client (`custos.rs` in PR3). Each one is walked in full by
/// [`derive_method_namespace_map`], on equal footing with `lib.rs`.
const SPLIT_PRODUCTION_FILES: &[&str] = &["custos.rs", "acta.rs", "platform.rs"];

/// The names of every `pub async fn` in `source` whose attribute block
/// carries `#[doc(hidden)]` (doc-comment lines may sit between the
/// attribute and the `fn`) — a D6 shim forwarder, not a real
/// implementation. Excluded from `lib.rs`'s own method population so a
/// method that moved to a split file is derived from its new home, not
/// double-counted (once as a real call there, once as a rootless forwarder
/// here). `#[doc(hidden)]` is the marker because the forwarders are not
/// `#[deprecated]`: this guard already pins every flat call site, so a new
/// flat call fails here without a deprecation warning, and a warning would
/// force `#![allow(deprecated)]` into every consumer crate under
/// `-D warnings`.
fn hidden_forwarder_names(source: &str) -> std::collections::HashSet<String> {
    let re = Regex::new(
        r"(?m)^[ \t]*#\[doc\(hidden\)\][ \t]*\r?\n(?:[ \t]*///[^\n]*\r?\n)*[ \t]*pub async fn (\w+)",
    )
    .unwrap();
    re.captures_iter(source)
        .map(|caps| caps[1].to_string())
        .collect()
}

/// (byte offset of the `fn` keyword, function name, whether it is `pub
/// async fn`), sorted by offset ascending.
struct FnBoundary {
    offset: usize,
    name: String,
    is_pub_async: bool,
}

fn function_boundaries(source: &str) -> Vec<FnBoundary> {
    let fn_re = Regex::new(r"(?m)^\s*(pub(?:\(crate\))? )?(async )?fn (\w+)").unwrap();
    fn_re
        .captures_iter(source)
        .map(|caps| {
            let whole = caps.get(0).unwrap();
            let is_pub_async = caps.get(1).is_some() && caps.get(2).is_some();
            FnBoundary {
                offset: whole.start(),
                name: caps[3].to_string(),
                is_pub_async,
            }
        })
        .collect()
}

fn function_body<'a>(source: &'a str, boundaries: &[FnBoundary], name: &str) -> Option<&'a str> {
    let index = boundaries.iter().position(|b| b.name == name)?;
    let start = boundaries.get(index)?.offset;
    let end = boundaries.get(index + 1).map_or(source.len(), |b| b.offset);
    Some(&source[start..end])
}

/// Resolves `name`'s home namespace by looking for its first direct
/// `self.<verb>(Component::X, ..)` call. `root_get` carries no component
/// and resolves to `None`. When `name`'s body has no direct verb call at
/// all (a pure delegating wrapper, e.g. `list_documents` calling
/// `list_documents_with_unfiled_filter`), recurses into the single
/// `self.<other_method>(..)` call found in its body, bounded to avoid an
/// infinite loop on a cyclic (buggy) delegation chain.
fn resolve_component(
    name: &str,
    boundaries: &[FnBoundary],
    source: &str,
    depth: usize,
) -> Option<&'static str> {
    if depth > 8 {
        return None;
    }

    let body = function_body(source, boundaries, name)?;

    let verb_re =
        Regex::new(r"self\s*\.\s*(get|post|patch|put|delete|root_get)\s*\(\s*(Component::(\w+))?")
            .unwrap();
    if let Some(caps) = verb_re.find(body).map(|_| verb_re.captures(body).unwrap()) {
        let verb = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        if verb == "root_get" {
            return None;
        }
        let component = caps.get(3).map(|m| m.as_str())?;
        return NAMESPACES
            .iter()
            .find(|ns| component.eq_ignore_ascii_case(ns))
            .copied();
    }

    let delegate_re = Regex::new(r"self\s*\.\s*(\w+)\s*\(").unwrap();
    for caps in delegate_re.captures_iter(body) {
        let target = &caps[1];
        if target == name {
            continue;
        }
        if boundaries
            .iter()
            .any(|b| b.is_pub_async && b.name == target)
            && let Some(component) = resolve_component(target, boundaries, source, depth + 1)
        {
            return Some(component);
        }
    }

    None
}

/// Derived map plus the population accounting [`method_namespace_map_size_reconciles_with_pr1s_extracted_call_count`]
/// cross-checks against `atlas_client_route_contract.rs`'s own pin.
struct DerivedMap {
    map: HashMap<String, &'static str>,
    total_pub_async_fns: usize,
    direct_call_methods: usize,
    excluded_by_name: Vec<&'static str>,
    naturally_rootless: Vec<String>,
}

/// Walks `atlas_client`'s production source and derives every `pub async
/// fn`'s home namespace, independently of `atlas_client_route_contract.rs`'s
/// `extract_calls` (design D5: "independent implementation, not a shared
/// function").
///
/// Walks `lib.rs` plus every [`SPLIT_PRODUCTION_FILES`] entry, on equal
/// footing. `lib.rs`'s own D6 shim forwarders (`#[doc(hidden)]`-attributed
/// `pub async fn`s with no direct verb call of their own) are excluded from
/// its population entirely — once a method's real implementation moves to a
/// split file, its home is derived from that file's own `Component::X`
/// literal, not treated as newly rootless because the root now only
/// forwards to it.
fn derive_method_namespace_map() -> DerivedMap {
    let verb_re = Regex::new(r"self\s*\.\s*(get|post|patch|put|delete|root_get)\s*\(").unwrap();

    let mut total_pub_async_fns = 0;
    let mut direct_call_methods = 0;
    let mut map = HashMap::new();
    let mut naturally_rootless = Vec::new();

    let mut files = vec!["lib.rs"];
    files.extend(SPLIT_PRODUCTION_FILES);

    for file_name in files {
        let source = client_production_source(file_name);
        let boundaries = function_boundaries(&source);
        let shim_names = if file_name == "lib.rs" {
            hidden_forwarder_names(&source)
        } else {
            std::collections::HashSet::new()
        };

        let pub_async_names: Vec<&str> = boundaries
            .iter()
            .filter(|b| b.is_pub_async && !shim_names.contains(&b.name))
            .map(|b| b.name.as_str())
            .collect();

        direct_call_methods += pub_async_names
            .iter()
            .filter(|name| {
                function_body(&source, &boundaries, name).is_some_and(|body| verb_re.is_match(body))
            })
            .count();

        for name in &pub_async_names {
            if NEVER_NAMESPACED
                .iter()
                .any(|(excluded, _)| excluded == name)
            {
                continue;
            }

            match resolve_component(name, &boundaries, &source, 0) {
                Some(component) => {
                    map.insert((*name).to_string(), component);
                }
                None => naturally_rootless.push((*name).to_string()),
            }
        }

        total_pub_async_fns += pub_async_names.len();
    }

    DerivedMap {
        map,
        total_pub_async_fns,
        direct_call_methods,
        excluded_by_name: NEVER_NAMESPACED.iter().map(|(name, _)| *name).collect(),
        naturally_rootless,
    }
}

#[test]
fn hidden_forwarder_names_match_only_doc_hidden_pub_async_fns() {
    let source = "\
    #[doc(hidden)]
    pub async fn plain(&self) {}

    #[doc(hidden)]
    /// Forwarder to `custos()`; removed in PR12 of this slice.
    pub async fn documented(&self, ws: &str) {}

    /// `GET /real`
    pub async fn real(&self) {}

    #[doc(hidden)]
    pub fn not_async(&self) {}
";

    let names = hidden_forwarder_names(source);

    let mut sorted: Vec<&str> = names.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    assert_eq!(sorted, vec!["documented", "plain"]);
}

#[test]
fn lib_rs_forwarders_are_the_pr3_pr4_and_pr5_moves() {
    let names = hidden_forwarder_names(&client_production_source("lib.rs"));

    assert_eq!(
        names.len(),
        195,
        "PR3 added 34 custos() forwarders, PR4 added 157 acta() forwarders, \
         PR5 added 4 platform() forwarders"
    );
    assert!(names.contains("me"));
    assert!(names.contains("list_workspace_audit"));
    assert!(names.contains("list_projects"));
    assert!(names.contains("list_workspace_activity_with_cursor"));
    assert!(names.contains("get_ui_state"));
    assert!(names.contains("doctor"));
    assert!(hidden_forwarder_names(&client_production_source("custos.rs")).is_empty());
    assert!(hidden_forwarder_names(&client_production_source("acta.rs")).is_empty());
    assert!(hidden_forwarder_names(&client_production_source("platform.rs")).is_empty());
}

#[test]
fn the_derived_map_is_non_empty_and_covers_every_component() {
    let derived = derive_method_namespace_map();
    assert!(!derived.map.is_empty());
    for namespace in NAMESPACES {
        assert!(
            derived.map.values().any(|component| component == namespace),
            "no method resolved to component '{namespace}'"
        );
    }
}

#[test]
fn login_and_health_are_the_only_methods_with_no_home_namespace() {
    let derived = derive_method_namespace_map();
    assert_eq!(derived.excluded_by_name, vec!["login"]);
    assert_eq!(derived.naturally_rootless, vec!["health".to_string()]);
}

/// Cross-check against `atlas_client_route_contract.rs`'s own count pin
/// (`extracted_call_count_is_pinned`), proving the two independently-written
/// walkers agree on the client's method population. The two counts are not
/// raw-equal: PR1's pin counts *call sites* (194 = every `pub async fn` that
/// issues its own `self.<verb>(..)` call directly, one call apiece — no
/// method in the crate issues more than one); this map counts *methods with
/// a home namespace* (195 = those same 194 call-bearing methods, minus
/// `login` and `health`, which never move to a sub-client, plus the 3
/// delegate-only methods — `list_documents`, `list_documents_with_unfiled_filter`,
/// `create_task` — resolved transitively to their callee's home). The
/// reconciliation below is the real cross-check: every method the map
/// visits is either one of PR1's 194 call-bearing methods or one of these 3
/// named delegates, and the two audits agree on the full 197-method
/// population with no method uncounted by either.
#[test]
fn method_namespace_map_size_reconciles_with_pr1s_extracted_call_count() {
    const PR1_EXTRACTED_CALL_COUNT: usize = 194;

    let derived = derive_method_namespace_map();
    assert_eq!(
        derived.direct_call_methods, PR1_EXTRACTED_CALL_COUNT,
        "the number of pub async fn issuing their own self.<verb>(..) call directly \
         no longer matches atlas_client_route_contract.rs's extracted-call-count pin"
    );

    let delegate_only = derived.total_pub_async_fns - derived.direct_call_methods;
    assert_eq!(
        derived.map.len() + derived.excluded_by_name.len() + derived.naturally_rootless.len(),
        derived.total_pub_async_fns,
        "every pub async fn must be either mapped, explicitly excluded, or naturally rootless"
    );
    assert_eq!(
        derived.map.len(),
        PR1_EXTRACTED_CALL_COUNT
            - derived.excluded_by_name.len()
            - derived.naturally_rootless.len()
            + delegate_only,
        "the derived map's size must reconcile with PR1's pin via the named exceptions and delegates"
    );
}

// ---------------------------------------------------------------------------
// Forward check: per-crate / per-file flat-call-site counts
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `content`'s comments and string-literal contents masked out via
/// [`support::scan`], so a doc comment or a help string mentioning a mapped
/// method name is never counted as a call site.
fn masked_code(path: &Path) -> String {
    let content = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    scan(&content).code
}

/// Counts `code`'s occurrences of `\.\s*<method>\s*\(` that are not
/// immediately preceded by a namespace accessor (`\.\s*(acta|custos|platform)\s*\(\s*\)\s*`)
/// and whose receiver is not the `self` keyword — design D5.2's
/// forward-check rule. `\s*` spans newlines by construction in the `regex`
/// crate, so a chained `client\n    .<method>(` site counts identically to
/// a single-line one.
///
/// A `self` receiver (`self.<method>(`, `self\n    .<method>(`,
/// `(self).<method>(`) is never an `AtlasClient` call in any consumer:
/// every real client call goes through a local binding, while
/// `AtlasMcp`'s tool dispatch calls its own inherent methods through
/// `self`, several of which share a name with a client method. The
/// exclusion is keyed on the whole keyword (`\bself`), so an identifier
/// merely containing `self` still counts.
fn flat_call_count(code: &str, method: &str) -> usize {
    let re = Regex::new(&format!(
        r"(\bself\s*|\(\s*self\s*\)\s*)?(\.\s*(?:acta|custos|platform)\s*\(\s*\)\s*)?\.\s*{}\s*\(",
        regex::escape(method)
    ))
    .unwrap();

    re.captures_iter(code)
        .filter(|caps| caps.get(1).is_none() && caps.get(2).is_none())
        .count()
}

/// Sums [`flat_call_count`] over every method in `map`, for one file's
/// masked code.
fn flat_call_count_all(code: &str, map: &HashMap<String, &'static str>) -> usize {
    map.keys().map(|method| flat_call_count(code, method)).sum()
}

fn rust_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }

    files
}

/// Whether `code` (already comment/string-masked) references `AtlasClient`
/// as a real identifier — a doc comment or an allowlist reason string
/// mentioning the name is never enough, since [`masked_code`] already
/// stripped it.
fn references_atlas_client(code: &str) -> bool {
    Regex::new(r"\bAtlasClient\b").unwrap().is_match(code)
}

/// Every top-level `.rs` file directly in `crates/atlas_server/tests/`
/// (excluding `support/` and this guard's own file) whose masked code
/// references `AtlasClient` as a real identifier — the genuine consumer set
/// this PR pins, matching spec ground truth §0.4's "51 files import
/// `AtlasClient`" measurement (a raw, unmasked grep; masking here narrows
/// it to files that use the identifier in code, not only in a comment or an
/// allowlist reason string — see this PR's body for the resulting count).
fn atlas_server_test_files() -> Vec<PathBuf> {
    let dir = repo_root().join("crates/atlas_server/tests");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("read crates/atlas_server/tests")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| {
            path.file_name().and_then(|n| n.to_str()) != Some("client_call_shape_guard.rs")
        })
        .filter(|path| references_atlas_client(&masked_code(path)))
        .collect();
    files.sort();
    files
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .expect("utf-8 file name")
        .to_string()
}

/// `crates/atlas_cli/**`'s combined flat-call count — pinned as one
/// crate-wide sum (design D7's PR6 migrates the whole crate in one PR).
const ATLAS_CLI_PIN: usize = 0;

/// `crates/atlas_mcp/src/lib.rs`'s flat-call count. Flipped to 0 in PR7:
/// all 128 sites (123 acta, 5 custos) are namespaced directly, with no
/// shim, mirroring PR6's `atlas_cli` migration.
const ATLAS_MCP_PIN: usize = 0;

/// `crates/atlas_client/src/helpers.rs`'s flat-call count. Flipped to 0 in
/// PR5 (T5.7): its six sites (`list_projects`, `list_boards`,
/// `list_columns`) are namespaced directly, with no shim, since `helpers.rs`
/// is a client-internal consumer of the `acta` namespace it depends on.
const ATLAS_CLIENT_HELPERS_PIN: usize = 0;

/// Every `crates/atlas_server/tests/*.rs` file with a nonzero flat-call
/// count, pinned individually (design T2.4: "pin each file's count
/// individually, not as one crate-wide sum, so PR8-PR11's per-file
/// migration can flip pins one file at a time"). A file not listed here
/// MUST measure 0 — checked by [`every_test_file_not_pinned_measures_zero`],
/// the totality half of this table.
const ATLAS_SERVER_TEST_PINS: &[(&str, usize)] = &[
    ("api_account_status.rs", 20),
    ("api_activation.rs", 1),
    ("api_audit_read.rs", 14),
    ("api_audit_writes.rs", 0),
    ("api_auth.rs", 8),
    ("api_boards_tasks.rs", 0),
    ("api_capability_sweep.rs", 0),
    ("api_comment_attachments.rs", 0),
    ("api_comments.rs", 0),
    ("api_copy.rs", 13),
    ("api_create_workspace.rs", 7),
    ("api_doctor.rs", 7),
    ("api_document_comments.rs", 25),
    ("api_documents.rs", 0),
    ("api_events_stream.rs", 7),
    ("api_extractor.rs", 3),
    ("api_folders.rs", 0),
    ("api_global_admin_bypass.rs", 9),
    ("api_grants.rs", 0),
    ("api_group_grants.rs", 0),
    ("api_groups.rs", 0),
    ("api_key_grant_access.rs", 0),
    ("api_members.rs", 0),
    ("api_page_conformance.rs", 0),
    ("api_permissions.rs", 26),
    ("api_platform_status_templates.rs", 22),
    ("api_presence_agent.rs", 6),
    ("api_presence_document.rs", 2),
    ("api_property_definitions.rs", 26),
    ("api_saved_searches.rs", 0),
    ("api_search_permissions.rs", 2),
    ("api_self_protection.rs", 18),
    ("api_semantic_search.rs", 5),
    ("api_settings.rs", 0),
    ("api_status_templates.rs", 0),
    ("api_subtasks.rs", 0),
    ("api_system_admin.rs", 16),
    ("api_task_attachments.rs", 0),
    ("api_task_views.rs", 0),
    ("api_tenancy.rs", 9),
    ("api_trash.rs", 2),
    ("api_ui_state.rs", 11),
    ("api_user_api_keys.rs", 0),
    ("api_users.rs", 19),
    ("api_workspace_activity.rs", 0),
    ("api_workspace_attachments.rs", 0),
    ("api_workspace_tasks.rs", 0),
    ("api_workspaces.rs", 0),
    ("idempotency_live_sweep.rs", 0),
];

/// The self-test harness (design T2.7/T2.8): a pure, independently-testable
/// exact-equality check every forward-pin test below runs through, so the
/// "does a drifted pin get caught" guarantee is proven once, against
/// synthetic values, rather than trusted from the real assertions alone.
fn pin_check(label: &str, pinned: usize, measured: usize) -> Result<(), String> {
    if pinned == measured {
        Ok(())
    } else {
        Err(format!(
            "{label}'s measured flat-call count ({measured}) drifted from its pin ({pinned}) \
             without the pin moving"
        ))
    }
}

#[test]
fn atlas_cli_forward_pin_is_exact() {
    let derived = derive_method_namespace_map();
    let files = rust_files_recursive(&repo_root().join("crates/atlas_cli/src"));
    let total: usize = files
        .iter()
        .map(|path| flat_call_count_all(&masked_code(path), &derived.map))
        .sum();

    pin_check("atlas_cli", ATLAS_CLI_PIN, total).unwrap();
}

#[test]
fn atlas_mcp_forward_pin_is_exact() {
    let derived = derive_method_namespace_map();
    let path = repo_root().join("crates/atlas_mcp/src/lib.rs");
    let total = flat_call_count_all(&masked_code(&path), &derived.map);

    pin_check("atlas_mcp", ATLAS_MCP_PIN, total).unwrap();
}

#[test]
fn atlas_client_helpers_forward_pin_is_exact() {
    let derived = derive_method_namespace_map();
    let path = repo_root().join("crates/atlas_client/src/helpers.rs");
    let total = flat_call_count_all(&masked_code(&path), &derived.map);

    pin_check("atlas_client::helpers", ATLAS_CLIENT_HELPERS_PIN, total).unwrap();
}

#[test]
fn every_pinned_test_file_matches_its_measured_count() {
    let derived = derive_method_namespace_map();
    let pins: HashMap<&str, usize> = ATLAS_SERVER_TEST_PINS.iter().copied().collect();
    let mut mismatches = Vec::new();

    for path in atlas_server_test_files() {
        let name = file_name(&path);
        let measured = flat_call_count_all(&masked_code(&path), &derived.map);
        match pins.get(name.as_str()) {
            Some(&pinned) => {
                if let Err(message) = pin_check(&name, pinned, measured) {
                    mismatches.push(message);
                }
            }
            None if measured != 0 => {
                mismatches.push(format!("{name}: measured {measured} but has no pin entry"));
            }
            None => {}
        }
    }

    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn every_pin_table_entry_names_a_real_file() {
    let real_files: Vec<String> = atlas_server_test_files()
        .iter()
        .map(|p| file_name(p))
        .collect();
    let stale: Vec<&str> = ATLAS_SERVER_TEST_PINS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !real_files.contains(&(*name).to_string()))
        .collect();

    assert!(
        stale.is_empty(),
        "pin table names a file that no longer exists: {stale:?}"
    );
}

// ---------------------------------------------------------------------------
// Reverse check: a namespaced call site's namespace equals its declared home
// ---------------------------------------------------------------------------

/// Every `\.\s*(acta|custos|platform)\s*\(\s*\)\s*\.\s*<method>\s*\(` site in
/// `code`, as `(namespace, method)`.
fn namespaced_call_sites(code: &str) -> Vec<(String, String)> {
    let re = Regex::new(r"\.\s*(acta|custos|platform)\s*\(\s*\)\s*\.\s*(\w+)\s*\(").unwrap();
    re.captures_iter(code)
        .map(|caps| (caps[1].to_string(), caps[2].to_string()))
        .collect()
}

/// Flags every namespaced call site in `sites` whose namespace disagrees
/// with `map`'s declared home for that method (design D5.3's reverse
/// check).
fn reverse_check_mismatches(
    sites: &[(String, String)],
    map: &HashMap<String, &'static str>,
) -> Vec<String> {
    sites
        .iter()
        .filter_map(|(namespace, method)| {
            let home = map.get(method)?;
            (namespace != home).then(|| {
                format!("client.{namespace}().{method}(..) — {method}'s declared home is {home}")
            })
        })
        .collect()
}

/// `atlas_client::helpers.rs` is excluded here: PR5 (T5.7) namespaces its
/// six sites directly, with no shim, since it is a client-internal consumer
/// migrated in the same PR as the namespace it depends on. Its own reverse
/// check lives in
/// [`atlas_client_helpers_namespaced_sites_match_their_declared_home`].
/// `crates/atlas_cli/src` is excluded here too: PR6 namespaces all 149 of its
/// sites directly, with no shim. Its own reverse check lives in
/// [`atlas_cli_namespaced_sites_match_their_declared_home`].
/// `crates/atlas_mcp/src/lib.rs` is excluded here too: PR7 namespaces all
/// 128 of its sites directly, with no shim. Its own reverse check lives in
/// [`atlas_mcp_namespaced_sites_match_their_declared_home`].
/// `crates/atlas_server/tests/api_boards_tasks.rs` is excluded here too: PR8
/// namespaces all 526 of its sites directly, with no shim. Its own reverse
/// check lives in [`api_boards_tasks_namespaced_sites_match_their_declared_home`].
/// `api_documents.rs`, `api_comment_attachments.rs`, and `api_folders.rs`
/// are excluded here too: PR9 namespaces all of their sites directly, with
/// no shim. Their own reverse checks live in
/// [`api_documents_namespaced_sites_match_their_declared_home`],
/// [`api_comment_attachments_namespaced_sites_match_their_declared_home`],
/// and [`api_folders_namespaced_sites_match_their_declared_home`].
/// `api_capability_sweep.rs`, `api_comments.rs`, `api_group_grants.rs`, and
/// `api_user_api_keys.rs` are excluded here too: PR10 namespaces all of
/// their sites directly, with no shim. Their own reverse checks live in
/// [`api_capability_sweep_namespaced_sites_match_their_declared_home`],
/// [`api_comments_namespaced_sites_match_their_declared_home`],
/// [`api_group_grants_namespaced_sites_match_their_declared_home`], and
/// [`api_user_api_keys_namespaced_sites_match_their_declared_home`].
/// `api_task_views.rs`, `api_saved_searches.rs`, `api_status_templates.rs`,
/// `api_workspace_activity.rs`, `api_members.rs`, `api_audit_writes.rs`,
/// `api_workspace_attachments.rs`, and `api_grants.rs` are excluded here too:
/// PR11a namespaces all of their sites directly, with no shim. Their own
/// reverse checks live in
/// [`api_task_views_namespaced_sites_match_their_declared_home`],
/// [`api_saved_searches_namespaced_sites_match_their_declared_home`],
/// [`api_status_templates_namespaced_sites_match_their_declared_home`],
/// [`api_workspace_activity_namespaced_sites_match_their_declared_home`],
/// [`api_members_namespaced_sites_match_their_declared_home`],
/// [`api_audit_writes_namespaced_sites_match_their_declared_home`],
/// [`api_workspace_attachments_namespaced_sites_match_their_declared_home`],
/// and [`api_grants_namespaced_sites_match_their_declared_home`].
/// `api_workspace_tasks.rs`, `api_workspaces.rs`, `api_settings.rs`,
/// `api_key_grant_access.rs`, `idempotency_live_sweep.rs`, `api_subtasks.rs`,
/// `api_page_conformance.rs`, `api_groups.rs`, and `api_task_attachments.rs`
/// are excluded here too: PR11b namespaces all of their sites directly, with
/// no shim. Their own reverse checks live in
/// [`api_workspace_tasks_namespaced_sites_match_their_declared_home`],
/// [`api_workspaces_namespaced_sites_match_their_declared_home`],
/// [`api_settings_namespaced_sites_match_their_declared_home`],
/// [`api_key_grant_access_namespaced_sites_match_their_declared_home`],
/// [`idempotency_live_sweep_namespaced_sites_match_their_declared_home`],
/// [`api_subtasks_namespaced_sites_match_their_declared_home`],
/// [`api_page_conformance_namespaced_sites_match_their_declared_home`],
/// [`api_groups_namespaced_sites_match_their_declared_home`], and
/// [`api_task_attachments_namespaced_sites_match_their_declared_home`].
#[test]
fn no_namespaced_call_site_exists_anywhere_yet_outside_atlas_client_helpers_and_atlas_cli_and_atlas_mcp_and_pr8_pr9_pr10_pr11a_and_pr11b_files()
 {
    let derived = derive_method_namespace_map();
    let mut all_sites = Vec::new();
    let already_migrated = [
        "api_boards_tasks.rs",
        "api_documents.rs",
        "api_comment_attachments.rs",
        "api_folders.rs",
        "api_capability_sweep.rs",
        "api_comments.rs",
        "api_group_grants.rs",
        "api_user_api_keys.rs",
        "api_task_views.rs",
        "api_saved_searches.rs",
        "api_status_templates.rs",
        "api_workspace_activity.rs",
        "api_members.rs",
        "api_audit_writes.rs",
        "api_workspace_attachments.rs",
        "api_grants.rs",
        "api_workspace_tasks.rs",
        "api_workspaces.rs",
        "api_settings.rs",
        "api_key_grant_access.rs",
        "idempotency_live_sweep.rs",
        "api_subtasks.rs",
        "api_page_conformance.rs",
        "api_groups.rs",
        "api_task_attachments.rs",
    ];

    for path in atlas_server_test_files() {
        if already_migrated.contains(&file_name(&path).as_str()) {
            continue;
        }
        all_sites.extend(namespaced_call_sites(&masked_code(&path)));
    }

    assert_eq!(
        all_sites.len(),
        0,
        "found a namespaced call site outside atlas_client::helpers.rs / atlas_cli / atlas_mcp / \
         api_boards_tasks.rs / api_documents.rs / api_comment_attachments.rs / api_folders.rs / \
         api_capability_sweep.rs / api_comments.rs / api_group_grants.rs / api_user_api_keys.rs / \
         api_task_views.rs / api_saved_searches.rs / api_status_templates.rs / \
         api_workspace_activity.rs / api_members.rs / api_audit_writes.rs / \
         api_workspace_attachments.rs / api_grants.rs / api_workspace_tasks.rs / \
         api_workspaces.rs / api_settings.rs / api_key_grant_access.rs / \
         idempotency_live_sweep.rs / api_subtasks.rs / api_page_conformance.rs / \
         api_groups.rs / api_task_attachments.rs before its consumer PR migrated: {all_sites:?}"
    );

    // The reverse check runs and reports zero mismatches over zero sites —
    // distinct from "did not run at all" (design T2.5's non-vacuous-by-construction
    // requirement).
    let mismatches = reverse_check_mismatches(&all_sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

/// T5.6/T5.7 — `atlas_client::helpers.rs`'s six sites are namespaced
/// directly in PR5, with no shim. Every one must resolve to `acta` (its
/// declared home per [`derive_method_namespace_map`]) or this test names
/// the mismatch.
#[test]
fn atlas_client_helpers_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_client/src/helpers.rs"),
    ));

    assert_eq!(
        sites.len(),
        6,
        "expected 6 namespaced sites in atlas_client::helpers.rs (list_projects x2, \
         list_boards x2, list_columns x2), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn atlas_cli_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let mut sites = Vec::new();
    for path in rust_files_recursive(&repo_root().join("crates/atlas_cli/src")) {
        sites.extend(namespaced_call_sites(&masked_code(&path)));
    }

    assert_eq!(
        sites.len(),
        149,
        "expected 149 namespaced sites in crates/atlas_cli/src (PR6), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn atlas_mcp_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_mcp/src/lib.rs"),
    ));

    assert_eq!(
        sites.len(),
        128,
        "expected 128 namespaced sites in crates/atlas_mcp/src/lib.rs (PR7), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_boards_tasks_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_boards_tasks.rs"),
    ));

    assert_eq!(
        sites.len(),
        526,
        "expected 526 namespaced sites in crates/atlas_server/tests/api_boards_tasks.rs (PR8), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_documents_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_documents.rs"),
    ));

    assert_eq!(
        sites.len(),
        197,
        "expected 197 namespaced sites in crates/atlas_server/tests/api_documents.rs (PR9), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_comment_attachments_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_comment_attachments.rs"),
    ));

    assert_eq!(
        sites.len(),
        50,
        "expected 50 namespaced sites in crates/atlas_server/tests/api_comment_attachments.rs \
         (PR9), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_folders_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_folders.rs"),
    ));

    assert_eq!(
        sites.len(),
        107,
        "expected 107 namespaced sites in crates/atlas_server/tests/api_folders.rs (PR9), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_capability_sweep_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_capability_sweep.rs"),
    ));

    assert_eq!(
        sites.len(),
        113,
        "expected 113 namespaced sites in crates/atlas_server/tests/api_capability_sweep.rs \
         (PR10), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_comments_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_comments.rs"),
    ));

    assert_eq!(
        sites.len(),
        125,
        "expected 125 namespaced sites in crates/atlas_server/tests/api_comments.rs (PR10), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_group_grants_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_group_grants.rs"),
    ));

    assert_eq!(
        sites.len(),
        63,
        "expected 63 namespaced sites in crates/atlas_server/tests/api_group_grants.rs (PR10), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_user_api_keys_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_user_api_keys.rs"),
    ));

    assert_eq!(
        sites.len(),
        56,
        "expected 56 namespaced sites in crates/atlas_server/tests/api_user_api_keys.rs (PR10), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_task_views_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_task_views.rs"),
    ));

    assert_eq!(
        sites.len(),
        53,
        "expected 53 namespaced sites in crates/atlas_server/tests/api_task_views.rs (PR11a), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_saved_searches_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_saved_searches.rs"),
    ));

    assert_eq!(
        sites.len(),
        53,
        "expected 53 namespaced sites in crates/atlas_server/tests/api_saved_searches.rs (PR11a), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_status_templates_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_status_templates.rs"),
    ));

    assert_eq!(
        sites.len(),
        47,
        "expected 47 namespaced sites in crates/atlas_server/tests/api_status_templates.rs \
         (PR11a), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_workspace_activity_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_workspace_activity.rs"),
    ));

    assert_eq!(
        sites.len(),
        46,
        "expected 46 namespaced sites in crates/atlas_server/tests/api_workspace_activity.rs \
         (PR11a), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_members_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_members.rs"),
    ));

    assert_eq!(
        sites.len(),
        44,
        "expected 44 namespaced sites in crates/atlas_server/tests/api_members.rs (PR11a), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_audit_writes_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_audit_writes.rs"),
    ));

    assert_eq!(
        sites.len(),
        44,
        "expected 44 namespaced sites in crates/atlas_server/tests/api_audit_writes.rs (PR11a), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_workspace_attachments_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_workspace_attachments.rs"),
    ));

    assert_eq!(
        sites.len(),
        43,
        "expected 43 namespaced sites in crates/atlas_server/tests/api_workspace_attachments.rs \
         (PR11a), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_grants_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_grants.rs"),
    ));

    assert_eq!(
        sites.len(),
        40,
        "expected 40 namespaced sites in crates/atlas_server/tests/api_grants.rs (PR11a), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_workspace_tasks_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_workspace_tasks.rs"),
    ));

    assert_eq!(
        sites.len(),
        38,
        "expected 38 namespaced sites in crates/atlas_server/tests/api_workspace_tasks.rs \
         (PR11b), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_workspaces_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_workspaces.rs"),
    ));

    assert_eq!(
        sites.len(),
        37,
        "expected 37 namespaced sites in crates/atlas_server/tests/api_workspaces.rs (PR11b), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_settings_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_settings.rs"),
    ));

    assert_eq!(
        sites.len(),
        36,
        "expected 36 namespaced sites in crates/atlas_server/tests/api_settings.rs (PR11b), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_key_grant_access_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_key_grant_access.rs"),
    ));

    assert_eq!(
        sites.len(),
        36,
        "expected 36 namespaced sites in crates/atlas_server/tests/api_key_grant_access.rs \
         (PR11b), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn idempotency_live_sweep_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/idempotency_live_sweep.rs"),
    ));

    assert_eq!(
        sites.len(),
        34,
        "expected 34 namespaced sites in crates/atlas_server/tests/idempotency_live_sweep.rs \
         (PR11b), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_subtasks_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_subtasks.rs"),
    ));

    assert_eq!(
        sites.len(),
        28,
        "expected 28 namespaced sites in crates/atlas_server/tests/api_subtasks.rs (PR11b), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_page_conformance_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_page_conformance.rs"),
    ));

    assert_eq!(
        sites.len(),
        28,
        "expected 28 namespaced sites in crates/atlas_server/tests/api_page_conformance.rs \
         (PR11b), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_groups_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_groups.rs"),
    ));

    assert_eq!(
        sites.len(),
        28,
        "expected 28 namespaced sites in crates/atlas_server/tests/api_groups.rs (PR11b), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_task_attachments_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_task_attachments.rs"),
    ));

    assert_eq!(
        sites.len(),
        26,
        "expected 26 namespaced sites in crates/atlas_server/tests/api_task_attachments.rs \
         (PR11b), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn a_call_namespaced_to_the_wrong_home_is_flagged_by_name() {
    let derived = derive_method_namespace_map();
    // `get_task`'s declared home is `acta`; this fixture calls it through `custos`.
    let fixture = "let response = client.custos().get_task(ws, id).await?;";
    let sites = namespaced_call_sites(fixture);
    assert_eq!(sites, vec![("custos".to_string(), "get_task".to_string())]);

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches.len(), 1);
    let mismatch = mismatches.first().expect("checked len() == 1 above");
    assert!(mismatch.contains("get_task"));
    assert!(mismatch.contains("custos"));
    assert!(mismatch.contains("acta"));
}

#[test]
fn a_call_namespaced_to_its_declared_home_is_not_flagged() {
    let derived = derive_method_namespace_map();
    let fixture = "let response = client.acta().get_task(ws, id).await?;";
    let sites = namespaced_call_sites(fixture);
    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert!(mismatches.is_empty());
}

// ---------------------------------------------------------------------------
// Self-test probes: a real chained atlas_mcp site is counted; a stale pin
// is caught
// ---------------------------------------------------------------------------

#[test]
fn a_real_chained_atlas_mcp_call_site_is_counted() {
    // Verbatim shape from `crates/atlas_mcp/src/lib.rs`'s `search` handler:
    // the receiver and the method sit on different lines.
    let fixture = "let page = client\n            .search(\n                &params.workspace,\n                &params.query,\n            )\n            .await\n            .map_err(|e| enrich_client_error(e, \"search\"))?;\n";

    assert_eq!(flat_call_count(fixture, "search"), 1);
}

#[test]
fn a_self_receiver_is_never_counted_but_a_local_binding_is() {
    // `AtlasMcp`'s dispatch calls its own inherent `search` through `self`
    // (verbatim shape from `crates/atlas_mcp/src/lib.rs`); neither the
    // single-line nor the chained form is an `AtlasClient` call.
    assert_eq!(
        flat_call_count("self.search(catalog::decode(call.params)?, ctx)", "search"),
        0
    );
    assert_eq!(
        flat_call_count("self\n    .search(&ws, &query)\n    .await?", "search"),
        0
    );
    assert_eq!(
        flat_call_count("(self).search(&ws, &query).await?", "search"),
        0
    );

    // Only the whole keyword is excluded: an identifier containing `self`
    // is an ordinary receiver.
    assert_eq!(
        flat_call_count("myself.search(&ws, &query).await?", "search"),
        1
    );
    assert_eq!(
        flat_call_count("self_client.search(&ws, &query).await?", "search"),
        1
    );

    // Local bindings count in both shapes.
    assert_eq!(
        flat_call_count("client\n    .search(&ws, &query)\n    .await?", "search"),
        1
    );
    assert_eq!(flat_call_count("root.doctor(&ws).await?", "doctor"), 1);
}

#[test]
fn a_stale_pin_off_by_one_in_either_direction_is_caught() {
    let code = "client.get_task(ws, id).await?;\nclient.get_task(ws, id2).await?;\n";
    let measured = flat_call_count(code, "get_task");
    assert_eq!(
        measured, 2,
        "fixture must itself measure a known, non-zero count"
    );

    // A pin recorded one too high, and one too low, must both fail — this is
    // the self-test harness (design T2.7/T2.8) checking its own comparison
    // logic against a synthetic, re-measured value, not re-running the real
    // crates a second time.
    pin_check("fixture", measured + 1, measured).unwrap_err();
    pin_check("fixture", measured - 1, measured).unwrap_err();

    // The correct pin still passes.
    pin_check("fixture", measured, measured).unwrap();
}

#[test]
fn masking_excludes_a_call_shape_that_lives_only_in_a_comment_or_string() {
    let source = "// client.get_task(ws, id).await?;\n\
                  /* client\n    .get_task(ws, id) */\n\
                  let msg = \"client.get_task(ws, id)\";\n\
                  client.get_task(ws, id).await?;\n";

    let unmasked = flat_call_count(source, "get_task");
    let masked = flat_call_count(&scan(source).code, "get_task");

    assert_eq!(
        unmasked, 4,
        "without masking every commented or quoted call shape counts"
    );
    assert_eq!(
        masked, 1,
        "masking must leave exactly the one real call site"
    );
}
