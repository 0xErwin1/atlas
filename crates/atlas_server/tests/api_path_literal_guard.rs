#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! CI grep gate (`v2-e3-s5` design D3, collapsed to a single repo-wide gate
//! by `v2-e3-s7` design D3): a source-tree-walking `#[test]` proving that no
//! production or documentation file anywhere in the repository builds a
//! request path against the retired `/api` mount instead of `/api/v2/<component>`.
//!
//! **Scope**: the whole repository (`Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")`),
//! superseding the four separate scan-root tests `v2-e3-s5`/`v2-e3-s6`
//! introduced (`crates/atlas_server/tests/**`, `atlas_client/src`,
//! `atlas_mcp/src`, `atlas_cli/src`), folded into this one walk. `apps/web/src/**`
//! stays OUT of this gate: S6/D4.5 already owns it with its own Vitest guard
//! (`apps/web/src/__tests__/v1PathLiteralGuard.test.ts`), which needs
//! `openapi.json`'s keys and runs in the web CI job.
//!
//! **Extraction modes** (D3.2, the tree is not all Rust):
//! - **Rust-tokenized** — `**/*.rs` under `crates/` and
//!   `apps/desktop/src-tauri/{src,tests}`: reuses [`scan`]'s tokenizer, so
//!   only the contents of string literals are candidates; comments are
//!   skipped exactly as before.
//! - **Plain text** — `**/*.{md,nix,yml,yaml,toml,conf,template}`: the
//!   boundary rule applied to each raw line. These formats have no string
//!   literal or comment notion the tokenizer understands, and a markdown
//!   path reference is conventionally backtick- or pipe-delimited rather
//!   than quote-delimited, so the "preceded by" half of the boundary rule
//!   is widened to accept any predecessor (or start of line) — only the
//!   "followed by" half and the `/v2/` exclusion (below) still apply. See
//!   [`plain_text_matches_boundary_pattern`].
//! - Excluded from the walk by path (not by allowlist entry, because an
//!   allowlist entry on a regenerable artifact would be staleness-checked
//!   against a build output): `target/`, `node_modules/`, `.git/`, `dist/`,
//!   `apps/web/openapi.json`, `apps/web/src/api/types.d.ts`,
//!   `apps/desktop/src-tauri/gen/`, `pnpm-lock.yaml`, `Cargo.lock`, and all
//!   of `apps/web/src/**`.
//!
//! **Match rule**: inside a Rust string literal (normal `"…"` or raw
//! `r"…"`/`r#"…"#`; `//`, `///`, `//!`, and nested `/* … */` comment text is
//! skipped, and a `//` inside a literal is literal text, never a comment) or
//! a plain-text line, a hit is any `/api` followed by `/`, the end of the
//! text, or `?`, EXCLUDING a hit whose following text is exactly `/v2/`
//! (`v2-e3-s7` D3.1: a migrated `/api/v2/acta/…` literal is never a hit; a
//! bare `/api/v2` with nothing after it stays a hit, since that flat form is
//! never mounted either). In Rust-tokenized mode, the preceding character
//! must additionally be the start of the literal, `}`, `/`, or an ASCII
//! alphanumeric — `/api-keys` (a real custos registry path) never matches
//! because the following character is neither of the three; prose such as
//! `"GET /api/x failed"` never matches because the preceding character is a
//! space.
//!
//! **Allowlist**: `ALLOWED` names every file permitted to contain a matching
//! literal, with a stated reason and a class (`permanent` or `historical`),
//! as a typed [`Exemption`] checked in both directions in the same test run.
//! `WholeFile` exempts every matching literal in the file and is stale
//! (FAILS) when the file has none left. `MarkedSpans` exempts only literals
//! inside a `// api-path-guard:off` / `// api-path-guard:on` span: a
//! matching literal outside a span FAILS naming `file:line`, and the entry
//! is stale (FAILS) unless at least one span contains a matching literal. An
//! unlisted file with a matching literal FAILS, inside or outside a marker
//! span. The allowlist admits no `TRANSIENT` entries: `v2-e3-s5`/PR2–PR3 and
//! `v2-e3-s6`/PR4 landed before this gate opened, so every remaining
//! reference to `/api` outside `/api/v2` is either a permanent structural
//! fact (a negative probe, a storage-key constant, a security check on the
//! shared prefix) or a historical decision record.
//!
//! **CI**: no workflow edit needed (D3.5) — `style.yml`'s `source-gates` job
//! already runs `cargo test -p atlas_server --test api_path_literal_guard`,
//! and the collapsed gate is the same binary.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// How one allowlisted file is exempt from the unlisted-literal check.
enum Exemption {
    /// Every matching literal in `path` is exempt; stale once none is left.
    WholeFile {
        path: &'static str,
        class: Class,
        reason: &'static str,
    },
    /// Only literals inside a `// api-path-guard:off`/`:on` span of `path`
    /// are exempt; one outside a span is a hit, and the entry is stale unless
    /// at least one span contains a matching literal.
    MarkedSpans {
        path: &'static str,
        class: Class,
        reason: &'static str,
    },
}

