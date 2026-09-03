#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! CI grep gate (`v2-e3-s5` design D3), structurally a copy of
//! `schema_qualification_gate.rs`: a source-tree-walking `#[test]` proving
//! that `tests/support/path.rs` (`api_path`/`api_url`) is the only place an
//! `atlas_server` integration test builds a request path against the `/api`
//! namespace, instead of hand-typing a literal that S7's namespace cutover
//! would otherwise leave stranded.
//!
//! **Scope**: `crates/atlas_server/tests/**/*.rs` only — never
//! `crates/atlas_server/src`, `atlas_client`, or `atlas_mcp`, which are out
//! of this slice's authority (S6/S7's scope).
//!
//! **Match rule**: the file is tokenized (see [`scan`]) so that only the
//! contents of string literals — normal `"…"` and raw `r"…"`/`r#"…"#` — are
//! candidates; `//`, `///`, `//!`, and nested `/* … */` comment text is
//! skipped, and a `//` inside a literal (`"http://{addr}/…"`) is literal
//! text, never a comment. Inside a literal, a hit is any `/api` that is
//! preceded by the start of the literal, `}`, `/`, or an ASCII alphanumeric
//! (a bare `"/api"`, a `format!` fragment such as `"{}/api/…"` or
//! `"http://{addr}/api/…"`, a literal host such as
//! `"http://localhost:3000/api/…"`) and followed by `/`, the end of the
//! literal, or `?`. `/api-keys` (a real custos registry path) never
//! matches, because after `/api` the next character is neither of the
//! three; prose such as `"GET /api/x failed"`, a label such as `"V1 (/api)"`,
//! a markdown link `"[x](/api/…)"`, or fixture source `path = "/api/…"`
//! never matches, because the character before `/api` is a space, `(`, or
//! `"` — punctuation other than `}` and `/` is excluded on purpose.
//!
//! **Allowlist**: `ALLOWED` names every file permitted to contain a matching
//! literal, with a stated reason, as a typed [`Exemption`] checked in both
//! directions in the same test run. `WholeFile` exempts every matching
//! literal in the file and is stale (FAILS) when the file has none left.
//! `MarkedSpans` exempts only literals inside a
//! `// api-path-guard:off` / `// api-path-guard:on` span (mirroring
//! `schema_qualification_gate.rs`'s own `schema-gate:off`/`:on` precedent):
//! a matching literal outside a span FAILS naming `file:line`, and the entry
//! is stale (FAILS) unless at least one span contains a matching literal. An
//! unlisted file with a matching literal FAILS, inside or outside a marker
//! span: a marker pair without a `MarkedSpans` entry exempts nothing, so
//! the allowlist stays the single place a reason lives.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// How one allowlisted file is exempt from the unlisted-literal check.
enum Exemption {
    /// Every matching literal in `path` is exempt; stale once none is left.
    WholeFile {
        path: &'static str,
        reason: &'static str,
    },
    /// Only literals inside a `// api-path-guard:off`/`:on` span of `path`
    /// are exempt; one outside a span is a hit, and the entry is stale unless
    /// at least one span contains a matching literal.
    MarkedSpans {
        path: &'static str,
        reason: &'static str,
    },
}

impl Exemption {
    fn path(&self) -> &'static str {
        match self {
            Self::WholeFile { path, .. } | Self::MarkedSpans { path, .. } => path,
        }
    }

    fn variant(&self) -> &'static str {
        match self {
            Self::WholeFile { .. } => "WholeFile",
            Self::MarkedSpans { .. } => "MarkedSpans",
        }
    }

    fn reason(&self) -> &'static str {
        match self {
            Self::WholeFile { reason, .. } | Self::MarkedSpans { reason, .. } => reason,
        }
    }
}

const fn whole_file(path: &'static str, reason: &'static str) -> Exemption {
    Exemption::WholeFile { path, reason }
}

const fn marked_spans(path: &'static str, reason: &'static str) -> Exemption {
    Exemption::MarkedSpans { path, reason }
}

