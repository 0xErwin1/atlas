#![allow(dead_code, clippy::panic)]

//! `v2-e11-s5` PR6a (design D7.4): `AtlasClient` method name ⇒
//! `(component, method, path_template)`, the lookup `cli_mcp_openapi_coverage.rs`
//! composes CLI commands and MCP operations against.
//!
//! **Not an independent implementation.** This extractor shares its parsing
//! helpers (`support::source_walk`: string-aware paren matching, function
//! boundaries, `let`-binding and owning-helper resolution) with the same
//! functions `atlas_client_route_contract.rs` carries privately, and walks
//! the same four `crates/atlas_client/src/*.rs` files. What differs is the
//! product: the route contract extracts call sites to check them against
//! the registry, this module keys the same resolutions by method name so a
//! surface walk can look a call up. A divergence between the two — one
//! changed without the other — is caught by cardinality, not by
//! independence: [`CROSS_CHECK_EXTRACTED_CALL_COUNT`] must equal the route
//! contract's own pinned extracted-call count.
//!
//! **Cross-check.** `atlas_client_route_contract.rs`'s own
//! `extracted_call_count_is_pinned` test pins its extractor's total resolved
//! call count to 194 across `lib.rs`/`custos.rs`/`acta.rs`/`platform.rs`.
//! Since each test file compiles as an independent binary crate, this module
//! cannot read that constant directly; [`CROSS_CHECK_EXTRACTED_CALL_COUNT`]
//! is this module's own copy of that same number, and this module's own
//! test asserts the number of *verb-bearing* routes equals it.
//!
//! **Delegating wrappers.** A public `AtlasClient` method whose body has no
//! verb call but calls another method of the same file (`create_task` ⇒
//! `create_task_with_references`, `list_documents` ⇒
//! `list_documents_with_unfiled_filter` ⇒ `list_documents_with_options`)
//! reaches a route only through its delegate. The route contract counts
//! call sites, so it never sees a wrapper; a surface walk that calls the
//! wrapper by name (`atlas tasks create`, `atlas docs list`) does. This
//! module follows `self.<method>(` delegation transitively (cycle-guarded,
//! stopping at the first verb-bearing method) and emits the delegate's
//! route under the wrapper's own name, flagged `delegated`. The two counts
//! are pinned separately: the 194 verb-bearing routes are the cross-check
//! against the route contract, and [`WRAPPER_ROUTE_COUNT`] is the number of
//! wrappers on top of them, so `client_routes()` carries `194 + 3` method
//! names. A wrapper whose chain never reaches a verb call, or reaches more
//! than one distinct route, is an extraction failure naming the wrapper,
//! never a silently missing route.

use std::fs;
use std::path::{Path, PathBuf};

use atlas_core::registry::HttpMethod;
use regex::Regex;

use super::source_walk::{
    capitalize, enclosing_fn_name, first_path_literal, function_body, function_boundaries,
    is_pub_fn, match_paren, resolve_relative, self_call_targets, split_top_level_comma,
    truncate_at_test_module,
};

/// The route contract's own pinned extracted-call-count (`v2-e11-s4`,
/// `atlas_client_route_contract.rs::extracted_call_count_is_pinned`), copied
/// here because the two test binaries cannot share a constant. See this
/// module's own doc comment for why equal cardinality is the cross-check.
pub(crate) const CROSS_CHECK_EXTRACTED_CALL_COUNT: usize = 195;

/// The number of delegating wrappers (public methods with no verb call of
/// their own that reach a route through another method of the same file)
/// across the four mapped files. These are invisible to the route contract's
/// call-site count, so they are pinned apart from
/// [`CROSS_CHECK_EXTRACTED_CALL_COUNT`]; `client_routes()` yields the sum.
pub(crate) const WRAPPER_ROUTE_COUNT: usize = 3;

/// One `AtlasClient` method resolved to the route it calls. `delegated` is
/// `true` when the method has no verb call of its own and the route is the
/// one its delegation chain ends at.
#[derive(Debug, Clone)]
pub(crate) struct ClientRoute {
    pub(crate) method_name: String,
    pub(crate) component: String,
    pub(crate) http_method: HttpMethod,
    pub(crate) path_template: String,
    pub(crate) delegated: bool,
}

/// One mapped file's extraction: its resolved routes and every delegating
/// wrapper that could not be resolved to exactly one route.
#[derive(Debug, Default)]
pub(crate) struct Extraction {
    pub(crate) routes: Vec<ClientRoute>,
    pub(crate) failures: Vec<String>,
}

fn client_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../atlas_client/src")
}

