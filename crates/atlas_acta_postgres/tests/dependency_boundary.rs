#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Enforces that `atlas_acta_postgres` depends only on `atlas_core`,
//! `atlas_acta`, and `atlas_postgres` (plus std/third-party deps) and never
//! on `atlas_custos` or any application crate (design D2's forbidden-edge
//! rejection: mirrors `atlas_custos_postgres`'s `dependency_boundary` test,
//! run in the opposite direction).

use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::process::Command;

/// Product/application crates `atlas_acta_postgres` must never reach,
/// directly or transitively.
const FORBIDDEN: &[&str] = &["atlas_custos", "atlas_api", "atlas_server"];

#[test]
fn atlas_acta_postgres_dependency_closure_excludes_forbidden_crates() {
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

    let closure = dependency_closure(&metadata, "atlas_acta_postgres");

    for forbidden in FORBIDDEN {
        assert!(
            !closure.contains(*forbidden),
            "atlas_acta_postgres's dependency closure includes forbidden crate `{forbidden}`; \
             atlas_acta_postgres must depend only on atlas_core, atlas_acta, and atlas_postgres"
        );
    }
}

#[test]
fn atlas_server_depends_on_atlas_acta_postgres() {
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

    let closure = dependency_closure(&metadata, "atlas_server");

    assert!(
        closure.contains("atlas_acta_postgres"),
        "atlas_server's dependency closure must include atlas_acta_postgres so the \
         `persistence::entities` re-export facade (S4 PR1, R1 scaffolding) resolves"
    );
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

    let name_by_id: HashMap<&str, &str> = packages
        .iter()
        .map(|package| {
            (
                package["id"].as_str().expect("package.id is a string"),
                package["name"].as_str().expect("package.name is a string"),
            )
        })
        .collect();

    let node_by_id: HashMap<&str, &Value> = nodes
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