/// Every file permitted to contain a matching `/api` literal, and why. No
/// current entry is `MarkedSpans`: none of these files carries a marker
/// pair, so each is exempt as a whole. Entries marked `TRANSIENT` are
/// removed by PR2/PR3 as their file is migrated onto `api_path`/`api_url`;
/// the bidirectional check's stale-entry direction is what keeps that debt
/// visible if a later PR forgets one (design R6).
const ALLOWED: &[Exemption] = &[
    whole_file(
        "support/path.rs",
        "the one place the namespace is constructed",
    ),
    whole_file(
        "api_router_mount_assertion.rs",
        "a_foreign_prefix_never_matches_any_declared_route and \
         flat_and_wrong_component_v2_forms_never_match_any_declared_route construct \
         malformed and wrong-component V2-shaped paths the helper must never produce",
    ),
    whole_file(
        "api_unmatched_path_fallback.rs",
        "negative probe: a nonexistent path must never resolve",
    ),
    whole_file(
        "idempotency_middleware.rs",
        "40 synthetic mock-route literals on a test-local router, plus the canonical \
         idempotency store-key literals",
    ),
    whole_file(
        "idempotency_repo.rs",
        "a canonical store-key row, not a request path",
    ),
    // TRANSIENT (PR1 only) — not yet migrated onto api_path/api_url; removed
    // file-by-file as PR2/PR3 land (design R6, spec "counts are a review
    // aid, the gate is the guard's zero-unallowlisted-literal property").
    whole_file(
        "api_comment_attachments.rs",
        "TRANSIENT: pending v2-e3-s5 PR2",
    ),
    whole_file(
        "api_integration_configs.rs",
        "TRANSIENT: pending v2-e3-s5 PR2",
    ),
    whole_file("api_activation.rs", "TRANSIENT: pending v2-e3-s5 PR2"),
    whole_file("api_webhooks.rs", "TRANSIENT: pending v2-e3-s5 PR2"),
    whole_file("api_automation_rules.rs", "TRANSIENT: pending v2-e3-s5 PR2"),
    whole_file("api_comments.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_documents.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file(
        "api_capability_sweep.rs",
        "TRANSIENT: pending v2-e3-s5 PR3 (Class-B relative-path literals handed to \
         AtlasClient)",
    ),
    whole_file("api_csrf_ratelimit.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_task_attachments.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_trash.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_boards_tasks.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_semantic_search.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file(
        "e2e_incoming_automation.rs",
        "TRANSIENT: pending v2-e3-s5 PR3",
    ),
    whole_file("visibility_ancestors.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_audit_writes.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file(
        "api_global_admin_bypass.rs",
        "TRANSIENT: pending v2-e3-s5 PR3",
    ),
    whole_file("api_login_rate_limit.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_presence.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file(
        "api_presence_document.rs",
        "TRANSIENT: pending v2-e3-s5 PR3",
    ),
    whole_file("api_rate_limit.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_search.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file(
        "api_search_permissions.rs",
        "TRANSIENT: pending v2-e3-s5 PR3",
    ),
    whole_file("e2e_webhooks.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_events_stream.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_folders.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_members.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_permissions.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_presence_agent.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file(
        "api_search_pagination.rs",
        "TRANSIENT: pending v2-e3-s5 PR3",
    ),
    whole_file("api_self_protection.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_system_admin.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file("api_user_api_keys.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
    whole_file(
        "api_workspace_activity.rs",
        "TRANSIENT: pending v2-e3-s5 PR3",
    ),
    whole_file("grant_hygiene.rs", "TRANSIENT: pending v2-e3-s5 PR3"),
];

const MARKER_OFF: &str = "// api-path-guard:off";
const MARKER_ON: &str = "// api-path-guard:on";

#[test]
fn no_unallowlisted_api_literal_exists_outside_the_helper() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let tests_dir = Path::new(manifest_dir).join("tests");

    let violations = violations(&tests_dir, ALLOWED);

    assert!(
        violations.is_empty(),
        "api-path guard violations:\n{}",
        violations.join("\n")
    );
}

/// A `MarkedSpans` entry exempts only the span: the literal outside it is a
/// hit naming its line, while the one inside keeps the entry from being
/// stale.
#[test]
fn a_marked_spans_file_with_a_literal_outside_its_span_is_a_hit() {
    let probe_content = format!(
        "{MARKER_OFF}\nconst INSIDE: &str = \"{0}\";\n{MARKER_ON}\nconst OUTSIDE: &str = \"{0}\";\n",
        api_literal("/x")
    );
    let allowed = [marked_spans("probe.rs", "probe")];

    let violations = probe_violations("stray_probe", &probe_content, &allowed);

    assert_eq!(
        violations,
        vec!["probe.rs:4: matching literal outside a marked span (MarkedSpans entry)"]
    );
}

