#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `v2-e3-s6` PR2, D2.3 — binds every `atlas_client` verb call to a
//! registry-declared V2 route, in both directions, by walking the crate's
//! own source text rather than a hand-maintained list of its methods (Rust
//! has no reflection, so the file's own text is the enumeration).
//!
//! Lives in `atlas_server` because this is the only crate that sees both
//! sides: `atlas_client` does not depend on `atlas_core`/`atlas_server`, and
//! `atlas_server` already dev-depends on `atlas_client`.
//!
//! **Forward**: every `self.<verb>(Component::X, "<relative>")` call site's
//! resolved `(HttpMethod, Component, relative)` triple must match a route
//! `route_matrix()` declares for that component (D2.3, R2's mis-attribution
//! risk — a wrong `Component::` compiles and 404s at runtime with no other
//! signal). Placeholder names are normalized (`{ws}` vs `{project_slug}`
//! both become `{}`) before comparison, since the client and the registry
//! name the same positional placeholder differently at 14 custos sites
//! (D2.3's "two implementation facts").
//!
//! **Totality**: every `self.<verb>(` occurrence must carry a `Component::`
//! first argument — an unattributed call is a failure named by its
//! enclosing method, not a skip.
//!
//! **Reverse**: every registry-declared V2 route with no client call site is
//! named in [`UNCOVERED_ROUTES`] with a reason, checked bidirectionally — an
//! unlisted uncovered route fails, and a route the client now covers also
//! fails (stale entry), mirroring the grep gate's allowlist discipline.
//!
//! This test needs no database: it walks `atlas_client`'s own source text
//! and the in-process REG-5 registry, exactly like `route_matrix()`'s other
//! consumers.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use atlas_core::registry::HttpMethod;
use regex::Regex;

use support::route_matrix::{RouteMatrixEntry, route_matrix};

/// `atlas_client`'s three `Component` variants, matched against the
/// registry's own stable ids (`crate::reg5`) — the same three strings
/// `atlas_client::Component::as_str` returns.
const COMPONENTS: &[&str] = &["platform", "custos", "acta"];

fn client_lib_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../atlas_client/src/lib.rs")
}

/// The crate's source, with `#[cfg(test)] mod tests { .. }` truncated off —
/// the walker's scope is production code only (D2.4's same rule for the
/// grep gate).
fn production_source() -> String {
    let content = fs::read_to_string(client_lib_path()).expect("read atlas_client/src/lib.rs");
    truncate_at_test_module(&content)
}

fn truncate_at_test_module(content: &str) -> String {
    match content.find("#[cfg(test)]") {
        Some(index) => content[..index].to_string(),
        None => content.to_string(),
    }
}

/// One `self.<verb>(..)` or `self.root_get(..)` call site extracted from
/// `atlas_client`'s production source.
#[derive(Debug, Clone)]
struct ExtractedCall {
    /// The enclosing `pub async fn`/`fn`'s name, for failure messages.
    fn_name: String,
    /// 1-based source line the call starts on.
    line: usize,
    method: HttpMethod,
    /// `None` for a `root_get` call (no owning component on the wire) and
    /// for a totality failure (a verb call with no `Component::` first
    /// argument) — [`ExtractedCall::is_root`] and
    /// [`ExtractedCall::has_component`] disambiguate the two.
    component: Option<String>,
    is_root: bool,
    /// The resolved relative-path template, when resolvable at all (always
    /// resolvable for every real site in this crate; `None` only for a
    /// synthetic probe designed to exercise the unresolvable case).
    relative: Option<String>,
}

impl ExtractedCall {
    fn has_component(&self) -> bool {
        self.is_root || self.component.is_some()
    }
}

/// (start byte offset of the `fn` keyword's line, function name), sorted by
/// offset ascending — the enumeration `enclosing_fn_name` and
/// `function_body` walk.
fn function_boundaries(source: &str) -> Vec<(usize, String)> {
    let fn_re = Regex::new(r"(?m)^\s*(?:pub(?:\(crate\))? )?(?:async )?fn (\w+)").unwrap();
    fn_re
        .captures_iter(source)
        .map(|caps| {
            let whole = caps.get(0).unwrap();
            (whole.start(), caps[1].to_string())
        })
        .collect()
}

