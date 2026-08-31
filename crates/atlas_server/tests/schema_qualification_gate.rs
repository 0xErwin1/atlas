#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! CI grep gate (design §S3d, generalized by S4 PR11 §D5).
//!
//! Runs in the opposite direction from
//! `atlas_custos_postgres/tests/dependency_boundary.rs::no_acta_table_names_in_sql`:
//! that test keeps Acta table names out of the Custos-owned crate; this one
//! keeps every unqualified reference to an already-moved table out of the
//! rest of the workspace, for every schema a `SET SCHEMA` migration has
//! moved a table into so far. The sanctioned form is schema-qualified SQL
//! (`<schema>.<table>`) — that qualification is what lets production code
//! and the owning crate itself keep talking to these tables straight
//! through raw SQL and sea-orm entities, without a new composition API.
//!
//! `TABLE_SCHEMA` starts with the eight Custos tables
//! (`m20260830_000051_custos_set_schema`) and grows by two more
//! (`workspaces`, `workspace_memberships`) as of S4 PR11's
//! `m20260901_000053_acta_identity_workspaces_set_schema` — the first of
//! five Acta `SET SCHEMA` batches (design §D1) — and by ten more
//! (`property_definitions`, `projects`, `folders`, `documents`,
//! `document_revisions`, `document_links`, `attachments`,
//! `attachment_write_intents`, `comment_attachment_drafts`,
//! `comment_attachment_draft_uploads`) as of S4 PR12's
//! `m20260902_000054_acta_documents_set_schema`, batch 2, and by nine more
//! (`boards`, `board_columns`, `tasks`, `task_references`, `task_assignees`,
//! `task_checklist_items`, `task_activity`, `workspace_status_templates`,
//! `platform_status_templates`) as of S4 PR13's
//! `m20260903_000055_acta_boards_tasks_set_schema`, batch 3, and by eleven
//! more (`comments`, `comment_links`, `comment_link_events`, `tags`,
//! `events_outbox`, `webhook_subscriptions`, `webhook_delivery_log`,
//! `automation_rules`, `integration_configs`, `saved_searches`,
//! `task_views`) as of S4 PR14's
//! `m20260904_000056_acta_comments_events_tags_set_schema`, batch 4. Each
//! later batch (PR15) adds its own tables to this map as it lands; this is a
//! single shared table→schema map rather than one gate per product (design
//! §D5's rejected-alternative rationale: the tree-walk/exclusion logic is
//! schema-agnostic, so a second copy would only duplicate it).
//!
//! Excluded from the scan: `crates/migration` (the frozen historical block,
//! D5), `crates/atlas_custos_postgres/src/migrations` (Custos-owned
//! migrations, including its own `SET SCHEMA` migration), and
//! `crates/atlas_acta_postgres/src/migrations` (Acta-owned migrations,
//! including every `SET SCHEMA acta` batch). All three are DDL authored to
//! run at a specific point in migration history, before or exactly at the
//! schema move — by construction, their unqualified references describe the
//! schema as it existed at that point, not a post-move violation.
//!
//! A handful of migration-step tests (e.g.
//! `TestDb::create_with_migration_steps`) deliberately freeze a database at a
//! historical step before a table's `SET SCHEMA` migration runs, and seed
//! rows via raw SQL matching that exact pre-move shape. Those lines are
//! correct as unqualified SQL, not a violation, and are bracketed with an
//! explicit `// schema-gate:off` / `// schema-gate:on` marker pair (renamed
//! from `// custos-schema-gate:off` / `:on` in this PR) so the exception is
//! visible in the diff rather than silently carved out by file path.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// Every table a `SET SCHEMA` migration has moved so far, paired with its
/// owning schema. A table absent from this map has not moved yet and is not
/// checked here — the R8 classification gate
/// (`atlas_acta_postgres/tests/r8_classification_gate.rs`) tracks the full
/// D1 inventory independently of when each batch's `SET SCHEMA` lands.
const TABLE_SCHEMA: &[(&str, &str)] = &[
    ("users", "custos"),
    ("sessions", "custos"),
    ("user_activation_tokens", "custos"),
    ("api_keys", "custos"),
    ("groups", "custos"),
    ("group_members", "custos"),
    ("permission_grants", "custos"),
    ("security_audit_log", "custos"),
    ("workspaces", "acta"),
    ("workspace_memberships", "acta"),
    ("property_definitions", "acta"),
    ("projects", "acta"),
    ("folders", "acta"),
    ("documents", "acta"),
    ("document_revisions", "acta"),
    ("document_links", "acta"),
    ("attachments", "acta"),
    ("attachment_write_intents", "acta"),
    ("comment_attachment_drafts", "acta"),
    ("comment_attachment_draft_uploads", "acta"),
    ("boards", "acta"),
    ("board_columns", "acta"),
    ("tasks", "acta"),
    ("task_references", "acta"),
    ("task_assignees", "acta"),
    ("task_checklist_items", "acta"),
    ("task_activity", "acta"),
    ("workspace_status_templates", "acta"),
    ("platform_status_templates", "acta"),
    ("comments", "acta"),
    ("comment_links", "acta"),
    ("comment_link_events", "acta"),
    ("tags", "acta"),
    ("events_outbox", "acta"),
    ("webhook_subscriptions", "acta"),
    ("webhook_delivery_log", "acta"),
    ("automation_rules", "acta"),
    ("integration_configs", "acta"),
    ("saved_searches", "acta"),
    ("task_views", "acta"),
];