fn read_client_source(file_name: &str) -> String {
    let path = client_src_dir().join(file_name);
    let content = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    truncate_at_test_module(&content)
}

/// The four production files this extractor walks, and the component owned
/// by every non-mixed one of them (mirrors
/// `atlas_client_route_contract.rs::PRODUCTION_SOURCE_MAPPINGS`).
const MAPPED_FILES: &[(&str, Option<&str>)] = &[
    ("lib.rs", None), // mixed: component resolved per call
    ("custos.rs", Some("custos")),
    ("acta.rs", Some("acta")),
    ("platform.rs", Some("platform")),
];

const COMPONENTS: &[&str] = &["platform", "custos", "acta"];

/// Every `self.<verb>(..)`/`self.root_get(..)` call in `source`, resolved to
/// a `ClientRoute` when `file_component` is known (non-mixed file) or when
/// the call itself carries a `Component::X` literal (mixed `lib.rs`).
/// `root_get` calls are attributed to `platform` — the only component whose
/// namespace serves an unprefixed root-level path. `root_source` is
/// `lib.rs`, where an owning path-builder helper's body is resolved when it
/// is not in `source` itself.
fn extract_verb_routes(
    source: &str,
    file_component: Option<&str>,
    root_source: &str,
) -> Vec<ClientRoute> {
    let boundaries = function_boundaries(source);
    let mut routes = Vec::new();

    for verb in ["get", "post", "patch", "put", "delete"] {
        let re = Regex::new(&format!(r"self\s*\.\s*{verb}\s*\(")).expect("valid regex");
        for m in re.find_iter(source) {
            let open = m.end() - 1;
            let close = match_paren(source, open);
            let arg_text = &source[open + 1..close];
            let fn_name = enclosing_fn_name(&boundaries, m.start());
            let fn_body =
                function_body(source, &boundaries, &fn_name).unwrap_or(&source[m.start()..close]);

            let (component, relative) = match split_top_level_comma(arg_text) {
                Some((component_expr, relative_expr)) => {
                    let component_expr = component_expr.trim();
                    let literal_component = COMPONENTS
                        .iter()
                        .find(|name| component_expr == format!("Component::{}", capitalize(name)))
                        .map(|name| name.to_string());
                    let relative =
                        resolve_relative(&relative_expr, fn_body, &boundaries, source, root_source);
                    (literal_component, relative)
                }
                None => (
                    None,
                    resolve_relative(arg_text, fn_body, &boundaries, source, root_source),
                ),
            };

            let http_method = match verb {
                "get" => HttpMethod::Get,
                "post" => HttpMethod::Post,
                "patch" => HttpMethod::Patch,
                "put" => HttpMethod::Put,
                "delete" => HttpMethod::Delete,
                _ => unreachable!(),
            };

            let resolved_component = file_component.map(str::to_string).or(component);

            if let (Some(resolved_component), Some(path_template)) = (resolved_component, relative)
            {
                routes.push(ClientRoute {
                    method_name: fn_name,
                    component: resolved_component,
                    http_method,
                    path_template,
                    delegated: false,
                });
            }
        }
    }

    let root_re = Regex::new(r"self\s*\.\s*root_get\s*\(").expect("valid regex");
    for m in root_re.find_iter(source) {
        let open = m.end() - 1;
        let close = match_paren(source, open);
        let arg_text = &source[open + 1..close];
        let fn_name = enclosing_fn_name(&boundaries, m.start());

        if let Some(path_template) = first_path_literal(arg_text) {
            routes.push(ClientRoute {
                method_name: fn_name,
                component: "platform".to_string(),
                http_method: HttpMethod::Get,
                path_template,
                delegated: false,
            });
        }
    }

    routes
}

/// The distinct `(component, method, path)` routes `method_name`'s
/// delegation chain ends at: its own verb routes when it has any, otherwise
/// the union over every same-file method it calls through `self.<m>(`.
/// `visited` breaks cycles; a cycle contributes nothing, so a wrapper whose
/// chain only loops resolves to no route and is reported by the caller.
fn delegated_routes(
    method_name: &str,
    source: &str,
    boundaries: &[(usize, String)],
    verb_routes: &[ClientRoute],
    visited: &mut Vec<String>,
) -> Vec<(String, HttpMethod, String)> {
    let own: Vec<(String, HttpMethod, String)> = verb_routes
        .iter()
        .filter(|route| route.method_name == method_name)
        .map(|route| {
            (
                route.component.clone(),
                route.http_method,
                route.path_template.clone(),
            )
        })
        .collect();

    if !own.is_empty() {
        return own;
    }

    let Some(body) = function_body(source, boundaries, method_name) else {
        return Vec::new();
    };

    let mut resolved: Vec<(String, HttpMethod, String)> = Vec::new();

    for callee in self_call_targets(body) {
        let is_method = boundaries.iter().any(|(_, name)| *name == callee);
        if !is_method || visited.contains(&callee) {
            continue;
        }

        visited.push(callee.clone());
        for route in delegated_routes(&callee, source, boundaries, verb_routes, visited) {
            if !resolved.contains(&route) {
                resolved.push(route);
            }
        }
    }

    resolved
}