/// A marker pair without a `MarkedSpans` entry exempts nothing: an unlisted
/// file whose only matching literal sits inside a span is still a hit.
#[test]
fn an_unlisted_file_with_a_literal_only_inside_a_span_is_a_hit() {
    let probe_content = format!(
        "{MARKER_OFF}\nconst INSIDE: &str = \"{}\";\n{MARKER_ON}\n",
        api_literal("/x")
    );

    let violations = probe_violations("unlisted_span_probe", &probe_content, &[]);

    assert_eq!(
        violations,
        vec!["probe.rs:2: matching literal in a file with no allowlist entry"]
    );
}

/// An empty span cannot justify a `MarkedSpans` entry: with no literal inside
/// any span, the entry is stale.
#[test]
fn a_marked_spans_file_with_an_empty_span_is_stale() {
    let probe_content = format!("{MARKER_OFF}\n{MARKER_ON}\nfn ok() {{}}\n");
    let allowed = [marked_spans("probe.rs", "probe")];

    let violations = probe_violations("empty_span_probe", &probe_content, &allowed);

    assert_eq!(
        violations,
        vec![
            "probe.rs: stale MarkedSpans entry (probe), no marked span contains a matching literal"
        ]
    );
}

/// Boundary test: the gate must flag a deliberately reintroduced,
/// unallowlisted `/api` literal, naming its line. Writes a probe file into
/// a scratch tree shaped like `crates/atlas_server/tests`, asserts the scan
/// reports it, then removes the scratch tree. Built via `format!` so this
/// guard's own source never contains the pattern it forbids.
#[test]
fn the_gate_flags_a_reintroduced_unallowlisted_literal() {
    let probe_content = format!(
        "fn regression() -> &'static str {{\n    \"{}\"\n}}\n",
        api_literal("/foo")
    );

    let literal_files = scan_scratch_probe("probe", &probe_content);

    assert_eq!(
        literal_files,
        vec![("probe.rs".to_string(), 2)],
        "expected the gate to flag the reintroduced literal in probe.rs"
    );
}

/// The shapes a line-based `//` strip used to hide or misread: a `format!`
/// URL whose `http://` precedes the `/api` segment, a `'"'` char literal
/// before the offending literal, and a raw string. Each must be flagged,
/// and the first offender's line must be the URL's.
#[test]
fn the_gate_flags_literals_a_line_based_comment_strip_would_hide() {
    let url = format!("http://{{addr}}{}", api_literal("/mock-slow"));
    let probe_content = format!(
        "fn hidden() {{\n    let quote = '\"';\n    let url = format!(\"{url}\");\n    \
         let raw = r#\"{}\"#;\n}}\n",
        api_literal("/raw")
    );

    let literal_files = scan_scratch_probe("hidden_probe", &probe_content);

    assert_eq!(
        literal_files,
        vec![("probe.rs".to_string(), 3)],
        "expected the gate to flag the http:// format literal on line 3"
    );
}

/// A `//`, `///`, `//!`, or nested `/* … */` span containing a matching
/// literal — quoted or not — must not trigger the guard when it is the
/// file's only occurrence.
#[test]
fn a_comment_only_occurrence_does_not_trigger_the_guard() {
    let literal = api_literal("/foo");
    let probe_content = format!(
        "// see {literal} for context\n/// {literal}\n//! \"{literal}\"\n\
         /* \"{literal}\" */\n/* outer /* \"{literal}\" */ still a comment */\nfn ok() {{}}\n"
    );

    let literal_files = scan_scratch_probe("comment_probe", &probe_content);

    assert!(
        literal_files.is_empty(),
        "a comment-only occurrence must not be flagged: {literal_files:?}"
    );
}

/// The `/api-keys` false-positive regression: 5 real files carry this
/// literal (a genuine custos registry path), and the boundary rule must
/// never flag it.
#[test]
fn the_boundary_rule_never_flags_api_keys() {
    assert!(!literal_matches_boundary_pattern("/api-keys"));
    assert!(!literal_matches_boundary_pattern("{}/api-keys"));
}