/// Whether an allowlist entry is a structural fact this gate expects to hold
/// forever, or a decision record that describes what was true at the time it
/// was written and is never rewritten (`v2-e3-s7` D3.3). No entry is
/// `TRANSIENT`: this gate opens only once every migration it depends on has
/// landed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Permanent,
    Historical,
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

    fn class(&self) -> Class {
        match self {
            Self::WholeFile { class, .. } | Self::MarkedSpans { class, .. } => *class,
        }
    }
}

const fn whole_file(path: &'static str, class: Class, reason: &'static str) -> Exemption {
    Exemption::WholeFile {
        path,
        class,
        reason,
    }
}

const fn marked_spans(path: &'static str, class: Class, reason: &'static str) -> Exemption {
    Exemption::MarkedSpans {
        path,
        class,
        reason,
    }
}

/// Every file permitted to contain a matching `/api` literal, and why,
/// repo-relative (`v2-e3-s7` D3.3). Eleven `permanent` entries — a
/// structural fact each — and two `historical` decision records, dated by
/// the slice that wrote them; the tables they hold are not rewritten to V2
/// form (rewriting them would falsify what was true when they were
/// written), so each carries a dated note instead where one is warranted.
const ALLOWED: &[Exemption] = &[
    whole_file(
        "crates/atlas_server/tests/support/path.rs",
        Class::Permanent,
        "canonical_store_path, the one test-side reader of the idempotency store's frozen \
         key prefix",
    ),
    whole_file(
        "crates/atlas_server/src/middleware/idempotency.rs",
        Class::Permanent,
        "IDEMPOTENCY_STORE_PATH_PREFIX — a store-key prefix, not a mount (D2) — plus its own \
         test module's synthetic /api/... fixture literals",
    ),
    whole_file(
        "crates/atlas_server/src/router_audit.rs",
        Class::Permanent,
        "V2_PREFIX's own literal (\"/api/v2\") is the one place the V2 prefix is built; a bare \
         /api/v2 with nothing after it is intentionally still a hit under the boundary rule, \
         so this is the seam's own construction site, not a stray reference",
    ),
    marked_spans(
        "crates/atlas_client/src/lib.rs",
        Class::Permanent,
        "the seam's own V2_PREFIX construction (AtlasClient::mounted)",
    ),
    marked_spans(
        "apps/desktop/src-tauri/src/lib.rs",
        Class::Permanent,
        "validate_api_path/build_authenticated_request's starts_with(\"/api/\") allowlist — \
         a security check on the shared prefix, satisfied by /api/v2/... with no code change",
    ),
    whole_file(
        "apps/desktop/src-tauri/tests/host_contract.rs",
        Class::Permanent,
        "traversal negative probes (/api/../admin, /api/%2e%2e/admin, etc.) exercising \
         validate_api_path's rejection of a malformed path, independent of which mount is \
         live, plus one absolute-URL rejection literal",
    ),
    whole_file(
        "deploy/nginx.conf.template",
        Class::Permanent,
        "location /api/ is a prefix match covering /api/v2/...; narrowing it gains nothing",
    ),
    whole_file(
        "crates/atlas_server/tests/api_router_mount_assertion.rs",
        Class::Permanent,
        "the negative probes: /apiv2, /api/v3, bare /api/v2/<rel>, wrong-component, and \
         (since v2-e3-s7) /api itself — paths the codebase must never produce",
    ),
    whole_file(
        "crates/atlas_server/tests/api_unmatched_path_fallback.rs",
        Class::Permanent,
        "negative probe: a nonexistent path must never resolve",
    ),
    whole_file(
        "crates/atlas_server/tests/idempotency_middleware.rs",
        Class::Permanent,
        "40 synthetic mock-route literals on a test-local router, plus the canonical \
         idempotency store-key literals",
    ),
    whole_file(
        "crates/atlas_server/tests/idempotency_repo.rs",
        Class::Permanent,
        "a canonical store-key row, not a request path",
    ),
    whole_file(
        "docs/registry-route-ownership.md",
        Class::Historical,
        "the v2-e3-s2 decision record; its route table documents RouteDeclaration.path as it \
         was then (absolute V1 form). Rewriting the rows would falsify the record — see the \
         dated note recording the v2-e3-s7 retirement instead",
    ),
    whole_file(
        "docs/reg5-idempotent-judgment.md",
        Class::Historical,
        "the v2-e3-s3 idempotency judgment record, same class as registry-route-ownership.md",
    ),
];

