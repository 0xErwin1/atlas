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

fn client_source_path(file_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../atlas_client/src")
        .join(file_name)
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

/// Reads and truncates `crates/atlas_client/src/<file_name>`'s production
/// source the same way [`production_source`] reads `lib.rs`. Introduced in
/// PR3 (D2.2) once the client's methods start splitting across
/// per-component files; every file [`PRODUCTION_SOURCE_MAPPINGS`] names
/// (other than `lib.rs` itself) is read through this function.
fn source_for(file_name: &str) -> String {
    let content = fs::read_to_string(client_source_path(file_name))
        .unwrap_or_else(|_| panic!("read atlas_client/src/{file_name}"));
    truncate_at_test_module(&content)
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
            // The owning helper's own body is looked up first in `source`
            // (the calling method's own file), then in `lib.rs` — these
            // private path-builder free functions have not moved with the
            // per-component split (D2.2), so a call from `acta.rs`/
            // `custos.rs` resolves its callee's body in the root file.
            if let Some(callee_body) = function_body(source, boundaries, &callee) {
                first_path_literal(callee_body)
            } else {
                let root_source = production_source();
                let root_boundaries = function_boundaries(&root_source);
                let callee_body = function_body(&root_source, &root_boundaries, &callee)?;
                first_path_literal(callee_body)
            }
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

/// A `crates/atlas_client/src/*.rs` file's declared owning component for the
/// namespace-attribution check (D4.3). `Mixed` is the only value the check
/// skips; `lib.rs` is the sole file allowed to carry it at any point (D4.4's
/// totality assertion: "no second `Mixed` file").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileComponent {
    Acta,
    Custos,
    Platform,
    Mixed,
}

impl FileComponent {
    fn as_component_str(self) -> Option<&'static str> {
        match self {
            FileComponent::Acta => Some("acta"),
            FileComponent::Custos => Some("custos"),
            FileComponent::Platform => Some("platform"),
            FileComponent::Mixed => None,
        }
    }
}

/// One `crates/atlas_client/src/*.rs` file mapped for the namespace-
/// attribution and file-set-totality checks (D4.3, D4.4).
struct SourceMapping {
    file_name: &'static str,
    component: FileComponent,
    reason: &'static str,
}

/// Every production file in `crates/atlas_client/src/` at this PR's ground
/// truth. `lib.rs` stays `Mixed` permanently (design R10, D4.4): it holds
/// `login` (a custos wire call that mutates `self.token`, an effect a
/// `&self` sub-client cannot perform) and `health`/`root_get` (no owning
/// component on the wire) — the two named exceptions that never move to a
/// sub-client. `custos.rs` (PR3, D2.2), `acta.rs` (PR4, D2.2) and
/// `platform.rs` (PR5, D2.2) carry only their own component's methods and
/// are each mapped to their own component. `lib.rs` is the only file the
/// totality check ([`walked_source_file_set_is_total`]) permits to carry
/// `Mixed` at any point.
const PRODUCTION_SOURCE_MAPPINGS: &[SourceMapping] = &[
    SourceMapping {
        file_name: "lib.rs",
        component: FileComponent::Mixed,
        reason: "root client — holds login (a custos wire call whose effect is self.token = …, \
                 incompatible with a &self sub-client) and health/root_get (no owning component)",
    },
    SourceMapping {
        file_name: "custos.rs",
        component: FileComponent::Custos,
        reason: "custos-owned client methods, split out of lib.rs in PR3",
    },
    SourceMapping {
        file_name: "acta.rs",
        component: FileComponent::Acta,
        reason: "acta-owned client methods, split out of lib.rs in PR4",
    },
    SourceMapping {
        file_name: "platform.rs",
        component: FileComponent::Platform,
        reason: "platform-owned client methods, split out of lib.rs in PR5",
    },
];

/// Files in `crates/atlas_client/src/` that carry no `self.<verb>(..)` calls
/// of their own — consumers of `AtlasClient`, not contributors to this
/// walker's subject. `helpers.rs` calls the flat methods, never
/// `self.<verb>(...)` directly, so it needs no component mapping.
const NON_PRODUCTION_ALLOWLIST: &[&str] = &["helpers.rs"];

