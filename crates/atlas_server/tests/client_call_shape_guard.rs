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
use std::sync::{Mutex, OnceLock};

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

/// Every test in this module calls [`derive_method_namespace_map`], and the
/// map's inputs (`lib.rs` plus every [`SPLIT_PRODUCTION_FILES`] entry) never
/// change within one test run, so this cache computes it once per process
/// instead of once per test — PR12's runtime cut (design's "if cheap, cut
/// the guard's runtime" note): rebuilding the map from disk on every one of
/// this module's ~85 tests was the single largest repeated cost.
fn derive_method_namespace_map() -> &'static DerivedMap {
    static CACHE: OnceLock<DerivedMap> = OnceLock::new();
    CACHE.get_or_init(build_method_namespace_map)
}

/// Walks `atlas_client`'s production source and derives every `pub async
/// fn`'s home namespace, independently of `atlas_client_route_contract.rs`'s
/// `extract_calls` (design D5: "independent implementation, not a shared
/// function").
///
/// Walks `lib.rs` plus every [`SPLIT_PRODUCTION_FILES`] entry, on equal
/// footing. Since PR12 deleted `lib.rs`'s D6 shim forwarders for good, every
/// `pub async fn` this walk finds is a real implementation: `lib.rs`
/// contributes only `login` and `health`, and each split file contributes
/// its own component's methods.
fn build_method_namespace_map() -> DerivedMap {
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

        let pub_async_names: Vec<&str> = boundaries
            .iter()
            .filter(|b| b.is_pub_async)
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

/// T12.1/T12.5 — PR12 deletes every D6 shim forwarder (PR3's 34 `custos()`
/// forwarders, PR4's 157 `acta()` forwarders, PR5's 4 `platform()`
/// forwarders). The `#[doc(hidden)]` marker was this guard's own way of
/// telling a forwarder apart from a real implementation while the shim
/// existed (design D6's closing paragraph); now that the shim is gone for
/// good, this is a plain regression check that none of it reappears in
/// `lib.rs` or in any split file.
#[test]
fn no_doc_hidden_shim_forwarder_remains_anywhere_in_atlas_client() {
    let doc_hidden_re = Regex::new(r"#\[doc\(hidden\)\]").unwrap();

    for file_name in ["lib.rs", "custos.rs", "acta.rs", "platform.rs"] {
        let count = doc_hidden_re
            .find_iter(&client_production_source(file_name))
            .count();
        assert_eq!(
            count, 0,
            "{file_name} must carry no #[doc(hidden)] shim forwarder after PR12"
        );
    }
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
///
/// Cached per path: this module's forward and reverse checks re-mask the
/// same `crates/atlas_server/tests/*.rs` file across many tests (once per
/// dedicated per-file reverse-check test, plus again inside every aggregate
/// check that walks the whole pinned set), so masking each file's content
/// once per process — not once per test — is PR12's runtime cut alongside
/// [`derive_method_namespace_map`]'s cache.
fn masked_code(path: &Path) -> String {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Some(code) = cache.lock().unwrap().get(path) {
        return code.clone();
    }

    let content = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let code = scan(&content).code;
    cache
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), code.clone());
    code
}

/// Counts `code`'s occurrences of `\.\s*<method>\s*\(` that are not
/// immediately preceded by a namespace accessor (`\.\s*(acta|custos|platform)\s*\(\s*\)\s*`)
/// and whose receiver is not the `self` keyword or a known non-`AtlasClient`
/// repository receiver — design D5.2's forward-check rule. `\s*` spans
/// newlines by construction in the `regex` crate, so a chained
/// `client\n    .<method>(` site counts identically to a single-line one.
///
/// A `self` receiver (`self.<method>(`, `self\n    .<method>(`,
/// `(self).<method>(`) is never an `AtlasClient` call in any consumer:
/// every real client call goes through a local binding, while
/// `AtlasMcp`'s tool dispatch calls its own inherent methods through
/// `self`, several of which share a name with a client method. The
/// exclusion is keyed on the whole keyword (`\bself`), so an identifier
/// merely containing `self` still counts.
///
/// `PR11c` found `create_board` and `search` also name real inherent
/// methods on `PgBoardRepo` and `PgSearchRepo`, database repository types
/// test helpers construct directly (`PgBoardRepo::new(db.conn().clone())`
/// or `db.board_repo()`) to seed or query fixtures without going through
/// `AtlasClient` at all. Two receiver shapes are excluded here the same way
/// `self` is, rather than left to inflate a per-file pin that can never
/// reach 0 by namespacing (there is no `PgBoardRepo::acta()` or
/// `PgSearchRepo::acta()` to call):
///
/// * a direct constructor or accessor receiver — `<Something>Repo::new(..)
///   .<method>(` or `.<something>_repo().<method>(`;
/// * a local binding whose provenance in the *same* `code` is one of those
///   two shapes — `let repo = PgSearchRepo::new(..);` or
///   `let board_repo = db.board_repo();` — resolved by
///   [`repo_bound_identifiers`]. No identifier is excluded by name alone,
///   so a future `let repo = login(..)` binding an `AtlasClient` still
///   counts.
///
/// PR12's unconditional walk then reached the service-layer tests
/// (`comment_service.rs`, `semantic_index_pipeline.rs`, ...), whose
/// `TaskService`/`DocumentService` instances share method names with the
/// client the same way the repositories do. The same two shapes are
/// excluded for services, by provenance only: a direct
/// `<Something>Service::new(..).<method>(` or `.<something>_service()
/// .<method>(` receiver, and a local binding from one of those in the same
/// `code`.
fn flat_call_count(code: &str, method: &str) -> usize {
    flat_call_count_with(code, &repo_bound_identifiers(code), method)
}

