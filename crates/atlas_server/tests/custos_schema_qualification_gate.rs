#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! CI grep gate (design §S3d, spec "PR7 — `SET SCHEMA custos` with qualified
//! production SQL and a CI grep gate").
//!
//! Runs in the opposite direction from
//! `atlas_custos_postgres/tests/dependency_boundary.rs::no_acta_table_names_in_sql`:
//! that test keeps Acta table names out of the Custos-owned crate; this one
//! keeps every unqualified reference to the eight Custos tables out of the
//! rest of the workspace, now that `m20260830_000051_custos_set_schema` has
//! moved them out of `public`. The sanctioned form is schema-qualified SQL
//! (`custos.<table>`) — that qualification is what let production code and
//! `atlas_custos_postgres` itself keep talking to these tables straight
//! through raw SQL and sea-orm entities, without a new composition API.
//!
//! Excluded from the scan: `crates/migration` (the frozen historical block,
//! D5) and `crates/atlas_custos_postgres/src/migrations` (Custos-owned
//! migrations, including this PR's own `SET SCHEMA` migration). Both are
//! DDL authored to run at a specific point in migration history, before or
//! exactly at the schema move — by construction, their unqualified
//! references describe the schema as it existed at that point, not a
//! post-move violation.
//!
//! A handful of migration-step tests (e.g.
//! `TestDb::create_with_migration_steps`) deliberately freeze a database at a
//! historical step before `custos_new()` — and therefore before the SET
//! SCHEMA migration — runs, and seed rows via raw SQL matching that exact
//! pre-move shape. Those lines are correct as unqualified SQL, not a
//! violation, and are bracketed with an explicit
//! `// custos-schema-gate:off` / `// custos-schema-gate:on` marker pair so
//! the exception is visible in the diff rather than silently carved out by
//! file path.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

const CUSTOS_TABLES: &[&str] = &[
    "users",
    "sessions",
    "user_activation_tokens",
    "api_keys",
    "groups",
    "group_members",
    "permission_grants",
    "security_audit_log",
];

const SQL_KEYWORDS: &[&str] = &["FROM", "JOIN", "INTO", "UPDATE", "TABLE"];

const EXCLUDED_DIR_SUFFIXES: &[&str] = &[
    "crates/migration",
    "crates/atlas_custos_postgres/src/migrations",
];

#[test]
fn no_unqualified_custos_table_reference_exists_outside_migration_authoring() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .join("../..")
        .canonicalize()
        .expect("resolve workspace root");
    let crates_dir = workspace_root.join("crates");

    let mut violations = Vec::new();

    for path in rust_source_files(&crates_dir) {
        if is_excluded(&path, &workspace_root) {
            continue;
        }

        let content = fs::read_to_string(&path).expect("read source file");
        let mut gate_disabled = false;

        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();

            if trimmed.starts_with("// custos-schema-gate:off") {
                gate_disabled = true;
                continue;
            }
            if trimmed.starts_with("// custos-schema-gate:on") {
                gate_disabled = false;
                continue;
            }
            if gate_disabled || trimmed.starts_with("//") {
                continue;
            }

            for table in CUSTOS_TABLES {
                if !line.contains(table) {
                    continue;
                }
                if line.contains(&format!("custos.{table}")) {
                    continue;
                }

                for keyword in SQL_KEYWORDS {
                    let unqualified_pattern = format!("{keyword} {table}");
                    if line.contains(&unqualified_pattern) {
                        violations.push(format!(
                            "{}:{}: unqualified reference to `{table}` — must read `custos.{table}`",
                            path.display(),
                            line_no + 1
                        ));
                    }
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found unqualified references to Custos-owned tables outside migration authoring:\n{}",
        violations.join("\n")
    );
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