fn enclosing_fn_name(boundaries: &[(usize, String)], offset: usize) -> String {
    boundaries
        .iter()
        .rev()
        .find(|(start, _)| *start <= offset)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| "<module scope>".to_string())
}

/// The text from `name`'s own `fn` declaration to the next function
/// boundary (or end of `source`) — a bounded window for local `let`
/// resolution and (for a helper like `build_search_path`) its own
/// hardcoded literal.
fn function_body<'a>(
    source: &'a str,
    boundaries: &[(usize, String)],
    name: &str,
) -> Option<&'a str> {
    let index = boundaries.iter().position(|(_, n)| n == name)?;
    let start = boundaries.get(index)?.0;
    let end = boundaries
        .get(index + 1)
        .map_or(source.len(), |(next, _)| *next);
    Some(&source[start..end])
}

/// A safe, in-bounds-or-sentinel byte read, so the depth-tracking scanners
/// below never index a slice directly (`clippy::indexing_slicing`, denied
/// workspace-wide). `0` never collides with any byte this module matches on
/// (`(`, `)`, `{`, `}`, `[`, `]`, `,`, `;`, `"`, `\`).
fn byte_at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// Advances `i` past a `"…"` string literal's contents (honouring `\`
/// escapes), leaving `i` on the closing quote (or at `bytes.len()` if the
/// literal is unterminated).
fn skip_string_contents(bytes: &[u8], i: &mut usize) {
    *i += 1;
    while *i < bytes.len() && byte_at(bytes, *i) != b'"' {
        if byte_at(bytes, *i) == b'\\' {
            *i += 1;
        }
        *i += 1;
    }
}

/// Finds the byte offset of `text`'s matching close paren for the open
/// paren at `open`, skipping over `(`/`)` inside string literals.
fn match_paren(text: &str, open: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match byte_at(bytes, i) {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            b'"' => skip_string_contents(bytes, &mut i),
            _ => {}
        }
        i += 1;
    }
    panic!("unbalanced parens starting at byte {open} in:\n{text}");
}