/// The predecessor half of the boundary rule: a `format!` fragment whose
/// `/api` follows a `{…}` placeholder or `/` is a request path, while prose
/// where `/api` follows a space or `(` is a diagnostic message.
#[test]
fn the_boundary_rule_accepts_url_fragments_and_rejects_prose() {
    assert!(literal_matches_boundary_pattern(&api_literal("")));
    assert!(literal_matches_boundary_pattern(&api_literal("/x")));
    assert!(literal_matches_boundary_pattern(&api_literal("?q=1")));
    assert!(literal_matches_boundary_pattern(&format!(
        "{{}}{}",
        api_literal("/workspaces/{}/tags")
    )));
    assert!(literal_matches_boundary_pattern(&format!(
        "http://{{addr}}{}",
        api_literal("/mock-slow")
    )));
    assert!(literal_matches_boundary_pattern(&format!(
        "{{base_url}}{}",
        api_literal("/auth/login")
    )));
    assert!(literal_matches_boundary_pattern(&format!(
        "/{}",
        api_literal("/x")
    )));

    assert!(!literal_matches_boundary_pattern(&format!(
        "GET {} must succeed",
        api_literal("/auth/me")
    )));
    assert!(!literal_matches_boundary_pattern(&format!(
        "path = \"{}\"",
        api_literal("/fake/x")
    )));
    assert!(!literal_matches_boundary_pattern(&format!(
        "V1 ({})",
        api_literal("")
    )));
    assert!(!literal_matches_boundary_pattern(&format!(
        "[owner]({})",
        api_literal("/workspaces/ws/tasks")
    )));
}

/// A literal host before `/api` (an ASCII alphanumeric predecessor) is a
/// request URL and must be a hit.
#[test]
fn the_boundary_rule_accepts_a_literal_host_before_the_namespace() {
    assert!(literal_matches_boundary_pattern(&format!(
        "http://localhost:3000{}",
        api_literal("/x")
    )));
    assert!(literal_matches_boundary_pattern(&format!(
        "http://127.0.0.1:8080{}",
        api_literal("")
    )));
}

/// Prose where `/api` follows a space stays a diagnostic message, not a
/// request path, under the widened predecessor set.
#[test]
fn the_boundary_rule_rejects_prose_with_a_space_before_the_namespace() {
    assert!(!literal_matches_boundary_pattern(&format!(
        "GET {}",
        api_literal("/x")
    )));
}

#[test]
fn scan_treats_a_double_slash_inside_a_string_as_literal_text() {
    let source = format!("let url = \"http://{{addr}}{}\";\n", api_literal("/mock"));

    let scanned = scan(&source);

    assert_eq!(
        literal_texts(&scanned),
        vec![format!("http://{{addr}}{}", api_literal("/mock"))]
    );
    assert_eq!(scanned.code, "let url = ;\n");
}

#[test]
fn scan_drops_a_line_comment_after_code() {
    let source = format!("let x = 1; // \"{}\"\nlet y = 2;\n", api_literal("/x"));

    let scanned = scan(&source);

    assert!(literal_texts(&scanned).is_empty());
    assert_eq!(scanned.code, "let x = 1; \nlet y = 2;\n");
}

#[test]
fn scan_drops_a_block_comment_containing_a_quoted_literal() {
    let source = format!(
        "/* \"{0}\" */ fn f() {{}}\n/* outer /* \"{0}\" */ inner */ fn g() {{}}\n",
        api_literal("/x")
    );

    let scanned = scan(&source);

    assert!(literal_texts(&scanned).is_empty());
    assert_eq!(scanned.code, " fn f() {}\n fn g() {}\n");
}

#[test]
fn scan_reads_raw_strings_with_any_number_of_hashes() {
    let source = format!(
        "let a = r\"{0}\";\nlet b = r#\"{0}\"#;\nlet c = r##\"say \"#\" {0}\"##;\n",
        api_literal("/x")
    );

    let scanned = scan(&source);

    assert_eq!(
        literal_texts(&scanned),
        vec![
            api_literal("/x"),
            api_literal("/x"),
            format!("say \"#\" {}", api_literal("/x")),
        ]
    );
    assert_eq!(scanned.literals.get(2).map(|literal| literal.line), Some(3));
}