/// [`flat_call_count`] with `code`'s repository-bound identifiers already
/// resolved, so [`flat_call_count_all`] resolves them once per file rather
/// than once per method. The compiled pattern is cached per process: most
/// files bind no repository identifier, so one pattern per method serves
/// every such file, and the sweep compiles a few hundred regexes instead of
/// one per (file, method) pair.
fn flat_call_count_with(code: &str, repo_identifiers: &[String], method: &str) -> usize {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let mut receivers = vec![
        r"\bself\s*".to_string(),
        r"\(\s*self\s*\)\s*".to_string(),
        r"\w*Repo::new\([^;]*?\)\s*".to_string(),
        r"\.\s*\w+_repo\s*\(\s*\)\s*".to_string(),
        r"\w*Service::new\([^;]*?\)\s*".to_string(),
        r"\.\s*\w+_service\s*\(\s*\)\s*".to_string(),
    ];

    for identifier in repo_identifiers {
        receivers.push(format!(r"\b{}\s*", regex::escape(identifier)));
    }

    let pattern = format!(
        r"({})?(\.\s*(?:acta|custos|platform)\s*\(\s*\)\s*)?\.\s*{}\s*\(",
        receivers.join("|"),
        regex::escape(method)
    );
    let re = cache
        .lock()
        .unwrap()
        .entry(pattern)
        .or_insert_with_key(|pattern| Regex::new(pattern).unwrap())
        .clone();

    re.captures_iter(code)
        .filter(|caps| caps.get(1).is_none() && caps.get(2).is_none())
        .count()
}

/// Every identifier `code` binds with `let [mut] <name>[: T] =` directly
/// from a `<Something>Repo::new(..)` or `<Something>Service::new(..)`
/// constructor, or from a `<receiver>.<something>_repo()` or
/// `<receiver>.<something>_service()` accessor, sorted and deduplicated.
/// A binding from a call to a helper the same `code` declares with a
/// `Repo`/`Service` return type (`let svc = make_svc(&db);` next to
/// `fn make_svc(..) -> TaskService`) is resolved one hop through
/// [`repo_constructor_helpers`]. Only these provenances mark a binding as a
/// repository or service receiver for [`flat_call_count`]; a binding of the
/// same name from any other expression contributes nothing.
fn repo_bound_identifiers(code: &str) -> Vec<String> {
    let direct_re = Regex::new(
        r"\blet\s+(?:mut\s+)?(\w+)\s*(?::[^=;]+)?=\s*(?:\w*(?:Repo|Service)::new\s*\(|\w+\s*\.\s*\w+_(?:repo|service)\s*\(\s*\))",
    )
    .unwrap();

    let mut identifiers: Vec<String> = direct_re
        .captures_iter(code)
        .map(|caps| caps[1].to_string())
        .collect();

    let helpers = repo_constructor_helpers(code);
    if !helpers.is_empty() {
        let helper_re = Regex::new(&format!(
            r"\blet\s+(?:mut\s+)?(\w+)\s*(?::[^=;]+)?=\s*(?:{})\s*\(",
            helpers
                .iter()
                .map(|name| regex::escape(name))
                .collect::<Vec<_>>()
                .join("|")
        ))
        .unwrap();
        identifiers.extend(
            helper_re
                .captures_iter(code)
                .map(|caps| caps[1].to_string()),
        );
    }

    identifiers.sort();
    identifiers.dedup();
    identifiers
}

/// Every function `code` declares whose return type's last path segment
/// ends in `Repo` or `Service` (`fn make_svc(..) -> TaskService`,
/// `fn make_doc_repo(..) -> repos::PgDocumentRepo`), sorted and
/// deduplicated. One hop only: a helper that returns through another
/// helper, or through a type alias, is not followed.
fn repo_constructor_helpers(code: &str) -> Vec<String> {
    let re = Regex::new(r"\bfn\s+(\w+)\s*\([^)]*\)\s*->\s*(?:\w+\s*::\s*)*\w*(?:Repo|Service)\b")
        .unwrap();

    let mut names: Vec<String> = re
        .captures_iter(code)
        .map(|caps| caps[1].to_string())
        .collect();

    names.sort();
    names.dedup();
    names
}

/// Sums [`flat_call_count`] over every method in `map`, for one file's
/// masked code.
fn flat_call_count_all(code: &str, map: &HashMap<String, &'static str>) -> usize {
    let repo_identifiers = repo_bound_identifiers(code);
    map.keys()
        .map(|method| flat_call_count_with(code, &repo_identifiers, method))
        .sum()
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

    files.sort();
    files
}

/// `path` relative to `root`, `/`-separated — the key every per-file pin
/// table uses, so a file under a nested directory (`support/mod.rs`,
/// `commands/task.rs`) is named by its full root-relative path rather than
/// by a bare file name that two directories could share.
fn relative_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("walked path lies under its root")
        .components()
        .map(|c| c.as_os_str().to_str().expect("utf-8 path component"))
        .collect::<Vec<_>>()
        .join("/")
}

