#![allow(dead_code, clippy::panic)]

//! Shared Rust-source walking helpers for the source-auditing tests:
//! production-scope file reads, a recursive `.rs` walk, depth-tracking
//! scanners that honour string literals, function-boundary slicing, the
//! client's relative-path resolution rule, and the MCP dispatcher's
//! method/arm parsing.
//!
//! Originally private to `atlas_client_route_contract.rs` (`v2-e11-s4`) and
//! `cli_mcp_component_derivation.rs` (`v2-e11-s5` PR1/PR5); extracted here
//! in PR6a so `support::client_routes` and `cli_mcp_openapi_coverage.rs`
//! reuse them instead of carrying byte-identical copies. The two audits keep
//! their private copies until PR6b switches them to this module.
//! `support::scan` remains the only tokenizer: nothing here masks comments
//! or string literals, callers pass already-masked code where that matters.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

pub(crate) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Truncates a Rust source file at its first `#[cfg(test)]`: every walker's
/// scope is production code only.
pub(crate) fn truncate_at_test_module(content: &str) -> String {
    match content.find("#[cfg(test)]") {
        Some(index) => content[..index].to_string(),
        None => content.to_string(),
    }
}

/// Reads `path` and truncates it at its test module.
pub(crate) fn read_production_source(path: &Path) -> String {
    let content = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    truncate_at_test_module(&content)
}

