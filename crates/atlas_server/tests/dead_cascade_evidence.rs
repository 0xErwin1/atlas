#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! T6.1 — evidence that the four dead Custos-outbound cascades D1 drops
//! (`permission_grants.workspace_id`, `groups.workspace_id`,
//! `api_keys.workspace_id`, `security_audit_log.workspace_id`, all `ON
//! DELETE CASCADE`/`NO ACTION` onto `workspaces`) never fire in this
//! codebase: nothing hard-deletes a workspace row. A whole-workspace grep
//! for the two ways that could happen — raw SQL `DELETE FROM workspaces` or
//! `workspace::Entity::delete*` — finding zero call sites is the
//! behavior-neutrality argument the migration's FK drop relies on.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn no_call_site_hard_deletes_a_workspace_row() {
    let workspace_root = workspace_root();
    let crates_dir = workspace_root.join("crates");

    let mut violations = Vec::new();
    for path in rust_source_files(&crates_dir) {
        // The migration crate is expected to name `workspaces` in historical
        // DDL and in this migration's own FK-drop statements; it is not a
        // hard-delete call site. This test file's own literals (the two
        // strings it greps for) are excluded from its own scan.
        if path.components().any(|c| c.as_os_str() == "migration")
            || path.file_name().and_then(|n| n.to_str()) == Some("dead_cascade_evidence.rs")
        {
            continue;
        }

        let content = fs::read_to_string(&path).expect("read source file");
        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }

            // schema-gate:off — these literals are the grep patterns this
            // test itself searches for, not live queries; they are text data,
            // not references needing `acta.` qualification. Both the bare and
            // the schema-qualified DELETE forms are guarded: after PR11 every
            // correct call site writes `acta.workspaces`, so matching only
            // the bare form would defang this guard.
            if line.contains("DELETE FROM workspaces")
                || line.contains("DELETE FROM acta.workspaces")
                || line.contains("workspace::Entity::delete")
            // schema-gate:on
            {
                violations.push(format!("{}:{}: {}", path.display(), line_no + 1, trimmed));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found a workspace hard-delete call site; the dead-cascade argument for D1's \
         `workspace_id` FK drops no longer holds:\n{}",
        violations.join("\n")
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/atlas_server has a workspace root two levels up")
        .to_path_buf()
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
                if path.file_name().and_then(|n| n.to_str()) == Some("target") {
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