/// One directory the forward check walks in full, with the pin table that
/// must name every `.rs` file under it.
///
/// The file set is the directory walk itself, unconditionally: through
/// PR11c the `atlas_server/tests` set was narrowed to files whose masked
/// code mentioned the literal `AtlasClient`, and PR12 found 153 flat sites
/// in files that never spell the name because they obtain a client through
/// `support::login_*` or `server.client()` and simply call methods on it.
/// The compiler caught those once the forwarders were gone, but the guard's
/// completeness gate must not lean on a heuristic to decide which files it
/// looks at. `excluded` names only files that are not consumers by
/// construction: the client sources the map is derived from, and this guard
/// itself.
struct ConsumerRoot {
    dir: &'static str,
    excluded: &'static [&'static str],
    pins: &'static [(&'static str, usize)],
}

impl ConsumerRoot {
    fn path(&self) -> PathBuf {
        repo_root().join(self.dir)
    }

    /// Every `.rs` file under this root minus `excluded`, sorted, as
    /// root-relative keys paired with their absolute paths.
    fn files(&self) -> Vec<(String, PathBuf)> {
        walk_root(&self.path(), self.excluded)
    }
}

fn walk_root(root: &Path, excluded: &[&str]) -> Vec<(String, PathBuf)> {
    rust_files_recursive(root)
        .into_iter()
        .map(|path| (relative_key(root, &path), path))
        .filter(|(key, _)| !excluded.contains(&key.as_str()))
        .collect()
}

/// `(key, measured flat-call count)` for every file [`walk_root`] yields
/// under `root` — the one measurement every forward check and the
/// completeness gate run through, so a self-test on a synthetic directory
/// exercises the exact code path the real roots do.
fn measure_root(
    root: &Path,
    excluded: &[&str],
    map: &HashMap<String, &'static str>,
) -> Vec<(String, usize)> {
    walk_root(root, excluded)
        .into_iter()
        .map(|(key, path)| {
            let measured = flat_call_count_all(&masked_code(&path), map);
            (key, measured)
        })
        .collect()
}

/// The four consumer roots design D5 names, each pinned per file.
const CONSUMER_ROOTS: &[ConsumerRoot] = &[
    ConsumerRoot {
        dir: "crates/atlas_cli/src",
        excluded: &[],
        pins: ATLAS_CLI_PINS,
    },
    ConsumerRoot {
        dir: "crates/atlas_mcp/src",
        excluded: &[],
        pins: ATLAS_MCP_PINS,
    },
    ConsumerRoot {
        dir: "crates/atlas_client/src",
        excluded: &["lib.rs", "custos.rs", "acta.rs", "platform.rs"],
        pins: ATLAS_CLIENT_PINS,
    },
    ConsumerRoot {
        dir: "crates/atlas_server/tests",
        excluded: &["client_call_shape_guard.rs"],
        pins: ATLAS_SERVER_TEST_PINS,
    },
];

/// Every `crates/atlas_cli/src/**/*.rs` file's flat-call count. Pinned as
/// one crate-wide sum through PR11c (design D7's PR6 migrated the whole
/// crate in one PR); per file since PR12 so the table itself proves which
/// files the walk sees.
const ATLAS_CLI_PINS: &[(&str, usize)] = &[
    ("cli.rs", 0),
    ("commands/activity.rs", 0),
    ("commands/api_keys.rs", 0),
    ("commands/audit.rs", 0),
    ("commands/boards.rs", 0),
    ("commands/bulk.rs", 0),
    ("commands/columns.rs", 0),
    ("commands/common.rs", 0),
    ("commands/completions.rs", 0),
    ("commands/config.rs", 0),
    ("commands/docs.rs", 0),
    ("commands/doctor.rs", 0),
    ("commands/export/mod.rs", 0),
    ("commands/export/obsidian/mod.rs", 0),
    ("commands/export/obsidian/plan.rs", 0),
    ("commands/export/obsidian/render.rs", 0),
    ("commands/export/obsidian/write.rs", 0),
    ("commands/folders.rs", 0),
    ("commands/grants.rs", 0),
    ("commands/groups.rs", 0),
    ("commands/import/mod.rs", 0),
    ("commands/import/obsidian/create.rs", 0),
    ("commands/import/obsidian/frontmatter.rs", 0),
    ("commands/import/obsidian/manifest.rs", 0),
    ("commands/import/obsidian/mapping.rs", 0),
    ("commands/import/obsidian/mod.rs", 0),
    ("commands/import/obsidian/parser.rs", 0),
    ("commands/import/obsidian/plan.rs", 0),
    ("commands/members.rs", 0),
    ("commands/mod.rs", 0),
    ("commands/platform_status_templates.rs", 0),
    ("commands/projects.rs", 0),
    ("commands/property_definitions.rs", 0),
    ("commands/saved_searches.rs", 0),
    ("commands/search.rs", 0),
    ("commands/status_templates.rs", 0),
    ("commands/tags.rs", 0),
    ("commands/task_views.rs", 0),
    ("commands/tasks.rs", 0),
    ("commands/trash.rs", 0),
    ("commands/users.rs", 0),
    ("commands/workspaces.rs", 0),
    ("component.rs", 0),
    ("config.rs", 0),
    ("ctx.rs", 0),
    ("error.rs", 0),
    ("main.rs", 0),
    ("output.rs", 0),
    ("projections.rs", 0),
];