/// Every `.rs` file under `dir`, recursively, in directory-walk order.
pub(crate) fn rust_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).unwrap_or_else(|e| panic!("read_dir {current:?}: {e}"))
        {
            let entry = entry.expect("dir entry");
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

/// The source files of `module` declared under `parent_dir`: either the
/// single `<module>.rs` file or every `.rs` file under the `<module>/`
/// directory.
pub(crate) fn module_source_files(parent_dir: &Path, module: &str) -> Vec<PathBuf> {
    let single_file = parent_dir.join(format!("{module}.rs"));
    if single_file.is_file() {
        return vec![single_file];
    }

    let dir = parent_dir.join(module);
    if dir.is_dir() {
        return rust_files_recursive(&dir);
    }

    panic!("no source file or directory for module `{module}` under {parent_dir:?}");
}

// ---------------------------------------------------------------------------
// Depth-tracking scanners
// ---------------------------------------------------------------------------

/// A safe, in-bounds-or-sentinel byte read, so the depth-tracking scanners
/// never index a slice directly (`clippy::indexing_slicing`, denied
/// workspace-wide). `0` never collides with any byte this module matches on
/// (`(`, `)`, `{`, `}`, `[`, `]`, `,`, `;`, `"`, `\`).
pub(crate) fn byte_at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

/// Advances `i` past a `"…"` string literal's contents (honouring `\`
/// escapes), leaving `i` on the closing quote (or at `bytes.len()` if the
/// literal is unterminated).
pub(crate) fn skip_string_contents(bytes: &[u8], i: &mut usize) {
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
pub(crate) fn match_paren(text: &str, open: usize) -> usize {
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
/// any string literal.
pub(crate) fn split_top_level_comma(text: &str) -> Option<(String, String)> {
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
/// with `/`. Skips literals that don't start with `/` (a query-fragment
/// literal such as `"cursor={c}"` inside a helper's body never wins over
/// its own path-shaped literal).
pub(crate) fn first_path_literal(text: &str) -> Option<String> {
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
/// (bracket/brace/paren depth 0 is what ends the statement, so a
/// `match { .. }` or multi-line `format!(..)` right-hand side is captured
/// whole).
pub(crate) fn find_let_binding(scope: &str, ident: &str) -> Option<String> {
    let pattern = format!(r"let\s+(?:mut\s+)?{}\s*=\s*", regex::escape(ident));
    let re = Regex::new(&pattern).expect("valid regex");
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

/// The literal body of `const <const_name>` array in `source`, anchored on
/// the `=` that introduces the array so the type annotation's own `[...]`
/// (e.g. `&[(&str, Component)]`) is skipped.
pub(crate) fn extract_array_body(source: &str, const_name: &str) -> String {
    let marker = format!("const {const_name}");
    let const_start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("{const_name} not found"));
    let assign = source[const_start..]
        .find('=')
        .map(|offset| const_start + offset)
        .unwrap_or_else(|| panic!("{const_name}: no `=` found"));
    let open = source[assign..]
        .find('[')
        .map(|offset| assign + offset)
        .unwrap_or_else(|| panic!("{const_name}: no opening `[` found"));

    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match byte_at(bytes, i) {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return source[open + 1..i].to_string();
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!("{const_name}: unbalanced brackets");
}

// ---------------------------------------------------------------------------
// Identifiers and calls
// ---------------------------------------------------------------------------

pub(crate) fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && text.chars().next().is_some_and(|c| !c.is_ascii_digit())
}

/// If `text` (trimmed) is a plain call expression `name(..)`, returns
/// `name`.
pub(crate) fn call_target_name(text: &str) -> Option<String> {
    let text = text.trim();
    let paren = text.find('(')?;
    let name = &text[..paren];
    is_identifier(name).then(|| name.to_string())
}

/// Every distinct callee name of a `self.<name>(` call in `body`, in first-
/// occurrence order.
pub(crate) fn self_call_targets(body: &str) -> Vec<String> {
    let re = Regex::new(r"self\s*\.\s*(\w+)\s*\(").expect("valid regex");
    let mut targets: Vec<String> = Vec::new();

    for caps in re.captures_iter(body) {
        let name = caps[1].to_string();
        if !targets.contains(&name) {
            targets.push(name);
        }
    }

    targets
}

/// Whether the function whose boundary starts at `offset` is declared
/// `pub` (not `pub(crate)`, not private): the visibility that marks an
/// `AtlasClient` API method as opposed to its private transport helpers.
pub(crate) fn is_pub_fn(source: &str, offset: usize) -> bool {
    source
        .get(offset..)
        .map(str::trim_start)
        .is_some_and(|decl| decl.starts_with("pub ") || decl.starts_with("pub\t"))
}

pub(crate) fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `{...}` -> `{}` on every path segment, so `{ws}` and `{project_slug}`
/// compare equal: the client and the registry name the same positional
/// placeholder differently at several custos sites.
pub(crate) fn normalize_template(path: &str) -> String {
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

// ---------------------------------------------------------------------------
// Free-function boundaries (module-level `fn`, any indentation)
// ---------------------------------------------------------------------------

/// (start byte offset of the `fn` keyword's line, function name), sorted by
/// offset ascending: the enumeration `enclosing_fn_name` and
/// `function_body` walk.
pub(crate) fn function_boundaries(source: &str) -> Vec<(usize, String)> {
    let fn_re =
        Regex::new(r"(?m)^\s*(?:pub(?:\(crate\))? )?(?:async )?fn (\w+)").expect("valid regex");
    fn_re
        .captures_iter(source)
        .map(|caps| {
            let whole = caps.get(0).expect("match 0 exists");
            (whole.start(), caps[1].to_string())
        })
        .collect()
}

pub(crate) fn enclosing_fn_name(boundaries: &[(usize, String)], offset: usize) -> String {
    boundaries
        .iter()
        .rev()
        .find(|(start, _)| *start <= offset)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| "<module scope>".to_string())
}

/// The text from `name`'s own `fn` declaration to the next function
/// boundary (or end of `source`): a bounded window for local `let`
/// resolution and, for a path-builder helper, its own hardcoded literal.
pub(crate) fn function_body<'a>(
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

/// Resolves `expr` (an `AtlasClient` verb call's relative-path argument) to
/// its relative-path template:
///
/// 1. A literal or `format!("...")` directly in `expr`, used as-is.
/// 2. A bare identifier bound by a local `let` in `fn_body`, resolved from
///    its right-hand side (a direct literal/`format!` binding, or a
///    passthrough helper call whose literal argument is textually present).
/// 3. A call to an owning helper with no literal in its own call arguments,
///    resolved from the callee's own body: looked up first in `source` (the
///    calling method's own file), then in `root_source` (`lib.rs`), where
///    the private path-builder free functions still live after the
///    per-component split.
///
/// A trailing `?query=...` is stripped: registry templates never carry one.
pub(crate) fn resolve_relative(
    expr: &str,
    fn_body: &str,
    boundaries: &[(usize, String)],
    source: &str,
    root_source: &str,
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
            if let Some(callee_body) = function_body(source, boundaries, &callee) {
                first_path_literal(callee_body)
            } else {
                let root_boundaries = function_boundaries(root_source);
                let callee_body = function_body(root_source, &root_boundaries, &callee)?;
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

// ---------------------------------------------------------------------------
// `impl` methods and `"resource" => self.handler(..)` arms (atlas_mcp shape)
// ---------------------------------------------------------------------------

/// One method inside an `impl` block, bounded by its own header and the next
/// sibling method's header (or end of source for the last one). No brace
/// matching: the same next-marker slicing the CLI dispatch-arm parse uses,
/// applied one level up (methods rather than match arms).
pub(crate) struct McpFn {
    pub(crate) name: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) fn parse_impl_fn_boundaries(source: &str) -> Vec<McpFn> {
    let header_re = Regex::new(r"(?m)^    (?:pub(?:\(crate\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*\(")
        .expect("valid regex");

    let starts: Vec<(usize, String)> = header_re
        .captures_iter(source)
        .map(|caps| {
            let whole = caps.get(0).expect("match 0 exists");
            (whole.start(), caps[1].to_string())
        })
        .collect();

    starts
        .iter()
        .enumerate()
        .map(|(index, (start, name))| {
            let end = starts.get(index + 1).map_or(source.len(), |(s, _)| *s);
            McpFn {
                name: name.clone(),
                start: *start,
                end,
            }
        })
        .collect()
}

pub(crate) fn find_fn_body<'a>(fns: &[McpFn], source: &'a str, name: &str) -> Option<&'a str> {
    fns.iter()
        .find(|candidate| candidate.name == name)
        .map(|candidate| &source[candidate.start..candidate.end])
}

/// Finds the `impl` method whose body contains
/// `catalog::unknown_resource("<verb>", ..)`: every verb's
/// `match call.resource.as_str()` ends with exactly this fallback arm, so it
/// anchors the match block regardless of which function holds it (`delete`
/// delegates to `delete_resource`, `move` is spelled `move_resource`).
pub(crate) fn find_verb_match_body<'a>(fns: &'a [McpFn], source: &'a str, verb: &str) -> &'a str {
    let anchor = format!(r#"unknown_resource("{verb}","#);
    let mut matches = fns
        .iter()
        .filter(|candidate| source[candidate.start..candidate.end].contains(&anchor));

    let found = matches.next().unwrap_or_else(|| {
        panic!("no function contains `catalog::unknown_resource(\"{verb}\", ..)`")
    });
    assert!(
        matches.next().is_none(),
        "more than one function contains the `{verb}` unknown-resource anchor"
    );

    &source[found.start..found.end]
}

/// One `"resource" => ...` arm resolved to the handler function name its
/// body calls: both the decode shape (`self.h(catalog::decode(..)?, ctx)`)
/// and the bare shape (`self.h(ctx)`) contain exactly one `self.<name>(`
/// call in the arm's own text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct McpArm {
    pub(crate) resource: String,
    pub(crate) handler: String,
}

pub(crate) fn parse_resource_handlers(body: &str) -> Vec<McpArm> {
    let arm_re = Regex::new(r#""([a-z_]+)"\s*=>"#).expect("valid regex");
    let markers: Vec<(usize, usize, String)> = arm_re
        .captures_iter(body)
        .map(|caps| {
            let whole = caps.get(0).expect("match 0 exists");
            (whole.start(), whole.end(), caps[1].to_string())
        })
        .collect();

    // `self` and its call sometimes split across lines (`self\n    .list_attachments(`
    // when a `.map(ContentBlock::text)` chains after `.await`), so the gap
    // between `self` and `.` must be permitted, not just the gap after it.
    let handler_re = Regex::new(r"self\s*\.\s*(\w+)\s*\(").expect("valid regex");

    markers
        .iter()
        .enumerate()
        .filter_map(|(index, (_, end, resource))| {
            let arm_end = markers.get(index + 1).map_or(body.len(), |(s, _, _)| *s);
            let arm_text = &body[*end..arm_end];
            handler_re.captures(arm_text).map(|caps| McpArm {
                resource: resource.clone(),
                handler: caps[1].to_string(),
            })
        })
        .collect()
}