#[test]
fn scan_never_opens_a_string_on_a_char_literal() {
    let source = format!(
        "let q = '\"';\nlet e = '\\'';\nlet s = '\\\\';\nlet r: &'static str = \"{}\";\n",
        api_literal("/x")
    );

    let scanned = scan(&source);

    assert_eq!(literal_texts(&scanned), vec![api_literal("/x")]);
    assert_eq!(
        scanned.literals.first().map(|literal| literal.line),
        Some(4)
    );
}

#[test]
fn scan_keeps_escaped_quotes_inside_a_string() {
    let source = format!(
        "let s = \"say \\\"hi\\\" {}\"; // tail\n",
        api_literal("/x")
    );

    let scanned = scan(&source);

    assert_eq!(
        literal_texts(&scanned),
        vec![format!("say \\\"hi\\\" {}", api_literal("/x"))]
    );
    assert_eq!(scanned.code, "let s = ; \n");
}

#[test]
fn scan_reports_the_line_a_literal_starts_on() {
    let source = format!(
        "fn f() {{\n    /* two\n       lines */\n    let s = \"multi\nline {}\";\n    let t = \"{}\";\n}}\n",
        api_literal("/a"),
        api_literal("/b")
    );

    let scanned = scan(&source);

    let lines: Vec<usize> = scanned
        .literals
        .iter()
        .map(|literal| literal.line)
        .collect();
    assert_eq!(lines, vec![4, 6]);
}

/// `/api` followed by `rest`, built via `format!` so this guard's own source
/// never contains the pattern it forbids (mirrors
/// `schema_qualification_gate.rs`'s own self-pattern trick).
fn api_literal(rest: &str) -> String {
    format!("/{}{rest}", "api")
}

fn literal_texts(scanned: &Scanned) -> Vec<String> {
    scanned
        .literals
        .iter()
        .map(|literal| literal.text.clone())
        .collect()
}

/// Writes `probe_content` as `tests/probe.rs` under a fresh scratch tree,
/// runs the unmarked-literal scan over it, and removes the tree.
fn scan_scratch_probe(name: &str, probe_content: &str) -> Vec<(String, usize)> {
    with_scratch_probe(name, probe_content, files_with_unmarked_literals)
}

/// Like [`scan_scratch_probe`], but runs the full allowlist check against
/// `allowed` and returns its violation messages.
fn probe_violations(name: &str, probe_content: &str, allowed: &[Exemption]) -> Vec<String> {
    with_scratch_probe(name, probe_content, |tests| violations(tests, allowed))
}

fn with_scratch_probe<T>(name: &str, probe_content: &str, run: impl FnOnce(&Path) -> T) -> T {
    let scratch_root = std::env::temp_dir().join(format!(
        "api_path_literal_guard_{name}_{}",
        std::process::id()
    ));
    let scratch_tests = scratch_root.join("tests");
    fs::create_dir_all(&scratch_tests).expect("create scratch tests dir");
    fs::write(scratch_tests.join("probe.rs"), probe_content).expect("write probe file");

    let result = run(&scratch_tests);

    fs::remove_dir_all(&scratch_root).expect("remove scratch tree");

    result
}

/// The allowlist check over every file under `root`: an unlisted file's
/// first matching literal (markers exempt nothing there), a `MarkedSpans`
/// file's first matching literal
/// outside a span, a `WholeFile` entry whose file has no matching literal
/// left, and a `MarkedSpans` entry with no span containing one.
fn violations(root: &Path, allowed: &[Exemption]) -> Vec<String> {
    let files = relative_paths(root, |path| Some(file_literals(path)));
    let mut violations = Vec::new();

    for (file, literals) in &files {
        let exemption = allowed.iter().find(|exemption| exemption.path() == file);

        let hit = match exemption {
            None => literals
                .unmarked
                .iter()
                .chain(&literals.marked)
                .min()
                .map(|line| {
                    format!("{file}:{line}: matching literal in a file with no allowlist entry")
                }),
            Some(Exemption::MarkedSpans { .. }) => literals.unmarked.first().map(|line| {
                format!("{file}:{line}: matching literal outside a marked span (MarkedSpans entry)")
            }),
            Some(Exemption::WholeFile { .. }) => None,
        };

        if let Some(hit) = hit {
            violations.push(hit);
        }
    }

    for exemption in allowed {
        let literals = files
            .iter()
            .find(|(file, _)| file == exemption.path())
            .map(|(_, literals)| literals);
        let stale = match exemption {
            Exemption::WholeFile { .. } => {
                literals.is_none_or(|found| found.unmarked.is_empty() && found.marked.is_empty())
            }
            Exemption::MarkedSpans { .. } => literals.is_none_or(|found| found.marked.is_empty()),
        };

        if stale {
            let missing = match exemption {
                Exemption::WholeFile { .. } => "no matching literal left",
                Exemption::MarkedSpans { .. } => "no marked span contains a matching literal",
            };
            violations.push(format!(
                "{}: stale {} entry ({}), {missing}",
                exemption.path(),
                exemption.variant(),
                exemption.reason()
            ));
        }
    }

    violations
}

