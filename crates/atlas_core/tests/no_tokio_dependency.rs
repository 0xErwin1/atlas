//! E11-S2 orchestrator decision (2026-09-04): `atlas_core` stays free of any
//! async runtime dependency, in every PR of this slice. The supervisor that
//! needs `tokio`/`tokio-util` lives in `atlas_server::ops` (E11-S3b), built
//! against the contracts this slice ships (INV-NO-TOKIO).
//!
//! A source-level test over `Cargo.toml` rather than `cargo tree`: it needs
//! no network, no lockfile resolution, and fails immediately (not just at
//! CI's dependency-audit step) if a future edit adds the key back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

const FORBIDDEN_DEPENDENCY_KEYS: &[&str] = &["tokio", "tokio-util", "futures"];

fn cargo_toml_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    fs::read_to_string(&path).expect("read atlas_core/Cargo.toml")
}

/// A crude but sufficient TOML dependency-table-key check: this file's
/// `[dependencies]`/`[dev-dependencies]` entries are always written as
/// `key.workspace = true` or `key = "..."` at the start of a line, so a
/// line-anchored match on `^{key}` (ignoring leading whitespace) cannot
/// false-positive on a substring inside another key or a comment.
fn declares_dependency(source: &str, key: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with(['=', '.']))
    })
}

#[test]
fn atlas_core_cargo_toml_declares_no_forbidden_runtime_dependency() {
    let source = cargo_toml_text();

    for key in FORBIDDEN_DEPENDENCY_KEYS {
        assert!(
            !declares_dependency(&source, key),
            "atlas_core/Cargo.toml must not declare a `{key}` dependency (INV-NO-TOKIO): \n{source}"
        );
    }
}

#[test]
fn the_forbidden_dependency_check_is_non_vacuous() {
    let source = "async-trait.workspace = true\ntokio.workspace = true\n";
    assert!(
        declares_dependency(source, "tokio"),
        "the line-anchored matcher must actually detect a `tokio` dependency line"
    );
    assert!(!declares_dependency(source, "tokio-util"));
}