/// Every public method of `source` with no verb route of its own that
/// delegates to a same-file method: resolved to its chain's single route
/// under its own name, or reported as a failure when the chain reaches no
/// route or more than one.
fn extract_wrapper_routes(
    source: &str,
    boundaries: &[(usize, String)],
    verb_routes: &[ClientRoute],
) -> Extraction {
    let mut extraction = Extraction::default();

    for (offset, method_name) in boundaries {
        let has_own_route = verb_routes.iter().any(|r| r.method_name == *method_name);
        if has_own_route || !is_pub_fn(source, *offset) {
            continue;
        }

        let Some(body) = function_body(source, boundaries, method_name) else {
            continue;
        };
        let delegates: Vec<String> = self_call_targets(body)
            .into_iter()
            .filter(|callee| boundaries.iter().any(|(_, name)| name == callee))
            .collect();
        if delegates.is_empty() {
            continue;
        }

        let mut visited = vec![method_name.clone()];
        let resolved = delegated_routes(method_name, source, boundaries, verb_routes, &mut visited);

        match resolved.as_slice() {
            [(component, http_method, path_template)] => extraction.routes.push(ClientRoute {
                method_name: method_name.clone(),
                component: component.clone(),
                http_method: *http_method,
                path_template: path_template.clone(),
                delegated: true,
            }),
            [] => extraction.failures.push(format!(
                "wrapper `{method_name}` delegates to {delegates:?} but its chain never \
                 reaches a `self.<verb>(` call"
            )),
            many => extraction.failures.push(format!(
                "wrapper `{method_name}` delegates to {delegates:?} and its chain reaches \
                 {} distinct routes, not one",
                many.len()
            )),
        }
    }

    extraction
}

/// Both passes over one mapped file: verb-bearing methods first, then the
/// public wrappers that delegate to them.
fn extract_client_routes(
    source: &str,
    file_component: Option<&str>,
    root_source: &str,
) -> Extraction {
    let boundaries = function_boundaries(source);
    let verb_routes = extract_verb_routes(source, file_component, root_source);

    let mut extraction = extract_wrapper_routes(source, &boundaries, &verb_routes);
    extraction.routes.splice(0..0, verb_routes);

    extraction
}

/// The full `AtlasClient` method-name ⇒ route map, across all four mapped
/// production files. Panics naming every wrapper whose delegation chain
/// could not be resolved to exactly one route.
pub(crate) fn client_routes() -> Vec<ClientRoute> {
    let root_source = read_client_source("lib.rs");
    let mut routes = Vec::new();
    let mut failures = Vec::new();

    for (file_name, component) in MAPPED_FILES {
        let source = read_client_source(file_name);
        let extraction = extract_client_routes(&source, *component, &root_source);

        routes.extend(extraction.routes);
        failures.extend(
            extraction
                .failures
                .into_iter()
                .map(|failure| format!("{file_name}: {failure}")),
        );
    }

    assert!(
        failures.is_empty(),
        "unresolvable AtlasClient wrappers:\n{}",
        failures.join("\n")
    );

    routes
}