/// Splits `text` at its first comma sitting at bracket depth 0 and outside
/// any string literal — the boundary between a verb call's `Component::X`
/// argument and its relative-path argument.
fn split_top_level_comma(text: &str) -> Option<(String, String)> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match byte_at(bytes, i) {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b',' if depth == 0 => {
                return Some((text[..i].to_string(), text[i + 1..].to_string()));
            }
            b'"' => skip_string_contents(bytes, &mut i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The content of the first string literal in `text` whose content starts
/// with `/` — the shared shape of a relative-path template, whether it sits
/// directly in a verb call's argument, in a `let` binding's right-hand
/// side, or inside a path-building helper's own body. Skips literals that
/// don't start with `/` (a query-fragment literal such as `"cursor={c}"`
/// inside a helper's body never wins over its own path-shaped literal).
fn first_path_literal(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if byte_at(bytes, i) == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && byte_at(bytes, j) != b'"' {
                if byte_at(bytes, j) == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let literal = &text[start..j.min(text.len())];
            if literal.starts_with('/') {
                return Some(literal.to_string());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    None
}

/// Finds `let (mut )?{ident} = <rhs>;` in `scope` and returns `<rhs>`
/// (bracket/brace/paren depth 0 is what ends the statement, so a `match { .. }`
/// or multi-line `format!(..)` right-hand side is captured whole).
fn find_let_binding(scope: &str, ident: &str) -> Option<String> {
    let pattern = format!(r"let\s+(?:mut\s+)?{}\s*=\s*", regex::escape(ident));
    let re = Regex::new(&pattern).unwrap();
    let m = re.find(scope)?;
    let rhs_start = m.end();
    let bytes = scope.as_bytes();
    let mut depth = 0i32;
    let mut i = rhs_start;
    while i < bytes.len() {
        match byte_at(bytes, i) {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b';' if depth == 0 => {
                return Some(scope[rhs_start..i].to_string());
            }
            b'"' => skip_string_contents(bytes, &mut i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Resolves `expr` (a verb call's relative-path argument, already stripped
/// of its leading `&`) to its relative-path template:
///
/// 1. A literal or `format!("...")` directly in `expr` — used as-is.
/// 2. A bare identifier bound by a local `let` in `fn_body` — recurse into
///    its right-hand side (covers both a direct literal/`format!` binding
///    and a passthrough helper call like `build_paginated_path(&format!(
///    "/api-keys"), ..)`, whose literal argument is textually present in
///    the right-hand side regardless of nesting).
/// 3. A call to a helper with no literal in its own call arguments (an
///    "owning" helper like `build_search_path(ws, q, ..)`, which hardcodes
///    its template internally) — resolved from the callee's own body,
///    looked up by name in `source`.
///
/// A trailing `?query=...` is stripped: registry templates never carry one.
fn resolve_relative(
    expr: &str,
    fn_body: &str,
    boundaries: &[(usize, String)],
    source: &str,
) -> Option<String> {
    let expr = expr.trim().trim_start_matches('&').trim();

    let resolved = if let Some(literal) = first_path_literal(expr) {
        Some(literal)
    } else if is_identifier(expr) {
        let rhs = find_let_binding(fn_body, expr)?;
        if let Some(literal) = first_path_literal(&rhs) {
            Some(literal)
        } else {
            let callee = call_target_name(&rhs)?;
            let callee_body = function_body(source, boundaries, &callee)?;
            first_path_literal(callee_body)
        }
    } else {
        None
    };

    resolved.map(|template| {
        template
            .split('?')
            .next()
            .expect("split always yields at least one element")
            .to_string()
    })
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && text.chars().next().is_some_and(|c| !c.is_ascii_digit())
}

/// If `text` (trimmed) is a plain call expression `name(..)`, returns
/// `name`.
fn call_target_name(text: &str) -> Option<String> {
    let text = text.trim();
    let paren = text.find('(')?;
    let name = &text[..paren];
    is_identifier(name).then(|| name.to_string())
}

/// Extracts every `self.<verb>(..)` and `self.root_get(..)` call in
/// `source`, using `boundaries` to name each call's enclosing function.
/// `self.http.get(..)` (the seam's own implementation) never matches: the
/// token immediately after `self.` there is `http`, not a verb name.
fn extract_calls(source: &str, boundaries: &[(usize, String)]) -> Vec<ExtractedCall> {
    let mut calls = Vec::new();

    for verb in ["get", "post", "patch", "put", "delete"] {
        let re = Regex::new(&format!(r"self\s*\.\s*{verb}\s*\(")).unwrap();
        for m in re.find_iter(source) {
            let open = m.end() - 1;
            let close = match_paren(source, open);
            let arg_text = &source[open + 1..close];
            let line = 1 + source[..m.start()].matches('\n').count();
            let fn_name = enclosing_fn_name(boundaries, m.start());
            let fn_body =
                function_body(source, boundaries, &fn_name).unwrap_or(&source[m.start()..close]);

            let (component, relative) = match split_top_level_comma(arg_text) {
                Some((component_expr, relative_expr)) => {
                    let component_expr = component_expr.trim();
                    let component = COMPONENTS
                        .iter()
                        .find(|name| component_expr == format!("Component::{}", capitalize(name)))
                        .map(|name| name.to_string());
                    let relative = resolve_relative(&relative_expr, fn_body, boundaries, source);
                    (component, relative)
                }
                None => (
                    None,
                    resolve_relative(arg_text, fn_body, boundaries, source),
                ),
            };

            let method = match verb {
                "get" => HttpMethod::Get,
                "post" => HttpMethod::Post,
                "patch" => HttpMethod::Patch,
                "put" => HttpMethod::Put,
                "delete" => HttpMethod::Delete,
                _ => unreachable!(),
            };

            calls.push(ExtractedCall {
                fn_name,
                line,
                method,
                component,
                is_root: false,
                relative,
            });
        }
    }

    let root_re = Regex::new(r"self\s*\.\s*root_get\s*\(").unwrap();
    for m in root_re.find_iter(source) {
        let open = m.end() - 1;
        let close = match_paren(source, open);
        let arg_text = &source[open + 1..close];
        let line = 1 + source[..m.start()].matches('\n').count();
        let fn_name = enclosing_fn_name(boundaries, m.start());
        let relative = first_path_literal(arg_text);

        calls.push(ExtractedCall {
            fn_name,
            line,
            method: HttpMethod::Get,
            component: None,
            is_root: true,
            relative,
        });
    }

    calls.sort_by_key(|c| c.line);
    calls
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `{...}` -> `{}` on every path segment, so `{ws}` and `{project_slug}`
/// compare equal (D2.3/R6): the client and the registry name the same
/// positional placeholder differently at 14 custos sites.
fn normalize_template(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') && segment.len() >= 2 {
                "{}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Every registry route with no client call site — reason included, checked
/// bidirectionally by [`uncovered_routes_table_matches_the_live_gap`]: an
/// unlisted uncovered route fails, and a route the client now covers is a
/// stale entry.
const UNCOVERED_ROUTES: &[(HttpMethod, &str, &str)] = &[
    (
        HttpMethod::Get,
        "/ready",
        "readiness probe; no AtlasClient consumer needs it",
    ),
    (
        HttpMethod::Get,
        "/version",
        "build-version probe; no AtlasClient consumer needs it",
    ),
    (
        HttpMethod::Get,
        "/openapi.json",
        "document introspection; no AtlasClient consumer needs it",
    ),
    (
        HttpMethod::Get,
        "/scalar",
        "the HTML API-doc viewer; no AtlasClient consumer needs it",
    ),
    (
        HttpMethod::Post,
        "/users/{user_id}/system-admin",
        "root-only system-admin toggle; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Get,
        "/activate/{token}",
        "the self-service activation page; no AtlasClient consumer needs it",
    ),
    (
        HttpMethod::Post,
        "/activate/{token}",
        "self-service account activation; no AtlasClient consumer needs it",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/presence",
        "board presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}/presence",
        "board presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/presence",
        "document presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/presence",
        "document presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/integration-configs",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs/{config_id}",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/integration-configs/{config_id}",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/integration-configs/{config_id}",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/automation-rules",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Patch,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Delete,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/events",
        "the SSE event stream; a raw connection, not a request/response verb call",
    ),
    (
        HttpMethod::Get,
        "/workspaces/{ws}/semantic-search/reindex",
        "reindex status polling; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/semantic-search/reindex",
        "triggering a reindex; not yet exposed on AtlasClient",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/integrations/{integration}/events",
        "the incoming third-party webhook receiver; called by the integration, not by AtlasClient",
    ),
];

/// The registry's declared `(method, relative)` surface, deduplicated
/// across the three copies [`crate::router_audit::ROOT_LEVEL_PATHS`]
/// members carry (one per component, per `route_matrix()`'s own contract:
/// a root-level path is reachable through any component's mount).
fn registry_route_surface() -> Vec<RouteMatrixEntry> {
    route_matrix()
}

fn forward_check(calls: &[ExtractedCall], entries: &[RouteMatrixEntry]) -> Vec<String> {
    let mut failures = Vec::new();

    for call in calls {
        if call.is_root {
            let relative = call.relative.as_deref().unwrap_or("<unresolved>");
            if !atlas_server::router_audit::ROOT_LEVEL_PATHS.contains(&relative) {
                failures.push(format!(
                    "{} (line {}): root_get(\"{relative}\") is not a ROOT_LEVEL_PATHS member",
                    call.fn_name, call.line
                ));
            }
            continue;
        }

        let Some(component) = &call.component else {
            failures.push(format!(
                "{} (line {}): {} call has no Component:: first argument",
                call.fn_name, call.line, call.method
            ));
            continue;
        };

        let Some(relative) = &call.relative else {
            failures.push(format!(
                "{} (line {}): could not resolve a relative-path template for this call",
                call.fn_name, call.line
            ));
            continue;
        };

        let normalized_relative = normalize_template(relative);
        let matches = entries.iter().any(|entry| {
            entry.method == call.method
                && &entry.component == component
                && normalize_template(&entry.path_template) == normalized_relative
        });

        if !matches {
            failures.push(format!(
                "{} (line {}): {} Component::{}, \"{relative}\" matches no declared V2 route",
                call.fn_name,
                call.line,
                call.method,
                capitalize(component)
            ));
        }
    }

    failures
}

fn totality_check(calls: &[ExtractedCall]) -> Vec<String> {
    calls
        .iter()
        .filter(|call| !call.has_component())
        .map(|call| {
            format!(
                "{} (line {}): {} call has no Component:: first argument",
                call.fn_name, call.line, call.method
            )
        })
        .collect()
}

/// Bidirectional reverse check: an unlisted uncovered route fails, and a
/// listed route the client now covers is a stale entry.
fn reverse_check(calls: &[ExtractedCall], entries: &[RouteMatrixEntry]) -> Vec<String> {
    let mut failures = Vec::new();

    let is_covered = |method: HttpMethod, relative: &str| {
        let normalized = normalize_template(relative);
        calls.iter().any(|call| {
            call.method == method
                && call
                    .relative
                    .as_deref()
                    .is_some_and(|r| normalize_template(r) == normalized)
        })
    };

    let mut seen: Vec<(HttpMethod, String)> = Vec::new();
    for entry in entries {
        let key = (entry.method, normalize_template(&entry.path_template));
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());

        if is_covered(entry.method, &entry.path_template) {
            continue;
        }

        let listed = UNCOVERED_ROUTES
            .iter()
            .any(|(method, path, _)| *method == entry.method && normalize_template(path) == key.1);

        if !listed {
            failures.push(format!(
                "{} {} has no AtlasClient call site and is not listed in UNCOVERED_ROUTES",
                entry.method, entry.path_template
            ));
        }
    }

    for (method, path, reason) in UNCOVERED_ROUTES {
        if is_covered(*method, path) {
            failures.push(format!(
                "stale UNCOVERED_ROUTES entry ({method} {path}, \"{reason}\"): an AtlasClient call site now covers it"
            ));
        }
    }

    failures
}

#[test]
fn every_migrated_call_matches_a_declared_v2_route_both_directions() {
    let source = production_source();
    let boundaries = function_boundaries(&source);
    let calls = extract_calls(&source, &boundaries);
    let entries = registry_route_surface();

    let mut failures = totality_check(&calls);
    failures.extend(forward_check(&calls, &entries));
    failures.extend(reverse_check(&calls, &entries));

    assert!(
        failures.is_empty(),
        "atlas_client route contract violations:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_extracted_call_has_a_resolved_relative_template() {
    let source = production_source();
    let boundaries = function_boundaries(&source);
    let calls = extract_calls(&source, &boundaries);

    assert!(!calls.is_empty(), "expected to extract at least one call");

    let unresolved: Vec<String> = calls
        .iter()
        .filter(|call| call.relative.is_none())
        .map(|call| format!("{} (line {})", call.fn_name, call.line))
        .collect();

    assert!(
        unresolved.is_empty(),
        "calls with no resolvable relative-path template:\n{}",
        unresolved.join("\n")
    );
}

/// D2.20 — the probe self-test: a scratch source string containing (a) a
/// verb call with the wrong `Component::`, (b) a verb call with no
/// `Component::` argument, is flagged by name. Built via `format!` so this
/// test's own source carries no forbidden shape for the sibling grep gate.
#[test]
fn the_walker_flags_a_wrong_component_by_name() {
    let source = format!(
        "impl AtlasClient {{\n    pub async fn probe_wrong_component(&self) {{\n        let response = self.get(Component::Custos, {}).send().await?;\n    }}\n}}\n",
        format_args!("\"{}\"", "/workspaces/{ws}/tasks")
    );
    let boundaries = function_boundaries(&source);
    let calls = extract_calls(&source, &boundaries);
    let entries = registry_route_surface();

    let failures = forward_check(&calls, &entries);

    assert!(
        failures.iter().any(|f| f.contains("probe_wrong_component")),
        "expected the wrong-component call to be flagged by name, got:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_walker_flags_a_missing_component_by_name() {
    let source = format!(
        "impl AtlasClient {{\n    pub async fn probe_missing_component(&self) {{\n        let response = self.get({}).send().await?;\n    }}\n}}\n",
        format_args!("\"{}\"", "/workspaces/{ws}/tasks")
    );
    let boundaries = function_boundaries(&source);
    let calls = extract_calls(&source, &boundaries);

    let failures = totality_check(&calls);

    assert!(
        failures
            .iter()
            .any(|f| f.contains("probe_missing_component")),
        "expected the componentless call to be flagged by name, got:\n{}",
        failures.join("\n")
    );
}

/// (c) a stale `UNCOVERED_ROUTES` entry is flagged, mirroring the grep
/// gate's stale-allowlist discipline.
#[test]
fn a_stale_uncovered_routes_entry_is_flagged() {
    let call = ExtractedCall {
        fn_name: "probe".to_string(),
        line: 1,
        method: HttpMethod::Get,
        component: Some("acta".to_string()),
        is_root: false,
        relative: Some("/workspaces/{ws}/tasks".to_string()),
    };
    let entries = vec![RouteMatrixEntry {
        method: HttpMethod::Get,
        path_template: "/workspaces/{ws}/tasks".to_string(),
        is_public: false,
        component: "acta".to_string(),
    }];

    // A stale table naming an entry the probe call above now covers.
    let stale_table: &[(HttpMethod, &str, &str)] =
        &[(HttpMethod::Get, "/workspaces/{ws}/tasks", "stale probe")];

    let failures = reverse_check_with_table(&[call], &entries, stale_table);

    assert!(
        failures.iter().any(|f| f.contains("stale")),
        "expected the stale UNCOVERED_ROUTES entry to be flagged, got:\n{}",
        failures.join("\n")
    );
}

/// Test-only indirection so [`a_stale_uncovered_routes_entry_is_flagged`]
/// can exercise the stale-entry direction against a synthetic table,
/// without touching the real [`UNCOVERED_ROUTES`] const.
fn reverse_check_with_table(
    calls: &[ExtractedCall],
    entries: &[RouteMatrixEntry],
    table: &[(HttpMethod, &str, &str)],
) -> Vec<String> {
    let mut failures = Vec::new();
    let is_covered = |method: HttpMethod, relative: &str| {
        let normalized = normalize_template(relative);
        calls.iter().any(|call| {
            call.method == method
                && call
                    .relative
                    .as_deref()
                    .is_some_and(|r| normalize_template(r) == normalized)
        })
    };

    for (method, path, reason) in table {
        if is_covered(*method, path) {
            failures.push(format!(
                "stale UNCOVERED_ROUTES entry ({method} {path}, \"{reason}\"): an AtlasClient call site now covers it"
            ));
        }
    }

    let _ = entries;
    failures
}

#[test]
fn normalize_template_collapses_any_placeholder_name() {
    assert_eq!(
        normalize_template("/workspaces/{ws}/projects/{slug}/grants"),
        normalize_template("/workspaces/{ws}/projects/{project_slug}/grants"),
    );
    assert_ne!(
        normalize_template("/workspaces/{ws}/grants"),
        normalize_template("/workspaces/{ws}/groups"),
    );
}

#[test]
fn first_path_literal_skips_query_fragment_literals_and_finds_the_path() {
    let text = r#"params.push(format!("cursor={c}")); base.to_string(); "/admin/trash""#;
    assert_eq!(first_path_literal(text), Some("/admin/trash".to_string()));
}

#[test]
fn resolve_relative_follows_a_passthrough_helper_argument() {
    let source = "fn probe() {\n    let path = build_paginated_path(\"/api-keys\", cursor, limit);\n    self.get(Component::Custos, &path);\n}\n";
    let boundaries = function_boundaries(source);
    let body = function_body(source, &boundaries, "probe").unwrap();

    assert_eq!(
        resolve_relative("&path", body, &boundaries, source),
        Some("/api-keys".to_string())
    );
}

#[test]
fn resolve_relative_follows_an_owning_helper_to_its_own_body() {
    let source = "fn probe(ws: &str) {\n    let path = build_search_path(ws, q, None, None, None, None, None);\n    self.get(Component::Acta, &path);\n}\n\nfn build_search_path(ws: &str) -> String {\n    format!(\"/workspaces/{ws}/search?{}\", 1)\n}\n";
    let boundaries = function_boundaries(source);
    let body = function_body(source, &boundaries, "probe").unwrap();

    assert_eq!(
        resolve_relative("&path", body, &boundaries, source),
        Some("/workspaces/{ws}/search".to_string())
    );
}