/// D4.4's file-set totality: every `.rs` file name in `file_names` MUST be
/// either mapped in [`PRODUCTION_SOURCE_MAPPINGS`] or named in
/// [`NON_PRODUCTION_ALLOWLIST`]. Returns the unmapped, non-allowlisted names
/// — a new client module carrying verb calls that nobody mapped fails this
/// gate instead of silently escaping coverage.
fn unmapped_source_files(file_names: &[String]) -> Vec<String> {
    file_names
        .iter()
        .filter(|name| name.ends_with(".rs"))
        .filter(|name| {
            let mapped = PRODUCTION_SOURCE_MAPPINGS
                .iter()
                .any(|mapping| &mapping.file_name == name);
            let allowlisted = NON_PRODUCTION_ALLOWLIST.contains(&name.as_str());
            !mapped && !allowlisted
        })
        .cloned()
        .collect()
}

fn client_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../atlas_client/src")
}

fn real_client_source_file_names() -> Vec<String> {
    fs::read_dir(client_src_dir())
        .expect("read atlas_client/src")
        .map(|entry| entry.expect("dir entry").file_name())
        .filter_map(|name| name.into_string().ok())
        .collect()
}

/// D4.3 — for every call extracted from a file mapped to a *specific*
/// component (not `Mixed`), its literal `Component::X` MUST equal that
/// file's declared component. A root-level `root_get` call carries no
/// `Component::` literal and is exempt (it targets no owning namespace on
/// the wire). `Mixed`-mapped files are skipped entirely.
fn namespace_attribution_check(
    calls: &[ExtractedCall],
    file_component: FileComponent,
) -> Vec<String> {
    let Some(expected) = file_component.as_component_str() else {
        return Vec::new();
    };

    calls
        .iter()
        .filter(|call| !call.is_root)
        .filter_map(|call| match &call.component {
            Some(actual) if actual != expected => Some(format!(
                "{} (line {}): call carries Component::{} but its file is mapped to {}",
                call.fn_name,
                call.line,
                capitalize(actual),
                capitalize(expected)
            )),
            _ => None,
        })
        .collect()
}