/// Returns the sorted set of `(path relative to root, first offending
/// line)` pairs for files under `root` that contain at least one matching
/// literal OUTSIDE a marked span — the set the "unlisted literal" direction
/// checks against `ALLOWED`.
fn files_with_unmarked_literals(root: &Path) -> Vec<(String, usize)> {
    relative_paths(root, first_unmarked_literal_line)
}

/// The lines of one file's matching literals, split by whether each sits
/// inside a `// api-path-guard:off` / `:on` span.
struct FileLiterals {
    unmarked: Vec<usize>,
    marked: Vec<usize>,
}

fn file_literals(path: &Path) -> FileLiterals {
    let content = fs::read_to_string(path).expect("read source file");
    let marked_lines = marked_lines(&content);
    let mut found = FileLiterals {
        unmarked: Vec::new(),
        marked: Vec::new(),
    };

    for literal in &scan(&content).literals {
        if !literal_matches_boundary_pattern(&literal.text) {
            continue;
        }

        if marked_lines.get(literal.line).copied().unwrap_or(false) {
            found.marked.push(literal.line);
        } else {
            found.unmarked.push(literal.line);
        }
    }

    found
}

fn relative_paths<T>(root: &Path, probe: impl Fn(&Path) -> Option<T>) -> Vec<(String, T)> {
    let mut files: Vec<(String, T)> = rust_source_files(root)
        .into_iter()
        .filter_map(|path| {
            let found = probe(&path)?;
            let relative = path
                .strip_prefix(root)
                .expect("file is under the scanned root")
                .to_string_lossy()
                .replace('\\', "/");

            Some((relative, found))
        })
        .collect();

    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// The line of the first matching literal outside a
/// `// api-path-guard:off` / `:on` span, if any.
fn first_unmarked_literal_line(path: &Path) -> Option<usize> {
    file_literals(path).unmarked.first().copied()
}

/// One flag per 1-based line number (index 0 unused): `true` when the line
/// sits inside a `// api-path-guard:off` / `:on` span. Marker lines are
/// comments, so the tokenizer never sees them; they are matched on the raw
/// line text here instead.
fn marked_lines(content: &str) -> Vec<bool> {
    let mut marked = vec![false];
    let mut marker_depth: u32 = 0;

    for line in content.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with(MARKER_OFF) {
            marker_depth += 1;
        } else if trimmed.starts_with(MARKER_ON) {
            marker_depth = marker_depth.saturating_sub(1);
        }

        marked.push(marker_depth > 0);
    }

    // A literal can start on the trailing line when the file has no final
    // newline, and `str::lines` never yields an empty trailing line.
    marked.push(false);
    marked
}

/// The boundary rule over one string literal's contents: a hit is any
/// `/api` preceded by the start of the literal, `}`, `/`, or an ASCII
/// alphanumeric (the end of a literal host such as `localhost:3000`), and
/// followed by `/`, the end of the literal, or `?`. `/api-keys` never
/// matches (the following character is `-`); `"GET /api/x failed"` never
/// matches (the preceding character is a space), and neither do `(`, `"`,
/// or any other punctuation before `/api`.
fn literal_matches_boundary_pattern(text: &str) -> bool {
    let needle = api_literal("");
    let bytes = text.as_bytes();

    text.match_indices(needle.as_str()).any(|(start, _)| {
        let preceded = match start.checked_sub(1).and_then(|index| bytes.get(index)) {
            None => true,
            Some(previous) => matches!(previous, b'}' | b'/') || previous.is_ascii_alphanumeric(),
        };
        let followed = matches!(
            bytes.get(start + needle.len()),
            None | Some(b'/') | Some(b'?')
        );

        preceded && followed
    })
}

