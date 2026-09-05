#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `v2-e11-s5` PR1 (design D6, CLI half). PR5 adds the MCP half to this same
//! file.
//!
//! Proves `crates/atlas_cli/src/component.rs`'s declared component for every
//! CLI command equals the `AtlasClient` sub-client namespace
//! (`.acta()`/`.custos()`/`.platform()`) that command's own handler module
//! calls through — mechanically, by walking source text, rather than by a
//! reviewer trusting a hand-written table.
//!
//! **Why source-walking, from `atlas_server`.** `atlas_cli` is a bin-only
//! crate (no `[lib]` target); making it one, or making `atlas_mcp::catalog`
//! public, would widen a surface this slice's own invariant says does not
//! change (design §0.6). `atlas_server` already walks `atlas_client`'s
//! source for the same class of question
//! (`atlas_client_route_contract.rs`, `client_call_shape_guard.rs`); this
//! module extends that pattern one hop further, into `atlas_cli`'s
//! `dispatch` and command modules.
//!
//! **Two independent facts, not one.** [`parse_declared_components`] and
//! [`parse_no_call_site`] read `atlas_cli/src/component.rs`'s own literal
//! arrays as text; [`resolve_module_namespaces`] independently reads each
//! command module's own `.acta()`/`.custos()`/`.platform()` sites. Neither
//! function is aware of the other's answer, so [`check_command`] can
//! actually fail — the whole point of an audit rather than a duplicate copy
//! of the same table (design D3, D6).
//!
//! **Masking.** Every source file this module reads is passed through
//! `support::scan::scan`, the same string/comment-masking tokenizer
//! `client_call_shape_guard.rs` uses, so a doc comment or a string literal
//! that merely mentions `.acta()` is never counted as a real call site.
//! `#[cfg(test)] mod tests { .. }` is truncated off first, matching
//! `atlas_client_route_contract.rs:59-64`'s scope rule.

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use support::scan::scan;

// ---------------------------------------------------------------------------
// Component (independent of atlas_cli's own enum — atlas_cli is bin-only
// and unreachable as a dependency, design §0.6; this is the audit's own
// copy of the concept, not a shared type).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Component {
    Acta,
    Custos,
    Platform,
}

impl Component {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "acta" => Some(Component::Acta),
            "custos" => Some(Component::Custos),
            "platform" => Some(Component::Platform),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Component::Acta => "acta",
            Component::Custos => "custos",
            Component::Platform => "platform",
        }
    }
}

// ---------------------------------------------------------------------------
// Source locations
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cli_src_root() -> PathBuf {
    repo_root().join("crates/atlas_cli/src")
}

fn read_production_source(path: &Path) -> String {
    let content = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    truncate_at_test_module(&content)
}