/// Every registry route with no client call site — owning component and
/// reason included, checked bidirectionally by [`reverse_check`]: an
/// unlisted uncovered route fails, and a route the client now covers is a
/// stale entry.
///
/// Re-keyed component-aware in E11-S4 PR1 (D4.5): every entry now names the
/// component it belongs to, resolved against the namespaced end state (the
/// destination each route's owning method will live under after PR3–PR5),
/// not the unmigrated client's current `Mixed` file. Four entries were added
/// beyond the original 25 — `custos`'s and `acta`'s own namespaced
/// `/health`/`/ready` probes (E11-S3a) — which the pre-PR1 `(method, path)`
/// dedup silently skipped: each collided with the platform root's own
/// `/health`/`/ready` dedup key and was never independently checked (spec
/// Purpose, design §0.3). The component-aware re-key surfaces them for the
/// first time; they are genuinely uncovered (`AtlasClient` has no
/// per-component health/ready method), so they are listed here rather than
/// covered by a new method.
const UNCOVERED_ROUTES: &[(&str, HttpMethod, &str, &str)] = &[
    (
        "platform",
        HttpMethod::Get,
        "/ready",
        "readiness probe; no AtlasClient consumer needs it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/version",
        "build-version probe; no AtlasClient consumer needs it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/openapi.json",
        "document introspection; no AtlasClient consumer needs it",
    ),
    (
        "platform",
        HttpMethod::Get,
        "/scalar",
        "the HTML API-doc viewer; no AtlasClient consumer needs it",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/users/{user_id}/system-admin",
        "root-only system-admin toggle; not yet exposed on AtlasClient",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/activate/{token}",
        "the self-service activation page; no AtlasClient consumer needs it",
    ),
    (
        "custos",
        HttpMethod::Post,
        "/activate/{token}",
        "self-service account activation; no AtlasClient consumer needs it",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/health",
        "custos's own namespaced health probe (E11-S3a); AtlasClient's health() targets only the root-level probe, not this component-scoped copy",
    ),
    (
        "custos",
        HttpMethod::Get,
        "/ready",
        "custos's own namespaced readiness probe (E11-S3a); no AtlasClient consumer needs the per-component probe, only the aggregated root /ready",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/presence",
        "board presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/boards/{board_id}/presence",
        "board presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/presence",
        "document presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/documents/{slug}/presence",
        "document presence heartbeat; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/integration-configs",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/integration-configs/{config_id}",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/integration-configs/{config_id}",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/integration-configs/{config_id}",
        "integration-config management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/automation-rules",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Patch,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Delete,
        "/workspaces/{ws}/automation-rules/{rule_id}",
        "automation-rule management; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/events",
        "the SSE event stream; a raw connection, not a request/response verb call",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/semantic-search/reindex",
        "reindex status polling; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Post,
        "/workspaces/{ws}/semantic-search/reindex",
        "triggering a reindex; not yet exposed on AtlasClient",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/health",
        "acta's own namespaced health probe (E11-S3a); AtlasClient's health() targets only the root-level probe, not this component-scoped copy",
    ),
    (
        "acta",
        HttpMethod::Get,
        "/ready",
        "acta's own namespaced readiness probe (E11-S3a); no AtlasClient consumer needs the per-component probe, only the aggregated root /ready",
    ),
    (
        "acta",
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

/// Whether some call in `calls` covers `component`'s `(method, relative)`
/// route (D4.2). A root-level probe call (`self.root_get(..)`, `component:
/// None`, `is_root: true`) covers only `platform`'s copy of a
/// [`atlas_server::router_audit::ROOT_LEVEL_PATHS`] member — the one route
/// actually reachable unprefixed on the wire — never `custos`'s or `acta`'s
/// own namespaced copy of the same path string (D3, D4.2's "a call site
/// resolving to the wrong component is no longer accepted as coverage").
fn is_covered(
    calls: &[ExtractedCall],
    component: &str,
    method: HttpMethod,
    relative: &str,
) -> bool {
    let normalized = normalize_template(relative);
    calls.iter().any(|call| {
        call.method == method
            && call
                .relative
                .as_deref()
                .is_some_and(|r| normalize_template(r) == normalized)
            && if call.is_root {
                component == "platform"
            } else {
                call.component.as_deref() == Some(component)
            }
    })
}

/// The pre-PR1 dedup key and coverage predicate: `(method, path)` only, with
/// no owning component (spec Purpose, design §0.3). Retained solely so
/// [`two_components_sharing_a_path_are_checked_independently`] and
/// [`a_call_on_the_wrong_component_does_not_count_as_coverage`] can
/// demonstrate the component-aware re-key changes real behavior rather than
/// only its types — never used by the production checks below.
fn is_covered_component_blind(calls: &[ExtractedCall], method: HttpMethod, relative: &str) -> bool {
    let normalized = normalize_template(relative);
    calls.iter().any(|call| {
        call.method == method
            && call
                .relative
                .as_deref()
                .is_some_and(|r| normalize_template(r) == normalized)
    })
}

/// The pre-PR1 reverse check, built on [`is_covered_component_blind`] and a
/// `(HttpMethod, path)`-only dedup key — see
/// [`is_covered_component_blind`]'s doc comment for why this exists.
fn reverse_check_component_blind(
    calls: &[ExtractedCall],
    entries: &[RouteMatrixEntry],
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut seen: Vec<(HttpMethod, String)> = Vec::new();

    for entry in entries {
        let key = (entry.method, normalize_template(&entry.path_template));
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());

        if is_covered_component_blind(calls, entry.method, &entry.path_template) {
            continue;
        }

        failures.push(format!(
            "{} {} ({}) has no AtlasClient call site",
            entry.method, entry.path_template, entry.component
        ));
    }

    failures
}

/// Bidirectional reverse check: an unlisted uncovered route fails, and a
/// listed route the client now covers is a stale entry. Both the dedup key
/// and [`is_covered`] are component-aware (D4.1, D4.2) — S3a's four
/// `health`/`ready` probe routes shared across `platform`, `custos`, and
/// `acta` are four independent entries here, never collapsed into fewer.
fn reverse_check(calls: &[ExtractedCall], entries: &[RouteMatrixEntry]) -> Vec<String> {
    let mut failures = Vec::new();

    let mut seen: Vec<(String, HttpMethod, String)> = Vec::new();
    for entry in entries {
        let key = (
            entry.component.clone(),
            entry.method,
            normalize_template(&entry.path_template),
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());

        if is_covered(calls, &entry.component, entry.method, &entry.path_template) {
            continue;
        }

        let listed = UNCOVERED_ROUTES.iter().any(|(component, method, path, _)| {
            *component == entry.component
                && *method == entry.method
                && normalize_template(path) == key.2
        });

        if !listed {
            failures.push(format!(
                "{} {} ({}) has no AtlasClient call site and is not listed in UNCOVERED_ROUTES",
                entry.method, entry.path_template, entry.component
            ));
        }
    }

    for (component, method, path, reason) in UNCOVERED_ROUTES {
        if is_covered(calls, component, *method, path) {
            failures.push(format!(
                "stale UNCOVERED_ROUTES entry ({component} {method} {path}, \"{reason}\"): an AtlasClient call site now covers it"
            ));
        }
    }

    failures
}

/// Extracts calls from every file [`PRODUCTION_SOURCE_MAPPINGS`] names,
/// paired with that file's declared component — the combined population
/// [`totality_check`], [`forward_check`], and [`reverse_check`] run over
/// (D4.3/D4.4). A pure move between mapped files changes no call's resolved
/// route, so the combined set stays a stable proxy for "every route this
/// crate's production code can reach," regardless of which file a method
/// currently lives in.
fn all_mapped_calls() -> Vec<(FileComponent, Vec<ExtractedCall>)> {
    PRODUCTION_SOURCE_MAPPINGS
        .iter()
        .map(|mapping| {
            let source = if mapping.file_name == "lib.rs" {
                production_source()
            } else {
                source_for(mapping.file_name)
            };
            let boundaries = function_boundaries(&source);
            (mapping.component, extract_calls(&source, &boundaries))
        })
        .collect()
}

#[test]
fn every_migrated_call_matches_a_declared_v2_route_both_directions() {
    let per_file = all_mapped_calls();
    let calls: Vec<ExtractedCall> = per_file
        .iter()
        .flat_map(|(_, calls)| calls.iter().cloned())
        .collect();
    let entries = registry_route_surface();

    let mut failures = totality_check(&calls);
    failures.extend(forward_check(&calls, &entries));
    failures.extend(reverse_check(&calls, &entries));
    for (component, file_calls) in &per_file {
        failures.extend(namespace_attribution_check(file_calls, *component));
    }

    assert!(
        failures.is_empty(),
        "atlas_client route contract violations:\n{}",
        failures.join("\n")
    );
}

/// D4.4 — every real `crates/atlas_client/src/*.rs` file is mapped or
/// allowlisted; there is exactly one `Mixed`-mapped file (`lib.rs`,
/// permanently, per R10 — it holds `login` and `health`).
#[test]
fn every_client_source_file_is_mapped_or_allowlisted() {
    let unmapped = unmapped_source_files(&real_client_source_file_names());
    assert!(
        unmapped.is_empty(),
        "unmapped atlas_client/src files, neither mapped nor allowlisted: {unmapped:?}"
    );

    let mixed_count = PRODUCTION_SOURCE_MAPPINGS
        .iter()
        .filter(|mapping| mapping.component == FileComponent::Mixed)
        .count();
    assert_eq!(
        mixed_count, 1,
        "exactly one atlas_client/src file may be mapped Mixed at any point (D4.4)"
    );

    for mapping in PRODUCTION_SOURCE_MAPPINGS {
        assert!(
            !mapping.reason.is_empty(),
            "{} carries no written mapping reason (D4.4)",
            mapping.file_name
        );
    }
}

/// T1.6 — a synthetic unmapped file fails the file-set totality gate instead
/// of silently escaping coverage.
#[test]
fn an_unmapped_source_file_fails_the_totality_gate() {
    let mut file_names = real_client_source_file_names();
    file_names.push("mystery_module.rs".to_string());

    let unmapped = unmapped_source_files(&file_names);

    assert!(
        unmapped.iter().any(|name| name == "mystery_module.rs"),
        "expected the synthetic unmapped file to be flagged, got: {unmapped:?}"
    );
}

/// T1.8 — a call planted in a file mapped to a specific component, carrying
/// the *wrong* `Component::` literal, is flagged by name.
#[test]
fn the_namespace_attribution_check_flags_a_wrong_component_call_by_name() {
    let source = format!(
        "impl Acta<'_> {{\n    pub async fn probe_wrong_home(&self) {{\n        let response = self.get(Component::Custos, {}).send().await?;\n    }}\n}}\n",
        format_args!("\"{}\"", "/workspaces/{ws}/tasks")
    );
    let boundaries = function_boundaries(&source);
    let calls = extract_calls(&source, &boundaries);

    let failures = namespace_attribution_check(&calls, FileComponent::Acta);

    assert!(
        failures.iter().any(|f| f.contains("probe_wrong_home")),
        "expected the wrong-home call to be flagged by name, got:\n{}",
        failures.join("\n")
    );
}

/// T1.9 — a `Mixed`-mapped file is skipped by the namespace-attribution
/// check even when it carries calls to every component, confirmed against
/// real `lib.rs`.
#[test]
fn a_mixed_mapped_file_skips_the_namespace_attribution_check() {
    let source = production_source();
    let boundaries = function_boundaries(&source);
    let calls = extract_calls(&source, &boundaries);

    let failures = namespace_attribution_check(&calls, FileComponent::Mixed);

    assert!(
        failures.is_empty(),
        "expected a Mixed-mapped file to be a no-op, got:\n{}",
        failures.join("\n")
    );
}

/// D4.6 — count pins. `UNCOVERED_ROUTES.len()` and the extracted-call count
/// are pinned by exact equality, so a silently dropped or added entry fails
/// loudly. `UNCOVERED_ROUTES` grew from 25 to 29 in this PR: the four new
/// entries are `custos`'s and `acta`'s own namespaced `health`/`ready`
/// probes, newly visible under the component-aware re-key (see
/// [`UNCOVERED_ROUTES`]'s own doc comment).
#[test]
fn uncovered_routes_count_is_pinned() {
    assert_eq!(
        UNCOVERED_ROUTES.len(),
        29,
        "UNCOVERED_ROUTES grew or shrank without this pin moving"
    );
}

#[test]
fn extracted_call_count_is_pinned() {
    let per_file = all_mapped_calls();
    let total: usize = per_file.iter().map(|(_, calls)| calls.len()).sum();

    assert_eq!(
        total, 194,
        "the total number of self.<verb>(..)/self.root_get(..) call sites across every mapped \
         atlas_client/src file changed without this pin moving — a pure move between mapped \
         files must leave this total unchanged"
    );
}

/// D4.6, per-file — `custos.rs` split out exactly the 34 custos-owned call
/// sites in PR3, `acta.rs` split out exactly the 154 acta-owned call sites
/// in PR4, and `platform.rs` split out exactly the 4 platform-owned call
/// sites in PR5, leaving `lib.rs` with the rest (2: the one `login` custos
/// call and `health`'s `root_get`). Catches a call silently dropped or
/// duplicated during the move that the total-count pin above cannot
/// distinguish from a compensating change elsewhere.
#[test]
fn extracted_call_count_is_pinned_per_file() {
    let per_file = all_mapped_calls();
    let counts: std::collections::HashMap<&'static str, usize> = PRODUCTION_SOURCE_MAPPINGS
        .iter()
        .zip(per_file.iter())
        .map(|(mapping, (_, calls))| (mapping.file_name, calls.len()))
        .collect();

    assert_eq!(
        counts.get("lib.rs").copied(),
        Some(2),
        "lib.rs's own call count moved"
    );
    assert_eq!(
        counts.get("custos.rs").copied(),
        Some(34),
        "custos.rs's own call count moved"
    );
    assert_eq!(
        counts.get("acta.rs").copied(),
        Some(154),
        "acta.rs's own call count moved"
    );
    assert_eq!(
        counts.get("platform.rs").copied(),
        Some(4),
        "platform.rs's own call count moved"
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
/// gate's stale-allowlist discipline. Updated to the 4-tuple, component-aware
/// shape (D4.5, T1.5).
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
    let stale_table: &[(&str, HttpMethod, &str, &str)] = &[(
        "acta",
        HttpMethod::Get,
        "/workspaces/{ws}/tasks",
        "stale probe",
    )];

    let failures = reverse_check_with_table(&[call], &entries, stale_table);

    assert!(
        failures.iter().any(|f| f.contains("stale")),
        "expected the stale UNCOVERED_ROUTES entry to be flagged, got:\n{}",
        failures.join("\n")
    );
}

/// A stale entry naming the *wrong* component for a route the client now
/// covers must still be flagged stale — the component field does not give a
/// stale entry a new way to hide.
#[test]
fn a_stale_uncovered_routes_entry_is_flagged_regardless_of_which_component_it_names() {
    let call = ExtractedCall {
        fn_name: "probe".to_string(),
        line: 1,
        method: HttpMethod::Get,
        component: Some("custos".to_string()),
        is_root: false,
        relative: Some("/probe-shape".to_string()),
    };
    let entries = vec![RouteMatrixEntry {
        method: HttpMethod::Get,
        path_template: "/probe-shape".to_string(),
        is_public: false,
        component: "custos".to_string(),
    }];

    let stale_table: &[(&str, HttpMethod, &str, &str)] =
        &[("custos", HttpMethod::Get, "/probe-shape", "stale probe")];

    let failures = reverse_check_with_table(&[call], &entries, stale_table);

    assert!(
        failures.iter().any(|f| f.contains("stale")),
        "expected the stale entry to be flagged, got:\n{}",
        failures.join("\n")
    );
}

/// Test-only indirection so [`a_stale_uncovered_routes_entry_is_flagged`]
/// can exercise the stale-entry direction against a synthetic table,
/// without touching the real [`UNCOVERED_ROUTES`] const.
fn reverse_check_with_table(
    calls: &[ExtractedCall],
    entries: &[RouteMatrixEntry],
    table: &[(&str, HttpMethod, &str, &str)],
) -> Vec<String> {
    let mut failures = Vec::new();

    for (component, method, path, reason) in table {
        if is_covered(calls, component, *method, path) {
            failures.push(format!(
                "stale UNCOVERED_ROUTES entry ({component} {method} {path}, \"{reason}\"): an AtlasClient call site now covers it"
            ));
        }
    }

    let _ = entries;
    failures
}

/// T1.1(a) — two components declaring the same `(method, path)` shape are
/// checked independently under the component-aware re-key. Neither route is
/// covered or listed. The pre-PR1 component-blind dedup collapses them into
/// one check (masking one component's gap); the re-keyed check reports both.
#[test]
fn two_components_sharing_a_path_are_checked_independently() {
    let entries = vec![
        RouteMatrixEntry {
            method: HttpMethod::Get,
            path_template: "/probe-shared".to_string(),
            is_public: true,
            component: "custos".to_string(),
        },
        RouteMatrixEntry {
            method: HttpMethod::Get,
            path_template: "/probe-shared".to_string(),
            is_public: true,
            component: "acta".to_string(),
        },
    ];
    let calls: Vec<ExtractedCall> = Vec::new();

    let old_failures = reverse_check_component_blind(&calls, &entries);
    assert_eq!(
        old_failures.len(),
        1,
        "expected the component-blind dedup to collapse both routes into one check, got:\n{}",
        old_failures.join("\n")
    );

    let new_failures = reverse_check(&calls, &entries);
    assert_eq!(
        new_failures.len(),
        2,
        "expected the component-aware re-key to report both components' gaps independently, got:\n{}",
        new_failures.join("\n")
    );
}

/// T1.1(b) — a call site resolving to component `Y` that shares component
/// `X`'s method and normalized path must not count as coverage for `X`'s
/// route. The pre-PR1 component-blind `is_covered` wrongly accepts it; the
/// re-keyed check still reports `X`'s route uncovered.
#[test]
fn a_call_on_the_wrong_component_does_not_count_as_coverage() {
    let entries = vec![RouteMatrixEntry {
        method: HttpMethod::Get,
        path_template: "/probe-wrong-owner".to_string(),
        is_public: true,
        component: "custos".to_string(),
    }];
    let calls = vec![ExtractedCall {
        fn_name: "probe".to_string(),
        line: 1,
        method: HttpMethod::Get,
        component: Some("acta".to_string()),
        is_root: false,
        relative: Some("/probe-wrong-owner".to_string()),
    }];

    let old_failures = reverse_check_component_blind(&calls, &entries);
    assert!(
        old_failures.is_empty(),
        "expected the component-blind is_covered to wrongly accept the acta call as coverage, got:\n{}",
        old_failures.join("\n")
    );

    let new_failures = reverse_check(&calls, &entries);
    assert_eq!(
        new_failures.len(),
        1,
        "expected custos's route to still be reported uncovered, got:\n{}",
        new_failures.join("\n")
    );
    assert!(
        new_failures.first().is_some_and(|f| f.contains("custos")),
        "expected the failure to name custos, got: {new_failures:?}"
    );
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