/// A string literal's contents (without its delimiters) and the 1-based
/// line its opening delimiter sits on.
struct Literal {
    line: usize,
    text: String,
}

/// The parts of a Rust source file the guards care about: every string
/// literal, and the remaining code text with comments and literals removed.
struct Scanned {
    literals: Vec<Literal>,
    code: String,
}

/// Tokenizes `content` far enough to separate string literals from code
/// and comments: `//` line comments (including `///` and `//!`) and nested
/// `/* … */` block comments are dropped; normal `"…"` literals honour
/// backslash escapes; raw `r"…"`, `r#"…"#` (any number of hashes) literals
/// end only at their matching delimiter; `'"'`, `'\''`, and other char
/// literals never open a string, while lifetimes and labels pass through as
/// code. Everything else is code.
fn scan(content: &str) -> Scanned {
    let mut scanner = Scanner::new(content);
    let mut literals = Vec::new();
    let mut code = String::new();

    while let Some(current) = scanner.peek(0) {
        match current {
            '/' if scanner.peek(1) == Some('/') => scanner.skip_line_comment(),
            '/' if scanner.peek(1) == Some('*') => scanner.skip_block_comment(),
            '"' => {
                let line = scanner.line;
                let text = scanner.read_string();
                literals.push(Literal { line, text });
            }
            'r' => match scanner.raw_string_hashes() {
                Some(hashes) => {
                    let line = scanner.line;
                    let text = scanner.read_raw_string(hashes);
                    literals.push(Literal { line, text });
                }
                None => code.push(scanner.bump()),
            },
            '\'' => {
                let len = scanner.char_literal_len().unwrap_or(1);
                for _ in 0..len {
                    code.push(scanner.bump());
                }
            }
            _ => code.push(scanner.bump()),
        }
    }

    Scanned { literals, code }
}

struct Scanner {
    chars: Vec<char>,
    pos: usize,
    line: usize,
}

impl Scanner {
    fn new(content: &str) -> Self {
        Self {
            chars: content.chars().collect(),
            pos: 0,
            line: 1,
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    /// Consumes one char, tracking line numbers.
    ///
    /// # Panics
    /// Panics at end of input; callers only bump after a successful `peek`.
    fn bump(&mut self) -> char {
        let current = self.peek(0).expect("bump past end of input");
        self.pos += 1;

        if current == '\n' {
            self.line += 1;
        }

        current
    }

    fn skip_line_comment(&mut self) {
        while self.peek(0).is_some_and(|current| current != '\n') {
            self.bump();
        }
    }

    /// Positioned on `/*`; consumes through the matching `*/`, honouring
    /// Rust's nested block comments. An unterminated comment runs to end of
    /// input, as it would for `rustc`.
    fn skip_block_comment(&mut self) {
        self.bump();
        self.bump();
        let mut depth = 1;

        while depth > 0 {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), Some('*')) => {
                    self.bump();
                    self.bump();
                    depth += 1;
                }
                (Some('*'), Some('/')) => {
                    self.bump();
                    self.bump();
                    depth -= 1;
                }
                (Some(_), _) => {
                    self.bump();
                }
                (None, _) => break,
            }
        }
    }

    /// Positioned on the opening `"`; returns the contents up to the
    /// closing unescaped `"`, keeping escape sequences verbatim.
    fn read_string(&mut self) -> String {
        self.bump();
        let mut text = String::new();

        while let Some(current) = self.peek(0) {
            self.bump();

            match current {
                '"' => break,
                '\\' => {
                    text.push('\\');
                    if self.peek(0).is_some() {
                        text.push(self.bump());
                    }
                }
                _ => text.push(current),
            }
        }

        text
    }

    /// Positioned on `r`: `Some(n)` when `r` + `n` hashes + `"` follows,
    /// i.e. a raw string opens here.
    fn raw_string_hashes(&self) -> Option<usize> {
        let mut hashes = 0;

        loop {
            match self.peek(1 + hashes) {
                Some('#') => hashes += 1,
                Some('"') => return Some(hashes),
                _ => return None,
            }
        }
    }

    /// Positioned on `r`; returns the contents up to `"` followed by
    /// `hashes` hashes.
    fn read_raw_string(&mut self, hashes: usize) -> String {
        for _ in 0..hashes + 2 {
            self.bump();
        }
        let mut text = String::new();

        while let Some(current) = self.peek(0) {
            self.bump();

            let closes = current == '"' && (0..hashes).all(|offset| self.peek(offset) == Some('#'));
            if closes {
                for _ in 0..hashes {
                    self.bump();
                }
                break;
            }

            text.push(current);
        }

        text
    }

    /// Positioned on `'`: the length of the char literal starting here
    /// (`'x'`, `'"'`, `'\''`, `'\u{41}'`), or `None` for a lifetime or
    /// label such as `'static` or `'outer:`.
    fn char_literal_len(&self) -> Option<usize> {
        match (self.peek(1), self.peek(2)) {
            (Some('\\'), _) => {
                let mut offset = 3;

                while let Some(current) = self.peek(offset) {
                    offset += 1;

                    match current {
                        '\'' => return Some(offset),
                        '\n' => return None,
                        _ => {}
                    }
                }

                None
            }
            (Some(_), Some('\'')) => Some(3),
            _ => None,
        }
    }
}