const MARKER_OFF: &str = "// api-path-guard:off";
const MARKER_ON: &str = "// api-path-guard:on";

/// Repo root: `crates/atlas_server` is two levels below it.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn no_unallowlisted_api_literal_exists_anywhere_in_the_repository() {
    let violations = repo_violations(&repo_root(), ALLOWED);

    assert!(
        violations.is_empty(),
        "repo-wide api-path guard violations:\n{}",
        violations.join("\n")
    );
}

/// D3.3: the allowlist admits no `TRANSIENT` entries — every entry is either
/// `permanent` (a structural fact) or `historical` (a dated decision
/// record). This is a static property of `ALLOWED`, checked once rather than
/// trusted by convention.
#[test]
fn the_allowlist_has_exactly_two_historical_entries_and_the_rest_permanent() {
    let historical = ALLOWED
        .iter()
        .filter(|entry| entry.class() == Class::Historical)
        .count();

    assert_eq!(
        historical, 2,
        "exactly two allowlist entries (the two dated decision records) must be historical; \
         every other entry must be permanent"
    );
}

/// A hypothetical new file anywhere in the repository containing a
/// `/api/foo` literal is flagged, naming the file and line (spec: "A new
/// V1-form literal anywhere in the repository fails the gate").
#[test]
fn a_new_literal_anywhere_in_the_repository_fails_the_gate() {
    let probe_content = format!(
        "fn regression(base: &str) -> String {{\n    format!(\"{{base}}{}\")\n}}\n",
        api_literal("/foo")
    );

    let literal_files = scan_scratch_probe("repo_regression", &probe_content);

    assert_eq!(
        literal_files,
        vec![("probe.rs".to_string(), 2)],
        "expected the gate to flag the reintroduced literal"
    );
}