const SQL_KEYWORDS: &[&str] = &["FROM", "JOIN", "INTO", "UPDATE", "TABLE"];

const EXCLUDED_DIR_SUFFIXES: &[&str] = &[
    "crates/migration",
    "crates/atlas_custos_postgres/src/migrations",
    "crates/atlas_acta_postgres/src/migrations",
];

#[test]
fn no_unqualified_reference_to_a_moved_table_exists_outside_migration_authoring() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .join("../..")
        .canonicalize()
        .expect("resolve workspace root");
    let crates_dir = workspace_root.join("crates");

    let violations = scan_for_violations(&crates_dir, &workspace_root);

    assert!(
        violations.is_empty(),
        "found unqualified references to moved tables outside migration authoring:\n{}",
        violations.join("\n")
    );
}

/// Boundary test (T11.9): the gate must flag a deliberately reintroduced
/// unqualified reference to a moved table. Writes a probe file into a
/// scratch `crates/` tree (never the real workspace), asserts the scan
/// reports exactly the expected violation, then removes the scratch tree.
#[test]
fn the_gate_flags_a_reintroduced_unqualified_reference() {
    let scratch_root = std::env::temp_dir().join(format!(
        "schema_qualification_gate_probe_{}",
        std::process::id()
    ));
    let scratch_crates = scratch_root.join("crates").join("probe_crate").join("src");
    fs::create_dir_all(&scratch_crates).expect("create scratch crate dir");

    // Built via `format!` rather than one literal so this test file's own
    // source never contains the unqualified pattern it is asserting the
    // gate detects elsewhere.
    let probe_sql = format!("SELECT * {} workspaces", "FROM");
    let probe_content = format!("fn regression() -> &'static str {{\n    \"{probe_sql}\"\n}}\n");

    let probe_path = scratch_crates.join("lib.rs");
    fs::write(&probe_path, probe_content).expect("write probe file");

    let violations = scan_for_violations(&scratch_root.join("crates"), &scratch_root);

    fs::remove_dir_all(&scratch_root).expect("remove scratch tree");

    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("workspaces")
                && violation.contains("acta.workspaces")),
        "expected the gate to flag the reintroduced unqualified `workspaces` reference, got:\n{}",
        violations.join("\n")
    );
}

fn scan_for_violations(crates_dir: &Path, workspace_root: &Path) -> Vec<String> {
    let mut violations = Vec::new();

    for path in rust_source_files(crates_dir) {
        if is_excluded(&path, workspace_root) {
            continue;
        }

        let content = fs::read_to_string(&path).expect("read source file");
        let mut gate_disabled = false;

        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();

            if trimmed.starts_with("// schema-gate:off") {
                gate_disabled = true;
                continue;
            }
            if trimmed.starts_with("// schema-gate:on") {
                gate_disabled = false;
                continue;
            }
            if gate_disabled || trimmed.starts_with("//") {
                continue;
            }

            for (table, schema) in TABLE_SCHEMA {
                if !line.contains(table) {
                    continue;
                }
                if line.contains(&format!("{schema}.{table}")) {
                    continue;
                }

                for keyword in SQL_KEYWORDS {
                    let unqualified_pattern = format!("{keyword} {table}");
                    if line.contains(&unqualified_pattern) {
                        violations.push(format!(
                            "{}:{}: unqualified reference to `{table}` — must read `{schema}.{table}`",
                            path.display(),
                            line_no + 1
                        ));
                    }
                }
            }
        }
    }

    violations
}

fn is_excluded(path: &Path, workspace_root: &Path) -> bool {
    let relative = path
        .strip_prefix(workspace_root)
        .expect("source file is under the workspace root");
    let relative_str = relative.to_string_lossy();

    EXCLUDED_DIR_SUFFIXES
        .iter()
        .any(|excluded| relative_str.starts_with(excluded))
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
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == "target")
                {
                    continue;
                }
                queue.push_back(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    files
}
