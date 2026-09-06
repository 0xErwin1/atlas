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
//!
//! `v2-e11-s5` PR5 (design D6, MCP half) extends this same file with the
//! MCP-side walk: it proves every `catalog::OPERATIONS` entry's declared
//! `component` equals the sub-client namespace the entry's own dispatcher
//! arm resolves to, through `crates/atlas_mcp/src/lib.rs`'s verb-tool
//! `match call.resource.as_str()` arms into the handler function each arm
//! calls. Two independent facts again: [`parse_operations`] reads
//! `catalog.rs`'s own array as text; [`parse_resource_handlers`] and
//! [`resolve_handler_namespaces`] independently read `lib.rs`'s dispatch
//! arms and handler bodies. `identity/ping` is the MCP half of D3.2's
//! closed, justified zero-call-site exception (it answers `"pong"`
//! locally); `crates/atlas_mcp/src/main.rs`'s single `.custos().me()` call
//! is out of scope by a written rule (startup diagnostics, not an
//! operation), never read by this walk.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

use support::scan::scan;
use support::source_walk::{
    McpArm, McpFn, extract_array_body, find_fn_body, find_verb_match_body, module_source_files,
    parse_impl_fn_boundaries, parse_resource_handlers, read_production_source, repo_root,
};

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

fn cli_src_root() -> PathBuf {
    repo_root().join("crates/atlas_cli/src")
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
    for path in module_source_files(&cli_src_root().join("commands"), module) {
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

// ---------------------------------------------------------------------------
// PR5 — MCP half (design D6). Source: crates/atlas_mcp/src/{catalog.rs,lib.rs}.
// ---------------------------------------------------------------------------

fn mcp_src_root() -> PathBuf {
    repo_root().join("crates/atlas_mcp/src")
}

fn read_mcp_source(path: &Path) -> String {
    read_production_source(path)
}

fn mcp_lib_source() -> String {
    read_mcp_source(&mcp_src_root().join("lib.rs"))
}

// ---------------------------------------------------------------------------
// Step 1 — parse the declared tables out of atlas_mcp/src/catalog.rs
// ---------------------------------------------------------------------------

/// Extracts `(verb, resource, component)` from `OPERATIONS`'s own literal
/// `Operation { .. }` entries — a text parse, not a hand-retyped copy, for
/// the same reason as `parse_declared_components` on the CLI side.
fn parse_operations() -> Vec<(String, String, Component)> {
    let source = read_mcp_source(&mcp_src_root().join("catalog.rs"));
    let table = extract_array_body(&source, "OPERATIONS");

    let row_re = Regex::new(
        r#"verb:\s*"([a-z_]+)"[\s\S]*?resource:\s*"([a-z_]+)"[\s\S]*?component:\s*Component::(Acta|Custos|Platform)"#,
    )
    .expect("valid regex");

    row_re
        .captures_iter(&table)
        .map(|caps| {
            let verb = caps[1].to_string();
            let resource = caps[2].to_string();
            let component = match &caps[3] {
                "Acta" => Component::Acta,
                "Custos" => Component::Custos,
                "Platform" => Component::Platform,
                other => panic!("unknown Component variant {other}"),
            };
            (verb, resource, component)
        })
        .collect()
}

/// Extracts `(verb, resource)` from `NO_CALL_SITE`'s `(&str, &str, &str)`
/// rows (verb, resource, reason) — the reason string is not needed by this
/// audit; `catalog.rs`'s own tests cover its content.
fn parse_mcp_no_call_site() -> Vec<(String, String)> {
    let source = read_mcp_source(&mcp_src_root().join("catalog.rs"));
    let table = extract_array_body(&source, "NO_CALL_SITE");

    let row_re = Regex::new(r#"\(\s*"([a-z_]+)"\s*,\s*"([a-z_]+)"\s*,"#).expect("valid regex");
    row_re
        .captures_iter(&table)
        .map(|caps| (caps[1].to_string(), caps[2].to_string()))
        .collect()
}

/// Extracts the `VERBS` array's entries, in the order they are advertised.
fn parse_verbs() -> Vec<String> {
    let source = read_mcp_source(&mcp_src_root().join("catalog.rs"));
    let table = extract_array_body(&source, "VERBS");

    let row_re = Regex::new(r#""([a-z_]+)""#).expect("valid regex");
    row_re
        .captures_iter(&table)
        .map(|caps| caps[1].to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Step 2 — resolve a verb's advertised name to its resource-match body
// ---------------------------------------------------------------------------

#[test]
fn every_verb_has_exactly_one_resolvable_match_block() {
    let source = mcp_lib_source();
    let fns = parse_impl_fn_boundaries(&source);
    let verbs = parse_verbs();

    for verb in &verbs {
        let body = find_verb_match_body(&fns, &source, verb);
        assert!(
            body.contains("match call.resource.as_str()"),
            "the `{verb}` match block must dispatch on call.resource.as_str()"
        );
    }
}

// ---------------------------------------------------------------------------
// Step 5 — resolve a handler function's namespace set
// ---------------------------------------------------------------------------

fn resolve_handler_namespaces(fns: &[McpFn], source: &str, handler: &str) -> BTreeSet<Component> {
    let body = find_fn_body(fns, source, handler)
        .unwrap_or_else(|| panic!("no function body found for handler `{handler}`"));
    let masked = scan(body).code;
    resolve_namespaces_in_source(&masked)
}

// ---------------------------------------------------------------------------
// Step 6 — totality both ways (design D6, T5.1c)
// ---------------------------------------------------------------------------

/// A catalogued `(verb, resource)` with no matching dispatcher entry fails
/// as an unmatched operation; a dispatcher entry with no catalog entry fails
/// as an unmatched arm. Factored out so the real-tree test and its probe
/// exercise the same logic, not a duplicate copy of it.
fn check_totality(
    operations: &[(String, String, Component)],
    dispatcher: &BTreeMap<(String, String), String>,
) -> Vec<String> {
    let mut failures = Vec::new();

    let operation_keys: BTreeSet<(String, String)> = operations
        .iter()
        .map(|(v, r, _)| (v.clone(), r.clone()))
        .collect();

    for (verb, resource, _component) in operations {
        let key = (verb.clone(), resource.clone());
        if !dispatcher.contains_key(&key) {
            failures.push(format!(
                "{verb}/{resource}: catalogued operation has no dispatcher arm — unmatched"
            ));
        }
    }

    for key in dispatcher.keys() {
        if !operation_keys.contains(key) {
            failures.push(format!(
                "{}/{}: dispatcher arm has no catalog entry — unmatched",
                key.0, key.1
            ));
        }
    }

    failures
}

// ---------------------------------------------------------------------------
// T5.1 — self-test probes: the MCP walk must fail against a planted defect
// ---------------------------------------------------------------------------

#[test]
fn probe_d_a_dispatcher_arm_calling_a_different_namespace_than_catalogued_is_flagged_by_name() {
    let verb_body = r#"
        match call.resource.as_str() {
            "widget" => {
                self.synthetic_widget_handler(catalog::decode("synthetic_verb", "widget", call.params)?, ctx)
                    .await
            }
            other => Err(catalog::unknown_resource("synthetic_verb", other)),
        }
    "#;
    let handler_body = r#"
        async fn synthetic_widget_handler(
            &self,
            Parameters(params): Parameters<SyntheticParams>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<String, String> {
            let client = self.resolve_client(&ctx)?;
            let page = client.custos().list_widgets().await.map_err(|e| e.to_string())?;
            serde_json::to_string(&page).map_err(|e| e.to_string())
        }
    "#;

    let arms = parse_resource_handlers(verb_body);
    assert_eq!(
        arms,
        vec![McpArm {
            resource: "widget".to_string(),
            handler: "synthetic_widget_handler".to_string(),
        }]
    );

    let masked = scan(handler_body).code;
    let resolved = resolve_namespaces_in_source(&masked);

    let result = check_command("synthetic_verb/widget", Component::Acta, false, &resolved);
    let error = result
        .expect_err("a handler calling a different namespace than catalogued must be flagged");
    assert!(error.contains("synthetic_verb/widget"));
    assert!(error.contains("mismatch"));
}

#[test]
fn probe_e_both_real_mcp_arm_shapes_resolve_to_the_correct_handler_name() {
    // Verbatim from lib.rs's `find` verb: the decode shape.
    let decode_shape_arm = r#"
        "search" => {
            self.search(catalog::decode("find", "search", call.params)?, ctx)
                .await
        }
    "#;
    assert_eq!(
        parse_resource_handlers(decode_shape_arm),
        vec![McpArm {
            resource: "search".to_string(),
            handler: "search".to_string(),
        }]
    );

    // Verbatim from lib.rs's `find` verb: the bare shape (no
    // `catalog::decode` — `platform_status_templates` takes no parameters).
    let bare_shape_arm =
        r#""platform_status_templates" => self.list_platform_status_templates(ctx).await,"#;
    assert_eq!(
        parse_resource_handlers(bare_shape_arm),
        vec![McpArm {
            resource: "platform_status_templates".to_string(),
            handler: "list_platform_status_templates".to_string(),
        }]
    );
}

#[test]
fn probe_f_totality_both_ways_flags_an_unmatched_catalog_entry_and_an_unmatched_dispatcher_arm() {
    let operations = vec![
        ("find".to_string(), "widget".to_string(), Component::Acta),
        ("find".to_string(), "gadget".to_string(), Component::Acta),
    ];
    let mut dispatcher: BTreeMap<(String, String), String> = BTreeMap::new();
    dispatcher.insert(
        ("find".to_string(), "widget".to_string()),
        "list_widgets".to_string(),
    );
    dispatcher.insert(
        ("find".to_string(), "extra".to_string()),
        "list_extra".to_string(),
    );

    let failures = check_totality(&operations, &dispatcher);

    assert!(
        failures
            .iter()
            .any(|f| f.contains("find/gadget") && f.contains("no dispatcher arm")),
        "a catalogued operation with no dispatcher arm must be flagged: {failures:?}"
    );
    assert!(
        failures
            .iter()
            .any(|f| f.contains("find/extra") && f.contains("no catalog entry")),
        "a dispatcher arm with no catalog entry must be flagged: {failures:?}"
    );
    assert!(
        !failures.iter().any(|f| f.contains("find/widget")),
        "a matched pair must not be flagged: {failures:?}"
    );
}

// ---------------------------------------------------------------------------
// T5.3 — main.rs's `.custos().me()` is excluded by a written scope rule
// ---------------------------------------------------------------------------

/// `atlas_mcp/src/main.rs`'s single `.custos().me()` call is startup
/// diagnostics, not an operation (design D6, §0.4), and is excluded from
/// this walk by scope: the walk only ever reads `catalog.rs` and `lib.rs`.
/// This proves the exclusion is a written rule, not an accident of the
/// regex failing to match `main.rs` at all — first confirming the site is
/// real, then confirming the walked file set does not include it.
#[test]
fn main_rs_custos_me_call_is_excluded_by_a_written_scope_rule_not_a_regex_miss() {
    let main_path = mcp_src_root().join("main.rs");
    let main_source =
        fs::read_to_string(&main_path).unwrap_or_else(|e| panic!("read {main_path:?}: {e}"));
    let masked = scan(&main_source).code;
    let resolved = resolve_namespaces_in_source(&masked);
    assert!(
        resolved.contains(&Component::Custos),
        "main.rs must still contain a real `.custos()` call (startup diagnostics) — \
         if this ever becomes empty, the exclusion below would be vacuous"
    );

    let walked_files = [
        mcp_src_root().join("lib.rs"),
        mcp_src_root().join("catalog.rs"),
    ];
    assert!(
        !walked_files.contains(&main_path),
        "main.rs must not be part of the MCP walk's file set (startup diagnostics, not an operation)"
    );
}

// ---------------------------------------------------------------------------
// Real-tree checks: non-vacuity (design R4) and the actual MCP audit
// ---------------------------------------------------------------------------

#[test]
fn operations_table_has_112_rows_split_108_3_1() {
    let operations = parse_operations();
    assert_eq!(
        operations.len(),
        112,
        "parsed OPERATIONS must have 112 rows"
    );

    let acta = operations
        .iter()
        .filter(|(_, _, c)| *c == Component::Acta)
        .count();
    let custos = operations
        .iter()
        .filter(|(_, _, c)| *c == Component::Custos)
        .count();
    let platform = operations
        .iter()
        .filter(|(_, _, c)| *c == Component::Platform)
        .count();
    assert_eq!(
        (acta, custos, platform),
        (108, 3, 1),
        "measured split (design §0.4): 108 acta, 3 custos, 1 platform"
    );
}

#[test]
fn mcp_no_call_site_is_closed_to_identity_ping() {
    let listed = parse_mcp_no_call_site();
    assert_eq!(listed, vec![("identity".to_string(), "ping".to_string())]);
}

#[test]
fn verbs_table_has_11_entries() {
    let verbs = parse_verbs();
    assert_eq!(
        verbs.len(),
        11,
        "VERBS must have 11 entries, not the stale 12"
    );
}

#[test]
fn every_mcp_operation_matches_its_dispatcher_handler_component() {
    let operations = parse_operations();
    let no_call_site = parse_mcp_no_call_site();
    let verbs = parse_verbs();

    assert!(
        !operations.is_empty(),
        "anti-vacuity: no operations resolved"
    );
    assert!(!verbs.is_empty(), "anti-vacuity: no verbs resolved");

    let source = mcp_lib_source();
    let fns = parse_impl_fn_boundaries(&source);

    let mut dispatcher: BTreeMap<(String, String), String> = BTreeMap::new();
    for verb in &verbs {
        let body = find_verb_match_body(&fns, &source, verb);
        let arms = parse_resource_handlers(body);
        assert!(
            !arms.is_empty(),
            "anti-vacuity: verb `{verb}` resolved zero resource arms"
        );
        for arm in arms {
            dispatcher.insert((verb.clone(), arm.resource), arm.handler);
        }
    }

    let mut failures = check_totality(&operations, &dispatcher);

    let mut resolved_count = 0usize;
    for (verb, resource, component) in &operations {
        let key = (verb.clone(), resource.clone());
        let Some(handler) = dispatcher.get(&key) else {
            continue;
        };

        let resolved = resolve_handler_namespaces(&fns, &source, handler);
        resolved_count += 1;

        let is_listed = no_call_site.contains(&key);
        let label = format!("{verb}/{resource}");
        if let Err(error) = check_command(&label, *component, is_listed, &resolved) {
            failures.push(error);
        }
    }

    assert!(
        failures.is_empty(),
        "MCP component derivation audit failures:\n{}",
        failures.join("\n")
    );
    assert_eq!(
        resolved_count, 112,
        "anti-vacuity: expected to resolve all 112 operations to a dispatcher handler"
    );
}

#[test]
fn identity_ping_resolves_to_no_namespace_and_is_the_sole_no_call_site_entry() {
    let source = mcp_lib_source();
    let fns = parse_impl_fn_boundaries(&source);
    let resolved = resolve_handler_namespaces(&fns, &source, "ping");
    assert!(
        resolved.is_empty(),
        "identity/ping answers \"pong\" locally and calls no sub-client"
    );

    let no_call_site = parse_mcp_no_call_site();
    assert_eq!(
        no_call_site,
        vec![("identity".to_string(), "ping".to_string())]
    );
}

#[test]
fn the_three_custos_operations_resolve_to_custos_via_their_own_handlers() {
    let source = mcp_lib_source();
    let fns = parse_impl_fn_boundaries(&source);

    for handler in [
        "get_agent_identity",
        "get_workspace_audit",
        "get_platform_audit",
    ] {
        let resolved = resolve_handler_namespaces(&fns, &source, handler);
        assert_eq!(
            resolved,
            BTreeSet::from([Component::Custos]),
            "{handler} must resolve exclusively to client.custos()"
        );
    }
}
