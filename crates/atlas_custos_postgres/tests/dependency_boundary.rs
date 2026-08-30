#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Enforces that `atlas_custos_postgres` depends only on `atlas_core`,
//! `atlas_custos`, and `atlas_postgres` (plus std/third-party deps) and never
//! on `atlas_acta` or any application crate.
//!
//! This test enforces only the cargo dependency edge, matching
//! `crates/atlas_custos/tests/dependency_boundary.rs`. It cannot observe raw
//! SQL against another component's tables; the raw-SQL half of the boundary
//! (no Acta-owned table touched from this crate) is documented in `lib.rs`
//! together with its known temporary exception, and is closed by the revoke
//! split in the next change of this series.

use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::process::Command;

/// Product/application crates `atlas_custos_postgres` must never reach,
/// directly or transitively.
const FORBIDDEN: &[&str] = &["atlas_acta", "atlas_api", "atlas_server"];

#[test]
fn atlas_custos_postgres_dependency_closure_excludes_forbidden_crates() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_manifest = format!("{manifest_dir}/../../Cargo.toml");

    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--manifest-path",
            &workspace_manifest,
        ])
        .output()
        .expect("failed to run `cargo metadata`");

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata produced invalid JSON");

    let closure = dependency_closure(&metadata, "atlas_custos_postgres");

    for forbidden in FORBIDDEN {
        assert!(
            !closure.contains(*forbidden),
            "atlas_custos_postgres's dependency closure includes forbidden crate `{forbidden}`; \
             atlas_custos_postgres must depend only on atlas_core, atlas_custos, and \
             atlas_postgres"
        );
    }
}

/// Returns the set of package names reachable from `root_package`, following
/// only non-dev dependency edges (a `dev` dependency is test-only wiring, not
/// part of the crate's shipped dependency graph).
fn dependency_closure(metadata: &Value, root_package: &str) -> HashSet<String> {
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata.packages is an array");
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("metadata.resolve.nodes is an array");

    let name_by_id: std::collections::HashMap<&str, &str> = packages
        .iter()
        .map(|package| {
            (
                package["id"].as_str().expect("package.id is a string"),
                package["name"].as_str().expect("package.name is a string"),
            )
        })
        .collect();

    let node_by_id: std::collections::HashMap<&str, &Value> = nodes
        .iter()
        .map(|node| (node["id"].as_str().expect("node.id is a string"), node))
        .collect();

    let root_id = *name_by_id
        .iter()
        .find(|(_, name)| **name == root_package)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("package `{root_package}` not found in cargo metadata"));

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::from([root_id]);
    let mut seen_ids: HashSet<&str> = HashSet::from([root_id]);

    while let Some(id) = queue.pop_front() {
        let Some(node) = node_by_id.get(id) else {
            continue;
        };
        let deps = node["deps"].as_array().expect("node.deps is an array");

        for dep in deps {
            let is_dev_only = dep["dep_kinds"]
                .as_array()
                .expect("dep.dep_kinds is an array")
                .iter()
                .all(|kind| kind["kind"].as_str() == Some("dev"));

            if is_dev_only {
                continue;
            }

            let dep_id = dep["pkg"].as_str().expect("dep.pkg is a string");
            let dep_name = name_by_id
                .get(dep_id)
                .unwrap_or_else(|| panic!("dependency package id `{dep_id}` not in packages"));

            visited.insert((*dep_name).to_owned());

            if seen_ids.insert(dep_id) {
                queue.push_back(dep_id);
            }
        }
    }

    visited
}