/// A stale allowlist entry — naming a file with no remaining matching
/// literal — fails, mirroring the existing bidirectional discipline (spec:
/// "A stale allowlist entry fails the gate").
#[test]
fn a_stale_allowlist_entry_fails_the_gate() {
    let probe_content = "fn clean() -> &'static str {\n    \"/clean\"\n}\n".to_string();
    let allowed = [marked_spans(
        "probe.rs",
        Class::Permanent,
        "no longer needed",
    )];

    let violations = probe_violations("stale_allowlist", &probe_content, &allowed);

    assert_eq!(
        violations,
        vec![
            "probe.rs: stale MarkedSpans entry (no longer needed), no marked span contains a matching literal"
        ]
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
    let allowed = [marked_spans("probe.rs", Class::Permanent, "probe")];

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
    let allowed = [marked_spans("probe.rs", Class::Permanent, "probe")];

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

/// `v2-e3-s7` D3.1: a migrated `/api/v2/<component>/...` literal is never a
/// hit under either boundary rule, in either mode.
#[test]
fn a_v2_prefixed_literal_is_never_a_hit() {
    assert!(!literal_matches_boundary_pattern(&format!(
        "{}v2/acta/x",
        api_literal("/")
    )));
    assert!(!plain_text_matches_boundary_pattern(&format!(
        "see `{}v2/acta/x` for the route",
        api_literal("/")
    )));
}

/// `v2-e3-s7` D3.1: a bare `/api/v2` with nothing after it stays a hit —
/// that flat form is never mounted either (`flat_and_wrong_component_v2_
/// forms_never_match_any_declared_route`).
#[test]
fn a_bare_api_v2_with_nothing_after_it_stays_a_hit() {
    assert!(literal_matches_boundary_pattern(&format!(
        "{}v2",
        api_literal("/")
    )));
    assert!(plain_text_matches_boundary_pattern(&format!(
        "see `{}v2` for the flat form",
        api_literal("/")
    )));
}

/// `v2-e3-s7` D3.2: the plain-text boundary rule's widened predecessor set
/// catches a backtick- or pipe-delimited markdown reference the strict
/// (Rust-literal) rule would miss.
#[test]
fn the_plain_text_rule_accepts_a_backtick_delimited_markdown_reference() {
    let line = format!("| GET | `{}` | Something |", api_literal("/me/ui-state"));

    assert!(plain_text_matches_boundary_pattern(&line));
    assert!(!literal_matches_boundary_pattern(&line));
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

/// The allowlist check over every Rust file under `root` (used by the probe
/// self-tests above, which write a scratch `tests/probe.rs` tree): an
/// unlisted file's first matching literal (markers exempt nothing there), a
/// `MarkedSpans` file's first matching literal outside a span, a `WholeFile`
/// entry whose file has no matching literal left, and a `MarkedSpans` entry
/// with no span containing one.
fn violations(root: &Path, allowed: &[Exemption]) -> Vec<String> {
    violations_from_pairs(
        relative_paths(root, |path| Some(file_literals(path))),
        allowed,
    )
}

/// The repo-wide walk (`v2-e3-s7` D3): every candidate file under `root`,
/// dispatched to the Rust-tokenized or plain-text extraction mode by path
/// (D3.2), excluded paths skipped entirely (D3.2's exclusion list).
fn repo_violations(root: &Path, allowed: &[Exemption]) -> Vec<String> {
    violations_from_pairs(repo_relative_paths(root), allowed)
}

/// The bidirectional allowlist check shared by [`violations`] and
/// [`repo_violations`]: an unlisted file's first matching literal (markers
/// exempt nothing there) fails; a `MarkedSpans` file's first matching
/// literal outside a span fails; a `WholeFile` entry whose file has no
/// matching literal left is stale; a `MarkedSpans` entry with no span
/// containing one is stale.
fn violations_from_pairs(files: Vec<(String, FileLiterals)>, allowed: &[Exemption]) -> Vec<String> {
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
    literals_in_content(&content)
}

/// Like [`file_literals`], but truncates the content at a top-level
/// `#[cfg(test)]` marker before scanning, so a `src/` file's own unit-test
/// module — routinely carrying a synthetic fixture route or a "no V1 key
/// remains" negative assertion — is checked as production code only
/// (`v2-e3-s6` D2.4, extended repo-wide by `v2-e3-s7` D3.2).
fn production_file_literals(path: &Path) -> FileLiterals {
    let content = fs::read_to_string(path).expect("read source file");
    let production = content
        .find("#[cfg(test)]")
        .map_or(content.as_str(), |index| &content[..index]);
    literals_in_content(production)
}

fn literals_in_content(content: &str) -> FileLiterals {
    let marked_lines = marked_lines(content);
    let mut found = FileLiterals {
        unmarked: Vec::new(),
        marked: Vec::new(),
    };

    for literal in &scan(content).literals {
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

/// Plain-text extraction (D3.2): applies [`plain_text_matches_boundary_pattern`]
/// to each raw line, with the same marker-span support as Rust-tokenized
/// mode (the marker text is matched on the raw line regardless of the
/// file's own comment syntax, exactly like [`marked_lines`] already does for
/// Rust).
fn plain_text_file_literals(path: &Path) -> FileLiterals {
    let content = fs::read_to_string(path).expect("read source file");
    let marked_lines = marked_lines(&content);
    let mut found = FileLiterals {
        unmarked: Vec::new(),
        marked: Vec::new(),
    };

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        if !plain_text_matches_boundary_pattern(line) {
            continue;
        }

        if marked_lines.get(line_number).copied().unwrap_or(false) {
            found.marked.push(line_number);
        } else {
            found.unmarked.push(line_number);
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
/// comments in Rust, so the tokenizer never sees them; here they are matched
/// on the raw line text directly, which also lets a non-Rust file (D3.2)
/// carry the same marker text as an ordinary line, regardless of that
/// format's own comment syntax.
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
/// followed by `/`, the end of the literal, or `?`, excluding a hit whose
/// following text is exactly `/v2/` (`v2-e3-s7` D3.1). `/api-keys` never
/// matches (the following character is `-`); `"GET /api/x failed"` never
/// matches (the preceding character is a space), and neither do `(`, `"`,
/// or any other punctuation before `/api`.
fn literal_matches_boundary_pattern(text: &str) -> bool {
    boundary_hits(text, false)
}

/// Like [`literal_matches_boundary_pattern`], but for a raw plain-text line
/// (markdown, YAML, Nix, …) rather than the contents of a Rust string
/// literal (`v2-e3-s7` D3.2). Widens the "preceded by" half of the rule to
/// accept ANY predecessor (or start of line): plain text has no
/// literal-delimiter concept, and a route reference is conventionally
/// backtick- or pipe-delimited (`` `/api/workspaces/{ws}/tasks` ``, `` | GET
/// | `/api/me/ui-state` | `` ) rather than quote-delimited, so the narrower
/// Rust-literal predecessor set would silently miss every real documentation
/// reference. The "followed by" half and the `/v2/` exclusion are unchanged.
fn plain_text_matches_boundary_pattern(text: &str) -> bool {
    boundary_hits(text, true)
}

/// Shared boundary-rule engine for both extraction modes. `wide_preceded`
/// selects the plain-text mode's widened predecessor rule; the "followed
/// by" half and the `/v2/` exclusion are identical in both modes.
///
/// Plain-text mode's widened predecessor set (see
/// [`plain_text_matches_boundary_pattern`]) has one narrow exclusion of its
/// own: `src/api/` is this repository's own idiom for a TypeScript source
/// directory (`apps/web/src/api/types.d.ts`, referenced throughout the
/// docs), never a request path, and the widened rule would otherwise flag
/// every mention of it. The Rust-literal rule needs no such exclusion: a
/// Rust string literal never spells a filesystem path with this shape as
/// prose in the way markdown routinely does.
fn boundary_hits(text: &str, wide_preceded: bool) -> bool {
    let needle = api_literal("");
    let bytes = text.as_bytes();

    text.match_indices(needle.as_str()).any(|(start, _)| {
        let preceded = if wide_preceded {
            bytes.get(start.saturating_sub(3)..start) != Some(b"src")
        } else {
            match start.checked_sub(1).and_then(|index| bytes.get(index)) {
                None => true,
                Some(previous) => {
                    matches!(previous, b'}' | b'/') || previous.is_ascii_alphanumeric()
                }
            }
        };

        let after = start + needle.len();
        let followed = matches!(bytes.get(after), None | Some(b'/') | Some(b'?'));
        let is_v2_prefixed = bytes.get(after..after + 4) == Some(b"/v2/");

        preceded && followed && !is_v2_prefixed
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

/// Directory names excluded from the repo-wide walk wherever they occur
/// (`v2-e3-s7` D3.2): build output, dependency trees, and VCS metadata.
const EXCLUDED_DIR_SEGMENTS: &[&str] = &["target", "node_modules", ".git", "dist"];

/// Individual repo-relative files excluded from the walk (D3.2): generated
/// artifacts an allowlist entry would incorrectly staleness-check against a
/// build output.
const EXCLUDED_FILES: &[&str] = &[
    "apps/web/openapi.json",
    // Split across two literal fragments (concat!, evaluated at compile
    // time into one string) so this guard's own source never contains the
    // pattern it forbids — the same self-pattern trick `api_literal` uses
    // elsewhere in this file.
    concat!("apps/web/src/ap", "i/types.d.ts"),
    "pnpm-lock.yaml",
    "Cargo.lock",
];

/// Repo-relative path prefixes excluded wholesale (D3.2): `apps/web/src/**`
/// stays owned by S6's own Vitest guard, and the desktop app's generated
/// Tauri bindings are build output like `target/`.
const EXCLUDED_PATH_PREFIXES: &[&str] = &["apps/desktop/src-tauri/gen/", "apps/web/src/"];

fn is_excluded(relative: &str) -> bool {
    if EXCLUDED_FILES.contains(&relative) {
        return true;
    }

    if EXCLUDED_PATH_PREFIXES
        .iter()
        .any(|prefix| relative.starts_with(prefix))
    {
        return true;
    }

    relative
        .split('/')
        .any(|segment| EXCLUDED_DIR_SEGMENTS.contains(&segment))
}

/// D3.2's extraction-mode dispatch: which scan a repo-relative path gets, or
/// `None` when the path is out of this gate's scope entirely (every other
/// file extension, plus everything `is_excluded` already dropped).
///
/// `RustProduction` truncates at a file's first top-level `#[cfg(test)]`
/// before scanning (mirroring `production_file_literals`, `v2-e3-s6` D2.4):
/// a `src/` file's own unit-test module routinely carries synthetic or
/// negative-assertion literals (a fixture route, a "no V1 key remains"
/// check) that would otherwise read as a false-positive V1 dependency.
/// `RustTokenized` (integration test files under a `tests/` directory) scans
/// the whole file: there the test code IS the file's entire content, so
/// nothing is truncated.
enum ExtractionMode {
    RustProduction,
    RustTokenized,
    PlainText,
}

fn extraction_mode_for(relative: &str) -> Option<ExtractionMode> {
    if relative.ends_with(".rs") {
        let is_test_dir = relative.contains("/tests/");
        let is_src_dir = relative.contains("/src/");
        let in_rust_scan_root = relative.starts_with("crates/")
            || relative.starts_with("apps/desktop/src-tauri/src/")
            || relative.starts_with("apps/desktop/src-tauri/tests/");

        if in_rust_scan_root && is_test_dir {
            return Some(ExtractionMode::RustTokenized);
        }
        if in_rust_scan_root && is_src_dir {
            return Some(ExtractionMode::RustProduction);
        }
    }

    const PLAIN_TEXT_EXTENSIONS: &[&str] = &[
        ".md",
        ".nix",
        ".yml",
        ".yaml",
        ".toml",
        ".conf",
        ".template",
    ];
    if PLAIN_TEXT_EXTENSIONS
        .iter()
        .any(|extension| relative.ends_with(extension))
    {
        return Some(ExtractionMode::PlainText);
    }

    None
}

/// Every non-excluded, non-directory file under `root`.
fn candidate_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut queue = VecDeque::from([root.to_path_buf()]);

    while let Some(current) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("file is under the scanned root")
                .to_string_lossy()
                .replace('\\', "/");

            if is_excluded(&relative) {
                continue;
            }

            if path.is_dir() {
                queue.push_back(path);
            } else {
                files.push(path);
            }
        }
    }

    files
}

/// `(repo-relative path, extracted literals)` for every candidate file under
/// `root`, dispatched by [`extraction_mode_for`].
fn repo_relative_paths(root: &Path) -> Vec<(String, FileLiterals)> {
    let mut files: Vec<(String, FileLiterals)> = candidate_files(root)
        .into_iter()
        .filter_map(|path| {
            let relative = path
                .strip_prefix(root)
                .expect("file is under the scanned root")
                .to_string_lossy()
                .replace('\\', "/");

            let literals = match extraction_mode_for(&relative)? {
                ExtractionMode::RustTokenized => file_literals(&path),
                ExtractionMode::RustProduction => production_file_literals(&path),
                ExtractionMode::PlainText => plain_text_file_literals(&path),
            };

            Some((relative, literals))
        })
        .collect();

    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// The files permitted to call `router_audit::mounted_path` directly,
/// outside `support/path.rs` and `support/route_matrix.rs` (`v2-e3-s5`
/// design D5, spec "one seam, not two", `v2-e3-s7` D1/U2: pruned of the
/// files this slice deleted, and of `namespaces_for`, which no longer
/// exists anywhere in the codebase): the sweeps and per-component
/// router-parity tests that legitimately build a request path at a specific
/// component's own mount, by intent.
const SEAM_EXCEPTIONS: &[&str] = &[
    "api_401_sweep.rs",
    "api_rfc9457_sweep.rs",
    "api_router_mount_assertion.rs",
    "api_platform_router_parity.rs",
    "api_custos_router_parity.rs",
    "api_acta_router_parity.rs",
];

/// INV-SEAM (spec "one seam, not two", T1.29/T1.33): outside
/// `support/path.rs`/`support/route_matrix.rs` and the named
/// [`SEAM_EXCEPTIONS`], no file under `crates/atlas_server/tests` calls
/// `router_audit::mounted_path` directly.
#[test]
fn only_the_named_files_call_mounted_path_directly() {
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
        "found a direct mounted_path call outside the seam in: {}",
        offenders.join(", ")
    );
}

fn file_calls_seam_primitive_directly(path: &Path) -> bool {
    let content = fs::read_to_string(path).expect("read source file");
    let mounted_path_call = format!("{}(", "mounted_path");

    let code = scan(&content).code;

    code.contains(&mounted_path_call)
}