/// The files permitted to call `router_audit::namespaces_for` or
/// `router_audit::mounted_path` directly, outside `support/path.rs` and
/// `support/route_matrix.rs` (`v2-e3-s5` design D5, spec "one seam, not
/// two", T1.33's amended acceptance gate 7): the six dual-mount sweeps that
/// legitimately exercise both mounts by intent, the three per-component
/// router-parity tests (same reason), `api_v1_path_presence_guard.rs`,
/// whose `mounted_path(V1_NAMESPACE, ...)` call reads D9's frozen V1
/// baseline through the namespace constant directly, never through the
/// suite's flippable default, and `openapi_idempotency_annotations.rs`,
/// pinned the same way to the composed document's own `/api` keys until
/// S6/PR1 re-keys it.
const SEAM_EXCEPTIONS: &[&str] = &[
    "api_401_sweep.rs",
    "api_rfc9457_sweep.rs",
    "api_page_conformance.rs",
    "api_capability_sweep.rs",
    "api_v1_v2_byte_identical.rs",
    "api_router_mount_assertion.rs",
    "api_platform_router_parity.rs",
    "api_custos_router_parity.rs",
    "api_acta_router_parity.rs",
    "api_v1_path_presence_guard.rs",
    "openapi_idempotency_annotations.rs",
];

/// INV-SEAM (spec "one seam, not two", T1.29/T1.33): outside
/// `support/path.rs`/`support/route_matrix.rs` and the named
/// [`SEAM_EXCEPTIONS`], no file under `crates/atlas_server/tests` calls
/// `router_audit::namespaces_for` or `router_audit::mounted_path` directly.
#[test]
fn only_the_named_files_call_namespaces_for_or_mounted_path_directly() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let tests_dir = Path::new(manifest_dir).join("tests");

    let offenders: Vec<String> = rust_source_files(&tests_dir)
        .into_iter()
        .filter(|path| {
            let relative = path
                .strip_prefix(&tests_dir)
                .expect("file is under the scanned root")
                .to_string_lossy()
                .replace('\\', "/");

            relative != "support/path.rs"
                && relative != "support/route_matrix.rs"
                && !SEAM_EXCEPTIONS.contains(&relative.as_str())
                && file_calls_seam_primitive_directly(path)
        })
        .map(|path| {
            path.strip_prefix(&tests_dir)
                .expect("file is under the scanned root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "found a direct namespaces_for/mounted_path call outside the seam in: {}",
        offenders.join(", ")
    );
}

fn file_calls_seam_primitive_directly(path: &Path) -> bool {
    let content = fs::read_to_string(path).expect("read source file");
    let namespaces_for_call = format!("{}(", "namespaces_for");
    let mounted_path_call = format!("{}(", "mounted_path");

    let code = scan(&content).code;

    code.contains(&namespaces_for_call) || code.contains(&mounted_path_call)
}

fn rust_source_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut queue = VecDeque::from([dir.to_path_buf()]);

    while let Some(current) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();

            if path.is_dir() {
                queue.push_back(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    files
}