/// Every `crates/atlas_mcp/src/**/*.rs` file's flat-call count. Through
/// PR11c only `lib.rs` was measured (PR7 flipped its 128 sites to 0);
/// `catalog.rs`, `main.rs`, and `response.rs` are now pinned as well.
const ATLAS_MCP_PINS: &[(&str, usize)] = &[
    ("catalog.rs", 0),
    ("lib.rs", 0),
    ("main.rs", 0),
    ("response.rs", 0),
];

/// Every `crates/atlas_client/src/*.rs` file's flat-call count except the
/// namespace sources [`derive_method_namespace_map`] is built from
/// (`lib.rs` and every [`SPLIT_PRODUCTION_FILES`] entry). `helpers.rs`'s
/// six sites were namespaced in PR5 (T5.7).
const ATLAS_CLIENT_PINS: &[(&str, usize)] = &[("helpers.rs", 0)];

/// Every `crates/atlas_server/tests/**/*.rs` file's flat-call count,
/// including `support/`, pinned individually (design T2.4: "pin each file's
/// count individually, not as one crate-wide sum, so PR8-PR11's per-file
/// migration can flip pins one file at a time"). This guard's own file is
/// the only exclusion. A file on disk with no entry here, or an entry with
/// no file on disk, fails
/// [`every_consumer_root_pin_table_equals_its_directory_walk`].
const ATLAS_SERVER_TEST_PINS: &[(&str, usize)] = &[
    ("acta_boards_tasks_set_schema.rs", 0),
    ("acta_comments_events_tags_set_schema.rs", 0),
    ("acta_custos_raw_join_inventory.rs", 0),
    ("acta_documents_set_schema.rs", 0),
    ("acta_identity_workspaces_set_schema.rs", 0),
    ("acta_search_attachments_lifecycle_set_schema.rs", 0),
    ("anchor_interval_divergence.rs", 0),
    ("api_401_sweep.rs", 0),
    ("api_account_status.rs", 0),
    ("api_acta_router_parity.rs", 0),
    ("api_activation.rs", 0),
    ("api_audit_read.rs", 0),
    ("api_audit_writes.rs", 0),
    ("api_auth.rs", 0),
    ("api_automation_rules.rs", 0),
    ("api_boards_tasks.rs", 0),
    ("api_capability_sweep.rs", 0),
    ("api_comment_attachments.rs", 0),
    ("api_comments.rs", 0),
    ("api_copy.rs", 0),
    ("api_create_workspace.rs", 0),
    ("api_csrf_ratelimit.rs", 0),
    ("api_custos_router_parity.rs", 0),
    ("api_doctor.rs", 0),
    ("api_document_comments.rs", 0),
    ("api_documents.rs", 0),
    ("api_events_stream.rs", 0),
    ("api_extractor.rs", 0),
    ("api_folders.rs", 0),
    ("api_global_admin_bypass.rs", 0),
    ("api_grants.rs", 0),
    ("api_group_grants.rs", 0),
    ("api_groups.rs", 0),
    ("api_health.rs", 0),
    ("api_integration_configs.rs", 0),
    ("api_key_grant_access.rs", 0),
    ("api_key_revoke_split.rs", 0),
    ("api_key_scope_gate.rs", 0),
    ("api_key_scope_migration.rs", 0),
    ("api_login_rate_limit.rs", 0),
    ("api_members.rs", 0),
    ("api_meta_contract.rs", 0),
    ("api_page_conformance.rs", 0),
    ("api_path_literal_guard.rs", 0),
    ("api_permissions.rs", 0),
    ("api_platform_router_parity.rs", 0),
    ("api_platform_status_templates.rs", 0),
    ("api_presence.rs", 0),
    ("api_presence_agent.rs", 0),
    ("api_presence_document.rs", 0),
    ("api_projects.rs", 0),
    ("api_property_definitions.rs", 0),
    ("api_rate_limit.rs", 0),
    ("api_readiness.rs", 0),
    ("api_rfc9457_sweep.rs", 0),
    ("api_route_exclusion_list.rs", 0),
    ("api_router_mount_assertion.rs", 0),
    ("api_saved_searches.rs", 0),
    ("api_search.rs", 0),
    ("api_search_pagination.rs", 0),
    ("api_search_permissions.rs", 0),
    ("api_self_protection.rs", 0),
    ("api_semantic_search.rs", 0),
    ("api_settings.rs", 0),
    ("api_status_templates.rs", 0),
    ("api_subtasks.rs", 0),
    ("api_system_admin.rs", 0),
    ("api_tags.rs", 0),
    ("api_task_attachments.rs", 0),
    ("api_task_views.rs", 0),
    ("api_tenancy.rs", 0),
    ("api_trash.rs", 0),
    ("api_ui_state.rs", 0),
    ("api_unmatched_path_fallback.rs", 0),
    ("api_user_api_keys.rs", 0),
    ("api_users.rs", 0),
    ("api_webhooks.rs", 0),
    ("api_workspace_activity.rs", 0),
    ("api_workspace_attachments.rs", 0),
    ("api_workspace_tasks.rs", 0),
    ("api_workspaces.rs", 0),
    ("app_state.rs", 0),
    ("atlas_client_route_contract.rs", 0),
    ("attachment_store.rs", 0),
    ("authz_policy.rs", 0),
    ("automation_rule_repo.rs", 0),
    ("automation_service.rs", 0),
    ("board_res_extractor.rs", 0),
    ("boards.rs", 0),
    ("bootstrap.rs", 0),
    ("cli_mcp_component_derivation.rs", 0),
    ("comment_attachment_draft_repos.rs", 0),
    ("comment_freedom_migration.rs", 0),
    ("comment_mutation_service.rs", 0),
    ("comment_repo.rs", 0),
    ("comment_service.rs", 0),
    ("composed_migrator.rs", 0),
    ("config_registry_composition.rs", 0),
    ("custos_set_schema_migration.rs", 0),
    ("dead_cascade_evidence.rs", 0),
    ("desktop_gate_support.rs", 0),
    ("dispatcher.rs", 0),
    ("document_links.rs", 0),
    ("document_repo_ext.rs", 0),
    ("document_res_extractor.rs", 0),
    ("document_slug_migration.rs", 0),
    ("documents.rs", 0),
    ("e2e_incoming_automation.rs", 0),
    ("e2e_webhooks.rs", 0),
    ("embeddings_semantic_search.rs", 0),
    ("env_binding.rs", 0),
    ("error_model.rs", 0),
    ("grant_diagnostics.rs", 0),
    ("grant_hygiene.rs", 0),
    ("grant_resource_ref_migration.rs", 0),
    ("harness_self_test.rs", 0),
    ("idempotency_declare_and_verify_audit.rs", 0),
    ("idempotency_keys_migration.rs", 0),
    ("idempotency_live_sweep.rs", 0),
    ("idempotency_middleware.rs", 0),
    ("idempotency_repo.rs", 0),
    ("identity.rs", 0),
    ("integration_config_repo.rs", 0),
    ("lifecycle_audit.rs", 0),
    ("lifecycle_persistence.rs", 0),
    ("live_events.rs", 0),
    ("migration_slug_backfill.rs", 0),
    ("openapi_idempotency_annotations.rs", 0),
    ("openapi_semantic_contracts.rs", 0),
    ("openapi_zero_drift.rs", 0),
    ("outbox_repo.rs", 0),
    ("permission_grant_repo.rs", 0),
    ("purge_audit_string_guard.rs", 0),
    ("recoverable_deletion_migration.rs", 0),
    ("reg5_idempotent_rule_conformance.rs", 0),
    ("reg5_registry_build.rs", 0),
    ("schema_constraints.rs", 0),
    ("schema_qualification_gate.rs", 0),
    ("search_infra.rs", 0),
    ("search_repo.rs", 0),
    ("security_audit_repo.rs", 0),
    ("semantic_index_pipeline.rs", 0),
    ("semantic_search_migration.rs", 0),
    ("semantic_search_repo.rs", 0),
    ("shutdown_drain.rs", 0),
    ("startup_gate.rs", 0),
    ("support/mod.rs", 0),
    ("support/path.rs", 0),
    ("support/route_matrix.rs", 0),
    ("support/scan.rs", 0),
    ("task_references.rs", 0),
    ("task_repos_ext.rs", 0),
    ("task_service.rs", 0),
    ("tasks.rs", 0),
    ("tenancy.rs", 0),
    ("ui_state_repo_characterization.rs", 0),
    ("visibility_ancestors.rs", 0),
    ("workspace_core.rs", 0),
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
fn every_consumer_file_matches_its_pinned_count() {
    let derived = derive_method_namespace_map();
    let mut mismatches = Vec::new();

    for root in CONSUMER_ROOTS {
        let pins: HashMap<&str, usize> = root.pins.iter().copied().collect();

        for (key, measured) in measure_root(&root.path(), root.excluded, &derived.map) {
            let label = format!("{}/{key}", root.dir);
            match pins.get(key.as_str()) {
                Some(&pinned) => {
                    if let Err(message) = pin_check(&label, pinned, measured) {
                        mismatches.push(message);
                    }
                }
                None => {
                    mismatches.push(format!("{label}: measured {measured} but has no pin entry"))
                }
            }
        }
    }

    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn every_consumer_root_pin_table_equals_its_directory_walk() {
    let mut problems = Vec::new();

    for root in CONSUMER_ROOTS {
        let on_disk: Vec<String> = root.files().into_iter().map(|(key, _)| key).collect();
        let pinned: Vec<&str> = root.pins.iter().map(|(name, _)| *name).collect();

        for key in &on_disk {
            if !pinned.contains(&key.as_str()) {
                problems.push(format!(
                    "{}/{key}: on disk but missing from the pin table",
                    root.dir
                ));
            }
        }

        for name in &pinned {
            if !on_disk.iter().any(|key| key == name) {
                problems.push(format!(
                    "{}/{name}: in the pin table but not on disk",
                    root.dir
                ));
            }
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
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
        .filter_map(|(namespace, method)| match map.get(method) {
            Some(home) if namespace == home => None,
            Some(home) => Some(format!(
                "client.{namespace}().{method}(..) — {method}'s declared home is {home}"
            )),
            None => Some(format!(
                "client.{namespace}().{method}(..) — {method} is not a mapped AtlasClient method"
            )),
        })
        .collect()
}

/// T11.3 — the completeness gate PR12's shim deletion depends on. Through
/// PR11a and PR11b this module carried a growing "not yet migrated" sweep
/// (`no_namespaced_call_site_exists_anywhere_yet_outside_...`) that named an
/// ever-longer exclusion list of already-migrated files, one PR at a time.
/// By PR11c every pinned crate and file (`atlas_cli`, `atlas_mcp`,
/// `atlas_client::helpers`, and all 49 nonzero-at-some-point
/// `ATLAS_SERVER_TEST_PINS` entries) carries its own dedicated reverse-check
/// test, so a hand-maintained exclusion list is no longer the right shape:
/// [`every_forward_pin_is_zero_the_completeness_gate`] asserts the
/// migration's actual completion condition directly, and
/// [`a_flat_call_anywhere_in_the_consumer_set_would_still_be_caught`] proves
/// the forward-check machinery underneath it still works.
#[test]
fn every_forward_pin_is_zero_the_completeness_gate() {
    let nonzero: Vec<String> = CONSUMER_ROOTS
        .iter()
        .flat_map(|root| {
            root.pins
                .iter()
                .filter(|(_, pinned)| *pinned != 0)
                .map(move |(name, pinned)| format!("{}/{name} = {pinned}", root.dir))
        })
        .collect();

    assert!(
        nonzero.is_empty(),
        "every consumer file's forward pin must be 0 once PR11c lands: {nonzero:?}"
    );
}

/// Self-test proving the file set is the directory walk, not a heuristic: a
/// fixture file that never spells `AtlasClient` (it receives a client from a
/// `support::login_*`-shaped helper, as most `atlas_server/tests` files do)
/// but carries one flat call is walked and measured as 1, a sibling under a
/// nested directory with no call at all is still walked and measured as 0,
/// and a non-`.rs` file is not walked.
#[test]
fn a_file_without_the_atlas_client_literal_is_still_walked_and_counted() {
    let derived = derive_method_namespace_map();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "client_call_shape_guard_{}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(dir.join("support")).expect("create fixture dir");

    let fixture = "async fn flow(server: &TestServer) {\n\
                   \x20   let client = support::login_admin(server).await;\n\
                   \x20   let response = client.get_task(ws, id).await?;\n\
                   }\n";
    fs::write(dir.join("api_fixture.rs"), fixture).expect("write fixture");
    fs::write(dir.join("support/mod.rs"), "pub fn noop() {}\n").expect("write sibling");
    fs::write(dir.join("notes.txt"), "client.get_task(ws, id)\n").expect("write non-rust file");

    let measured = measure_root(&dir, &[], &derived.map);
    fs::remove_dir_all(&dir).expect("remove fixture dir");

    assert!(
        !Regex::new(r"\bAtlasClient\b").unwrap().is_match(fixture),
        "the fixture must not spell the literal the old gate keyed on"
    );
    assert_eq!(
        measured,
        vec![
            ("api_fixture.rs".to_string(), 1),
            ("support/mod.rs".to_string(), 0)
        ]
    );
}

/// Self-test proving [`every_forward_pin_is_zero_the_completeness_gate`] is
/// not vacuously true because the detection machinery stopped working: a
/// synthetic flat call site, run through the same [`flat_call_count_all`]
/// the real forward checks use, is still counted as nonzero.
#[test]
fn a_flat_call_anywhere_in_the_consumer_set_would_still_be_caught() {
    let derived = derive_method_namespace_map();
    let fixture = "let response = client.get_task(ws, id).await?;";

    let measured = flat_call_count_all(fixture, &derived.map);

    assert_eq!(
        measured, 1,
        "a synthetic flat call site must still be counted by the forward-check \
         machinery this completeness gate relies on"
    );
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
fn api_account_status_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_account_status.rs"),
    ));

    assert_eq!(
        sites.len(),
        20,
        "expected 20 namespaced sites in crates/atlas_server/tests/api_account_status.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_activation_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_activation.rs"),
    ));

    assert_eq!(
        sites.len(),
        1,
        "expected 1 namespaced sites in crates/atlas_server/tests/api_activation.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_audit_read_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_audit_read.rs"),
    ));

    assert_eq!(
        sites.len(),
        14,
        "expected 14 namespaced sites in crates/atlas_server/tests/api_audit_read.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_auth_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_auth.rs"),
    ));

    assert_eq!(
        sites.len(),
        8,
        "expected 8 namespaced sites in crates/atlas_server/tests/api_auth.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_copy_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_copy.rs"),
    ));

    assert_eq!(
        sites.len(),
        13,
        "expected 13 namespaced sites in crates/atlas_server/tests/api_copy.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_create_workspace_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_create_workspace.rs"),
    ));

    assert_eq!(
        sites.len(),
        7,
        "expected 7 namespaced sites in crates/atlas_server/tests/api_create_workspace.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_doctor_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_doctor.rs"),
    ));

    assert_eq!(
        sites.len(),
        7,
        "expected 7 namespaced sites in crates/atlas_server/tests/api_doctor.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_document_comments_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_document_comments.rs"),
    ));

    assert_eq!(
        sites.len(),
        25,
        "expected 25 namespaced sites in crates/atlas_server/tests/api_document_comments.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_events_stream_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_events_stream.rs"),
    ));

    assert_eq!(
        sites.len(),
        6,
        "expected 6 namespaced sites in crates/atlas_server/tests/api_events_stream.rs (PR11c; \
         one of the file's 7 flat-call-shaped sites is PgBoardRepo::create_board, never migrated), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_extractor_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_extractor.rs"),
    ));

    assert_eq!(
        sites.len(),
        3,
        "expected 3 namespaced sites in crates/atlas_server/tests/api_extractor.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_global_admin_bypass_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_global_admin_bypass.rs"),
    ));

    assert_eq!(
        sites.len(),
        9,
        "expected 9 namespaced sites in crates/atlas_server/tests/api_global_admin_bypass.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_permissions_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_permissions.rs"),
    ));

    assert_eq!(
        sites.len(),
        26,
        "expected 26 namespaced sites in crates/atlas_server/tests/api_permissions.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_platform_status_templates_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_platform_status_templates.rs"),
    ));

    assert_eq!(
        sites.len(),
        22,
        "expected 22 namespaced sites in crates/atlas_server/tests/api_platform_status_templates.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_presence_agent_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_presence_agent.rs"),
    ));

    assert_eq!(
        sites.len(),
        5,
        "expected 5 namespaced sites in crates/atlas_server/tests/api_presence_agent.rs (PR11c; \
         one of the file's 6 flat-call-shaped sites is PgBoardRepo::create_board, never migrated), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_presence_document_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_presence_document.rs"),
    ));

    assert_eq!(
        sites.len(),
        2,
        "expected 2 namespaced sites in crates/atlas_server/tests/api_presence_document.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_property_definitions_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_property_definitions.rs"),
    ));

    assert_eq!(
        sites.len(),
        26,
        "expected 26 namespaced sites in crates/atlas_server/tests/api_property_definitions.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_search_permissions_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_search_permissions.rs"),
    ));

    assert_eq!(
        sites.len(),
        0,
        "expected 0 namespaced sites in crates/atlas_server/tests/api_search_permissions.rs \
         (PR11c; the file's only client binding calls login(), which never moves to a \
         sub-client, and token(), which is not a mapped method; both of the file's 2 \
         flat-call-shaped sites — PgBoardRepo::create_board and PgSearchRepo::search — are \
         never migrated), found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_self_protection_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_self_protection.rs"),
    ));

    assert_eq!(
        sites.len(),
        18,
        "expected 18 namespaced sites in crates/atlas_server/tests/api_self_protection.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_semantic_search_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_semantic_search.rs"),
    ));

    assert_eq!(
        sites.len(),
        3,
        "expected 3 namespaced sites in crates/atlas_server/tests/api_semantic_search.rs (PR11c; \
         two of the file's 5 flat-call-shaped sites are PgBoardRepo::create_board, never migrated), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_system_admin_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_system_admin.rs"),
    ));

    assert_eq!(
        sites.len(),
        16,
        "expected 16 namespaced sites in crates/atlas_server/tests/api_system_admin.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_tenancy_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_tenancy.rs"),
    ));

    assert_eq!(
        sites.len(),
        9,
        "expected 9 namespaced sites in crates/atlas_server/tests/api_tenancy.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_trash_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_trash.rs"),
    ));

    assert_eq!(
        sites.len(),
        1,
        "expected 1 namespaced site in crates/atlas_server/tests/api_trash.rs (PR11c; \
         one of the file's 2 flat-call-shaped sites is PgBoardRepo::create_board, never migrated), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_ui_state_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_ui_state.rs"),
    ));

    assert_eq!(
        sites.len(),
        11,
        "expected 11 namespaced sites in crates/atlas_server/tests/api_ui_state.rs (PR11c), \
         found: {sites:?}"
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn api_users_namespaced_sites_match_their_declared_home() {
    let derived = derive_method_namespace_map();
    let sites = namespaced_call_sites(&masked_code(
        &repo_root().join("crates/atlas_server/tests/api_users.rs"),
    ));

    assert_eq!(
        sites.len(),
        19,
        "expected 19 namespaced sites in crates/atlas_server/tests/api_users.rs (PR11c), \
         found: {sites:?}"
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

/// PR12 orchestrator note: a namespaced call to a method absent from the
/// derived map must itself be a mismatch, not silently skipped — an unknown
/// method name is never a legitimate namespaced call.
#[test]
fn a_call_to_a_method_absent_from_the_map_is_flagged_as_a_mismatch() {
    let derived = derive_method_namespace_map();
    let fixture = "let response = client.acta().not_a_real_method(ws, id).await?;";
    let sites = namespaced_call_sites(fixture);
    assert_eq!(
        sites,
        vec![("acta".to_string(), "not_a_real_method".to_string())]
    );

    let mismatches = reverse_check_mismatches(&sites, &derived.map);
    assert_eq!(mismatches.len(), 1);
    let mismatch = mismatches.first().expect("checked len() == 1 above");
    assert!(mismatch.contains("not_a_real_method"));
    assert!(mismatch.contains("acta"));
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

/// PR11c: `PgBoardRepo::new(db.conn().clone())`, `db.board_repo()`, and a
/// binding declared from either of those shapes in the same file are the
/// verbatim receiver shapes this codebase uses to call
/// `PgBoardRepo::create_board` or `PgSearchRepo::search` directly — never
/// an `AtlasClient` call, since `create_board` and `search` are also
/// `Acta`-homed client method names. None of these shapes is counted; a
/// genuine client binding still is, including one that happens to be
/// named `repo` but has no repository provenance.
#[test]
fn a_pg_repo_receiver_is_never_counted_but_a_client_binding_is() {
    assert_eq!(
        flat_call_count(
            "PgBoardRepo::new(db.conn().clone())\n        .create_board(&ctx, new_board)",
            "create_board"
        ),
        0
    );
    assert_eq!(
        flat_call_count(
            "db\n        .board_repo()\n        .create_board(&ctx, new_board)",
            "create_board"
        ),
        0
    );
    assert_eq!(
        flat_call_count(
            "db.board_repo().create_board(&ctx, new_board)",
            "create_board"
        ),
        0
    );

    // A binding declared from a repository constructor or accessor in the
    // same code is a repository receiver.
    assert_eq!(
        flat_call_count(
            "let repo = PgBoardRepo::new(db);\n    let board = repo.create_board(&ctx, new_board);",
            "create_board"
        ),
        0
    );
    assert_eq!(
        flat_call_count(
            "let board_repo = PgBoardRepo::new(db.conn().clone());\n    let board = board_repo\n        .create_board(ctx, new_board)",
            "create_board"
        ),
        0
    );
    assert_eq!(
        flat_call_count(
            "let repo = PgSearchRepo::new(db.conn().clone());\n    let hits = repo\n        .search(&ctx, &principal, &query)",
            "search"
        ),
        0
    );
    assert_eq!(
        flat_call_count(
            "let board_repo = db.board_repo();\n    board_repo.create_board(&ctx, new_board)",
            "create_board"
        ),
        0
    );

    // The same names with no repository provenance are ordinary receivers:
    // an `AtlasClient` bound as `repo` must still count.
    assert_eq!(
        flat_call_count(
            "let repo = login(&server, \"alice\").await;\n    repo.create_board(&ws, new_board)",
            "create_board"
        ),
        1
    );
    assert_eq!(
        flat_call_count(
            "board_repo\n        .create_board(ctx, new_board)",
            "create_board"
        ),
        1
    );
    assert_eq!(flat_call_count("repo.search(&ws, &query)", "search"), 1);

    // Namespaced and flat client calls behave as everywhere else.
    assert_eq!(
        flat_call_count("client.acta().create_board(&ws, new_board)", "create_board"),
        0
    );
    assert_eq!(
        flat_call_count("client.create_board(&ws, new_board)", "create_board"),
        1
    );
    assert_eq!(flat_call_count("client.search(&ws, &query)", "search"), 1);
}

/// PR12 orchestrator decision O2: a service receiver is excluded by the
/// same provenance rule a repository receiver is, and a binding of the same
/// name from any other expression still counts.
#[test]
fn a_service_receiver_is_never_counted_but_a_client_binding_of_the_same_name_is() {
    let service_bound = "let svc = TaskService::new(db);\nsvc.delete_task(&ctx, task.id).await?;\n";
    let accessor_bound = "let svc = db.task_service();\nsvc.delete_task(&ctx, task.id).await?;\n";
    let direct_constructor = "TaskService::new(db).delete_task(&ctx, task.id).await?;\n";
    let direct_accessor = "db.task_service().delete_task(&ctx, task.id).await?;\n";
    let client_bound =
        "let svc = login(&server, \"alice\").await;\nsvc.delete_task(ws, id).await?;\n";

    assert_eq!(flat_call_count(service_bound, "delete_task"), 0);
    assert_eq!(flat_call_count(accessor_bound, "delete_task"), 0);
    assert_eq!(flat_call_count(direct_constructor, "delete_task"), 0);
    assert_eq!(flat_call_count(direct_accessor, "delete_task"), 0);
    assert_eq!(flat_call_count(client_bound, "delete_task"), 1);
    assert_eq!(repo_bound_identifiers(service_bound), vec!["svc"]);
    assert!(repo_bound_identifiers(client_bound).is_empty());
}

/// PR12 orchestrator decision: provenance follows a same-file constructor
/// helper one hop, keyed on the helper's declared return type, never on a
/// name.
#[test]
fn a_same_file_helper_returning_a_service_marks_its_binding_but_a_client_helper_does_not() {
    let service_helper = "fn make_svc(db: &TestDb) -> TaskService {\n    TaskService::new(db.conn().clone())\n}\n\
                          let svc = make_svc(&db);\nsvc.delete_task(&ctx, task.id).await?;\n";
    let qualified_repo_helper = "fn make_doc_repo(db: &TestDb, n: u32) -> repos::PgDocumentRepo {\n    todo!()\n}\n\
                                 let repo = make_doc_repo(&db, 50);\nrepo.update_content(&ctx, id, rev, \"v2\").await?;\n";
    let client_helper = "async fn make_client(server: &TestServer) -> AtlasClient {\n    login(server, \"alice\").await\n}\n\
                         let c = make_client(&server).await;\nc.delete_task(ws, id).await?;\n";

    assert_eq!(repo_constructor_helpers(service_helper), vec!["make_svc"]);
    assert_eq!(flat_call_count(service_helper, "delete_task"), 0);
    assert_eq!(flat_call_count(qualified_repo_helper, "update_content"), 0);
    assert!(repo_constructor_helpers(client_helper).is_empty());
    assert_eq!(flat_call_count(client_helper, "delete_task"), 1);
}

#[test]
fn repo_bound_identifiers_follow_provenance_not_names() {
    let code = "let repo = PgSearchRepo::new(db.conn().clone());\n\
                let mut board_repo = db.board_repo();\n\
                let typed: PgBoardRepo = PgBoardRepo::new(db);\n\
                let client = login(&server, \"alice\").await;\n\
                let repo = PgSearchRepo::new(db.conn().clone());\n";

    assert_eq!(
        repo_bound_identifiers(code),
        vec!["board_repo", "repo", "typed"]
    );
    assert!(repo_bound_identifiers("let repo = login(&server, \"alice\").await;").is_empty());
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
