#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! D6 static regression guard (spec Scenario "Existing raw joins keep
//! compiling and passing").
//!
//! `search.rs`, `boards_tasks.rs`, `workspace_core.rs`, and `documents.rs`
//! each read `custos.users`/`custos.api_keys` via raw SQL predating S4.
//! Design §D6 requires these joins to stay exactly as they are: S4
//! schema-qualifies only the Acta side of these queries (unqualified table
//! names became `acta.<table>`), never the already-qualified Custos side.
//!
//! This test greps the workspace for each expected snippet, verbatim, so a
//! future change that accidentally rewrites or drops one of these
//! cross-schema reads is caught here instead of only by a full-suite run.
//! It is a text-presence guard, not a query-semantics test — the actual
//! read/write behavior is covered by each repo's own integration tests
//! (e.g. `boards_tasks_repos_characterization.rs`,
//! `search_index_queue_lifecycle_repos_characterization.rs`).

use std::fs;
use std::path::Path;

/// (file path relative to the workspace root, expected verbatim snippet).
const EXPECTED_RAW_JOINS: &[(&str, &str)] = &[
    (
        "crates/atlas_acta_postgres/src/repos/search.rs",
        "JOIN custos.users u ON u.id = ta.assignee_user_id",
    ),
    (
        "crates/atlas_acta_postgres/src/repos/boards_tasks.rs",
        "LEFT JOIN custos.api_keys ak ON ak.id = ta.assignee_api_key_id",
    ),
    (
        "crates/atlas_server/src/persistence/repos/workspace_core.rs",
        "SELECT 1 FROM custos.users",
    ),
    (
        "crates/atlas_acta_postgres/src/repos/documents.rs",
        "SELECT 1 FROM custos.users",
    ),
];

#[test]
fn expected_acta_to_custos_raw_reads_are_unchanged() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root");

    let mut violations = Vec::new();

    for (relative_path, expected_snippet) in EXPECTED_RAW_JOINS {
        let path = workspace_root.join(relative_path);
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        if !content.contains(expected_snippet) {
            violations.push(format!(
                "{relative_path}: expected snippet not found: `{expected_snippet}`"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "an Acta->Custos raw read changed or disappeared:\n{}",
        violations.join("\n")
    );
}