/// Truncates a Rust source file at its first `#[cfg(test)]`, the same scope
/// rule `atlas_client_route_contract.rs:59-64` uses.
fn truncate_at_test_module(content: &str) -> String {
    match content.find("#[cfg(test)]") {
        Some(index) => content[..index].to_string(),
        None => content.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Step 1 — parse the two declared tables out of atlas_cli/src/component.rs
// ---------------------------------------------------------------------------

/// Extracts the `COMMAND_COMPONENTS` array's literal `("name", Component::X)`
/// rows from `component.rs`'s own source text. A text parse, not a
/// hand-retyped copy — a second hand-typed table would trivially always
/// agree with itself and prove nothing (design D3's rejected alternative).
fn parse_declared_components() -> Vec<(String, Component)> {
    let source = read_production_source(&cli_src_root().join("component.rs"));
    let table = extract_array_body(&source, "COMMAND_COMPONENTS");

    let row_re = Regex::new(r#"\(\s*"([a-z-]+)"\s*,\s*Component::(Acta|Custos|Platform)\s*\)"#)
        .expect("valid regex");
    row_re
        .captures_iter(&table)
        .map(|caps| {
            let name = caps[1].to_string();
            let component = match &caps[2] {
                "Acta" => Component::Acta,
                "Custos" => Component::Custos,
                "Platform" => Component::Platform,
                other => panic!("unknown Component variant {other}"),
            };
            (name, component)
        })
        .collect()
}

/// Extracts the `NO_CALL_SITE` array's `"name"` entries (the reason string
/// is not needed by this audit; `component.rs`'s own tests cover its
/// content).
fn parse_no_call_site() -> Vec<String> {
    let source = read_production_source(&cli_src_root().join("component.rs"));
    let table = extract_array_body(&source, "NO_CALL_SITE");

    let row_re = Regex::new(r#"\(\s*"([a-z-]+)"\s*,"#).expect("valid regex");
    row_re
        .captures_iter(&table)
        .map(|caps| caps[1].to_string())
        .collect()
}

/// Returns the text between `pub(crate) const <name>: ... = &[` and its
/// matching `];`, by counting `[`/`]` depth from the opening bracket.
fn extract_array_body(source: &str, const_name: &str) -> String {
    let marker = format!("const {const_name}");
    let const_start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("{const_name} not found in component.rs"));
    // Skip past the type annotation's own `[...]` (e.g. `&[(&str, Component)]`)
    // by anchoring on the `=` that introduces the array literal itself.
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
        match bytes.get(i).copied().unwrap_or(0) {
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
// Step 2 — parse dispatch's match arms: Commands::X -> module (or none)
// ---------------------------------------------------------------------------

/// One `Commands::X => ...` arm from `commands/mod.rs::dispatch`.
#[derive(Debug, Clone)]
struct DispatchArm {
    /// The `Commands` variant identifier, e.g. `PlatformStatusTemplates`.
    variant: String,
    /// The module the arm dispatches into, resolved from the first
    /// `snake_case_ident::` reference in the arm's body — `None` for
    /// `Commands::Version`, whose body is inline with no module reference.
    module: Option<String>,
    /// The arm's own body text (used to resolve namespaces directly when
    /// `module` is `None`).
    body: String,
}

fn parse_dispatch_arms() -> Vec<DispatchArm> {
    let source = read_production_source(&cli_src_root().join("commands/mod.rs"));
    let masked = scan(&source).code;

    let arm_re = Regex::new(r"Commands::(\w+)\s*(?:\([^)]*\))?\s*=>").expect("valid regex");
    let markers: Vec<(usize, usize, String)> = arm_re
        .captures_iter(&masked)
        .map(|caps| {
            let whole = caps.get(0).unwrap();
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

/// CamelCase -> kebab-case, matching clap's default `Subcommand` derive
/// naming (`PlatformStatusTemplates` -> `platform-status-templates`).
fn camel_to_kebab(variant: &str) -> String {
    let mut out = String::new();
    for (index, ch) in variant.char_indices() {
        if ch.is_uppercase() && index != 0 {
            out.push('-');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

// ---------------------------------------------------------------------------
// Step 3 — resolve a module's (or an inline arm body's) namespace set
// ---------------------------------------------------------------------------

/// Every source file under `crates/atlas_cli/src/commands/<module>.rs` or,
/// when that module is itself a directory (e.g. `import`, `export`, each
/// with a nested `obsidian/` tree), every `.rs` file under
/// `crates/atlas_cli/src/commands/<module>/`.
fn module_source_files(module: &str) -> Vec<PathBuf> {
    let single_file = cli_src_root().join("commands").join(format!("{module}.rs"));
    if single_file.is_file() {
        return vec![single_file];
    }

    let dir = cli_src_root().join("commands").join(module);
    if dir.is_dir() {
        return rust_files_recursive(&dir);
    }

    panic!("no source file or directory for module `{module}`");
}

fn rust_files_recursive(dir: &Path) -> Vec<PathBuf> {
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

/// Finds every `.acta()`/`.custos()`/`.platform()` namespace accessor call
/// in already-masked `code` (comments and string-literal contents already
/// stripped by [`scan`]).
fn resolve_namespaces_in_source(masked_code: &str) -> BTreeSet<Component> {
    let re = Regex::new(r"\.\s*(acta|custos|platform)\s*\(\s*\)").expect("valid regex");
    re.captures_iter(masked_code)
        .filter_map(|caps| Component::from_str(&caps[1]))
        .collect()
}

fn resolve_module_namespaces(module: &str) -> BTreeSet<Component> {
    let mut namespaces = BTreeSet::new();
    for path in module_source_files(module) {
        let source = read_production_source(&path);
        let masked = scan(&source).code;
        namespaces.extend(resolve_namespaces_in_source(&masked));
    }
    namespaces
}

// ---------------------------------------------------------------------------
// Step 4 — the audit rule (design D3.2): exactly one, or zero and listed
// ---------------------------------------------------------------------------

fn check_command(
    name: &str,
    declared: Component,
    is_listed_no_call_site: bool,
    resolved: &BTreeSet<Component>,
) -> Result<(), String> {
    if is_listed_no_call_site && !resolved.is_empty() {
        let calls: Vec<&str> = resolved.iter().map(|c| c.as_str()).collect();
        return Err(format!(
            "{name}: listed in NO_CALL_SITE but now calls {calls:?} — stale NO_CALL_SITE entry"
        ));
    }

    if resolved.is_empty() {
        if is_listed_no_call_site {
            return Ok(());
        }
        return Err(format!(
            "{name}: calls no sub-client and is not listed in NO_CALL_SITE — unnamed gap"
        ));
    }

    if resolved.len() > 1 {
        let calls: Vec<&str> = resolved.iter().map(|c| c.as_str()).collect();
        return Err(format!(
            "{name}: calls multiple namespaces {calls:?} — must be split"
        ));
    }

    let only = *resolved.iter().next().expect("non-empty checked above");
    if only != declared {
        return Err(format!(
            "{name}: declared {} but calls {} — mismatch",
            declared.as_str(),
            only.as_str()
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// T1.4 — self-test probes: the walk must fail against a planted defect
// ---------------------------------------------------------------------------

#[test]
fn probe_a_a_command_calling_two_namespaces_is_flagged_by_name() {
    let masked = scan("async fn run() {\n    ctx.client.acta().list().await?;\n    ctx.client.custos().list().await?;\n}\n").code;
    let resolved = resolve_namespaces_in_source(&masked);

    let result = check_command("synthetic-acta-command", Component::Acta, false, &resolved);

    let error = result.expect_err("a command calling two namespaces must be flagged");
    assert!(error.contains("synthetic-acta-command"));
    assert!(error.contains("multiple namespaces"));
}

#[test]
fn probe_b_an_unlisted_zero_call_site_command_fails_as_an_unnamed_gap() {
    let masked = scan("fn run() {\n    println!(\"nothing to call\");\n}\n").code;
    let resolved = resolve_namespaces_in_source(&masked);

    let result = check_command(
        "synthetic-zero-site-command",
        Component::Platform,
        false,
        &resolved,
    );

    let error = result.expect_err("an unlisted zero-call-site command must fail");
    assert!(error.contains("synthetic-zero-site-command"));
    assert!(error.contains("unnamed gap"));
}

#[test]
fn probe_c_a_no_call_site_listed_command_given_a_call_site_fails_as_stale() {
    let masked = scan("fn run() {\n    ctx.client.acta().list().await?;\n}\n").code;
    let resolved = resolve_namespaces_in_source(&masked);

    let result = check_command(
        "synthetic-listed-command",
        Component::Platform,
        true,
        &resolved,
    );

    let error = result.expect_err("a listed command that now calls a sub-client must fail");
    assert!(error.contains("synthetic-listed-command"));
    assert!(error.contains("stale"));
}

#[test]
fn a_command_matching_its_declared_component_passes() {
    let masked = scan("async fn run() { ctx.client.acta().list().await?; }").code;
    let resolved = resolve_namespaces_in_source(&masked);
    check_command("synthetic-ok", Component::Acta, false, &resolved).expect("must pass");
}

#[test]
fn a_no_call_site_listed_command_with_no_call_site_passes() {
    let resolved: BTreeSet<Component> = BTreeSet::new();
    check_command("synthetic-listed-ok", Component::Platform, true, &resolved).expect("must pass");
}

#[test]
fn a_masked_comment_mentioning_a_namespace_is_never_counted() {
    let masked = scan("/// Calls ctx.client.acta().list() to fetch the page.\nfn run() {}\n").code;
    let resolved = resolve_namespaces_in_source(&masked);
    assert!(
        resolved.is_empty(),
        "a doc comment mentioning .acta() must not be counted as a real call site"
    );
}

// ---------------------------------------------------------------------------
// Real-tree checks: non-vacuity (design R4) and the actual audit
// ---------------------------------------------------------------------------

#[test]
fn declared_components_table_has_28_rows() {
    let declared = parse_declared_components();
    assert_eq!(
        declared.len(),
        28,
        "parsed COMMAND_COMPONENTS must have 28 rows"
    );
}

#[test]
fn no_call_site_is_closed_to_version_config_completions() {
    let mut listed = parse_no_call_site();
    listed.sort();
    assert_eq!(listed, vec!["completions", "config", "version"]);
}

#[test]
fn dispatch_arms_are_total_over_declared_components() {
    let arms = parse_dispatch_arms();
    assert_eq!(arms.len(), 28, "dispatch must have exactly 28 match arms");

    let declared = parse_declared_components();
    let declared_names: BTreeSet<&str> = declared.iter().map(|(name, _)| name.as_str()).collect();
    let arm_names: BTreeSet<String> = arms
        .iter()
        .map(|arm| camel_to_kebab(&arm.variant))
        .collect();
    let arm_names_ref: BTreeSet<&str> = arm_names.iter().map(String::as_str).collect();

    let missing_declaration: Vec<&&str> = arm_names_ref.difference(&declared_names).collect();
    assert!(
        missing_declaration.is_empty(),
        "dispatch arm(s) with no COMMAND_COMPONENTS row: {missing_declaration:?}"
    );

    let missing_dispatch_arm: Vec<&&str> = declared_names.difference(&arm_names_ref).collect();
    assert!(
        missing_dispatch_arm.is_empty(),
        "COMMAND_COMPONENTS row(s) with no dispatch arm: {missing_dispatch_arm:?}"
    );
}

#[test]
fn every_declared_command_matches_its_derived_call_site() {
    let declared = parse_declared_components();
    let no_call_site = parse_no_call_site();
    let arms = parse_dispatch_arms();

    assert!(
        !declared.is_empty(),
        "anti-vacuity: no declared components resolved"
    );
    assert!(!arms.is_empty(), "anti-vacuity: no dispatch arms resolved");

    let mut failures = Vec::new();
    let mut resolved_count = 0usize;

    for (name, component) in &declared {
        let arm = arms
            .iter()
            .find(|arm| camel_to_kebab(&arm.variant) == *name)
            .unwrap_or_else(|| panic!("no dispatch arm for declared command `{name}`"));

        let resolved = match &arm.module {
            Some(module) => resolve_module_namespaces(module),
            None => resolve_namespaces_in_source(&arm.body),
        };
        resolved_count += 1;

        let is_listed = no_call_site.iter().any(|listed| listed == name);
        if let Err(error) = check_command(name, *component, is_listed, &resolved) {
            failures.push(error);
        }
    }

    assert_eq!(
        resolved_count, 28,
        "anti-vacuity: expected to resolve all 28 commands"
    );
    assert!(
        failures.is_empty(),
        "component derivation audit failures:\n{}",
        failures.join("\n")
    );
}

/// T1.6 — recorded explicitly so a later reviewer cannot silently "correct"
/// the table to match the command names (design R3, PR1 review posture).
#[test]
fn platform_status_templates_resolves_to_acta_and_audit_resolves_to_custos() {
    let platform_status_templates = resolve_module_namespaces("platform_status_templates");
    assert_eq!(
        platform_status_templates,
        BTreeSet::from([Component::Acta]),
        "atlas platform-status-templates calls exclusively through client.acta()"
    );

    let audit = resolve_module_namespaces("audit");
    assert_eq!(
        audit,
        BTreeSet::from([Component::Custos]),
        "atlas audit calls exclusively through client.custos()"
    );
}

#[test]
fn import_and_export_resolve_to_acta_across_their_nested_obsidian_files() {
    let import = resolve_module_namespaces("import");
    assert_eq!(import, BTreeSet::from([Component::Acta]));

    let export = resolve_module_namespaces("export");
    assert_eq!(export, BTreeSet::from([Component::Acta]));
}

#[test]
fn version_has_no_module_and_no_call_site() {
    let arms = parse_dispatch_arms();
    let version_arm = arms
        .iter()
        .find(|arm| arm.variant == "Version")
        .expect("Version arm must exist");
    assert!(
        version_arm.module.is_none(),
        "Version dispatches inline, no module"
    );

    let resolved = resolve_namespaces_in_source(&version_arm.body);
    assert!(resolved.is_empty(), "Version issues no request");
}