/// Looks up a resolved route by the `AtlasClient` method name a CLI command
/// or MCP handler calls (`client.acta().<method_name>(..)`).
pub(crate) fn find_route<'a>(
    routes: &'a [ClientRoute],
    method_name: &str,
) -> Option<&'a ClientRoute> {
    routes.iter().find(|route| route.method_name == method_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_routes_is_not_vacuous() {
        let routes = client_routes();
        assert!(
            !routes.is_empty(),
            "anti-vacuity: no client routes resolved"
        );
    }

    #[test]
    fn client_routes_cardinality_matches_the_route_contracts_pinned_extracted_call_count() {
        let routes = client_routes();
        let verb_bearing = routes.iter().filter(|route| !route.delegated).count();
        assert_eq!(
            verb_bearing, CROSS_CHECK_EXTRACTED_CALL_COUNT,
            "this module's derived verb-bearing route count diverged from \
             atlas_client_route_contract.rs's own pinned extracted-call count \
             — one of the two extractors changed without the other"
        );
    }

    #[test]
    fn wrapper_route_count_is_pinned_apart_from_the_cross_check() {
        let routes = client_routes();
        let wrappers: Vec<&str> = routes
            .iter()
            .filter(|route| route.delegated)
            .map(|route| route.method_name.as_str())
            .collect();
        assert_eq!(
            wrappers.len(),
            WRAPPER_ROUTE_COUNT,
            "delegating wrappers changed: {wrappers:?}; re-pin WRAPPER_ROUTE_COUNT and \
             re-measure both coverage tables"
        );
        assert_eq!(
            routes.len(),
            CROSS_CHECK_EXTRACTED_CALL_COUNT + WRAPPER_ROUTE_COUNT
        );
    }

    #[test]
    fn real_wrappers_resolve_to_their_delegates_routes() {
        let routes = client_routes();
        let by_name = |name: &str| {
            find_route(&routes, name).unwrap_or_else(|| panic!("`{name}` has no route"))
        };

        let create_task = by_name("create_task");
        assert!(create_task.delegated);
        assert_eq!(create_task.http_method, HttpMethod::Post);
        assert_eq!(
            create_task.path_template,
            by_name("create_task_with_references").path_template
        );

        let list_documents = by_name("list_documents");
        assert!(list_documents.delegated);
        assert_eq!(list_documents.http_method, HttpMethod::Get);
        assert_eq!(
            list_documents.path_template,
            by_name("list_documents_with_options").path_template
        );
    }

    /// Probe: a wrapper two delegation levels away from a verb-bearing
    /// method resolves to that method's route under the wrapper's name.
    #[test]
    fn probe_two_level_wrapper_resolves_to_the_verb_bearing_route() {
        let source = "impl Acta<'_> {\n\
            \x20   pub async fn probe_outer(&self, ws: &str) -> Result<(), ClientError> {\n\
            \x20       self.probe_middle(ws, false).await\n\
            \x20   }\n\
            \x20   pub async fn probe_middle(&self, ws: &str, flag: bool) -> Result<(), ClientError> {\n\
            \x20       self.probe_inner(ws, flag, None).await\n\
            \x20   }\n\
            \x20   pub async fn probe_inner(&self, ws: &str, flag: bool, cursor: Option<&str>) -> Result<(), ClientError> {\n\
            \x20       let response = self.get(Component::Acta, &format!(\"/workspaces/{ws}/probes\")).send().await?;\n\
            \x20       Ok(())\n\
            \x20   }\n\
            }\n";

        let extraction = extract_client_routes(source, Some("acta"), "");
        assert!(
            extraction.failures.is_empty(),
            "unexpected failures: {:?}",
            extraction.failures
        );

        let outer = find_route(&extraction.routes, "probe_outer").expect("probe_outer resolved");
        assert!(outer.delegated);
        assert_eq!(outer.component, "acta");
        assert_eq!(outer.http_method, HttpMethod::Get);
        assert_eq!(outer.path_template, "/workspaces/{ws}/probes");

        let middle = find_route(&extraction.routes, "probe_middle").expect("probe_middle resolved");
        assert!(middle.delegated);
        assert_eq!(middle.path_template, "/workspaces/{ws}/probes");

        let inner = find_route(&extraction.routes, "probe_inner").expect("probe_inner resolved");
        assert!(!inner.delegated);
    }

    /// Probe: a wrapper whose chain only loops, or ends at a method with no
    /// verb call, is reported by name rather than silently dropped.
    #[test]
    fn probe_unresolvable_wrapper_is_a_named_failure() {
        let source = "impl Acta<'_> {\n\
            \x20   pub async fn probe_loop_a(&self) -> Result<(), ClientError> {\n\
            \x20       self.probe_loop_b().await\n\
            \x20   }\n\
            \x20   pub async fn probe_loop_b(&self) -> Result<(), ClientError> {\n\
            \x20       self.probe_loop_a().await\n\
            \x20   }\n\
            \x20   pub fn probe_dead_end(&self) -> String {\n\
            \x20       self.probe_helper()\n\
            \x20   }\n\
            \x20   fn probe_helper(&self) -> String {\n\
            \x20       String::new()\n\
            \x20   }\n\
            }\n";

        let extraction = extract_client_routes(source, Some("acta"), "");
        assert!(extraction.routes.is_empty(), "{:?}", extraction.routes);

        for wrapper in ["probe_loop_a", "probe_loop_b", "probe_dead_end"] {
            assert!(
                extraction
                    .failures
                    .iter()
                    .any(|failure| failure.contains(&format!("`{wrapper}`"))),
                "expected a failure naming `{wrapper}`, got: {:?}",
                extraction.failures
            );
        }
    }
}
