#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `v2-e11-s6` PR1, design D3/D3.2 — proves the committed
//! `apps/web/openapi.json` is byte-identical to what `dump_openapi` would
//! print for the server's `document()` at this commit (INV-DRIFT-FAILS-CI).
//!
//! The expected bytes reproduce `dump_openapi`'s exact pipeline
//! (`src/bin/dump_openapi.rs`): `serde_json::to_string_pretty`, then one
//! trailing newline from `writeln!`. This is deliberately not the served
//! route's body — `openapi_json()` (`routes/openapi.rs`) serializes
//! compactly, with no trailing newline, so a byte comparison against it
//! could never pass regardless of drift (design §0.6).
//!
//! No workflow change, no container, no server spawn: `openapi()` is a pure
//! function over compiled-in registry data, so this test runs in the
//! existing `cargo test -p atlas_server` shards alongside
//! `openapi_zero_drift.rs`.

use std::path::PathBuf;

use atlas_server::routes::openapi::openapi;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root lies two directories above this crate's manifest")
}

/// Byte-equality check with a message pointing at the regeneration command,
/// factored out so [`self_test_probe_detects_one_byte_mutation`] can prove
/// it has teeth before it gates the real comparison.
fn assert_committed_matches(committed: &str, expected: &str) {
    assert_eq!(
        committed, expected,
        "apps/web/openapi.json has drifted from the server's document — run \
         `pnpm --filter @atlas/web run gen-types` to regenerate it"
    );
}

/// D3.2's self-test: a comparison that never fails proves nothing. This
/// confirms [`assert_committed_matches`] panics on a one-byte mutation and
/// stays silent on an exact match, before [`committed_document_matches_server`]
/// relies on it for the real comparison.
#[test]
fn self_test_probe_detects_one_byte_mutation() {
    let real = "line one\nline two\nline three\n".to_string();
    let mutated = format!("{}X{}", &real[..real.len() - 2], &real[real.len() - 1..]);
    assert_ne!(
        real, mutated,
        "the mutated copy must differ from the real string"
    );

    assert_committed_matches(&real, &real);

    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(|| assert_committed_matches(&mutated, &real));
    std::panic::set_hook(previous_hook);

    assert!(
        result.is_err(),
        "assert_committed_matches must panic on a one-byte mutation"
    );
}

/// The main drift assertion (INV-DRIFT-FAILS-CI): the committed document
/// equals exactly what `dump_openapi` would print for this commit's
/// `document()`.
#[test]
fn committed_document_matches_server() {
    let committed = std::fs::read_to_string(repo_root().join("apps/web/openapi.json"))
        .expect("apps/web/openapi.json is committed to the repository");
    let expected = format!(
        "{}\n",
        serde_json::to_string_pretty(&openapi()).expect("document serializes to pretty JSON")
    );

    assert_committed_matches(&committed, &expected);
}

fn operations(
    path_item: &utoipa::openapi::path::PathItem,
) -> Vec<&utoipa::openapi::path::Operation> {
    [
        path_item.get.as_ref(),
        path_item.put.as_ref(),
        path_item.post.as_ref(),
        path_item.delete.as_ref(),
        path_item.options.as_ref(),
        path_item.head.as_ref(),
        path_item.patch.as_ref(),
        path_item.trace.as_ref(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// A probe against the worst-case "reads nothing" implementation (§0.1's
/// measured baseline): 143 path keys, 216 operations, every one of them
/// carrying `x-atlas-component`. A drift check that silently compared an
/// empty or truncated document to itself would still pass the byte
/// comparison above; these counts catch that.
#[test]
fn document_counts_match_the_measured_baseline() {
    let document = openapi();

    assert_eq!(
        document.paths.paths.len(),
        143,
        "committed document's path key count drifted from the measured baseline (design §0.1)"
    );

    let mut operation_count = 0usize;
    let mut stamped_count = 0usize;

    for path_item in document.paths.paths.values() {
        for operation in operations(path_item) {
            operation_count += 1;

            let is_stamped = operation
                .extensions
                .as_ref()
                .is_some_and(|extensions| extensions.get("x-atlas-component").is_some());
            if is_stamped {
                stamped_count += 1;
            }
        }
    }

    assert_eq!(
        operation_count, 216,
        "committed document's operation count drifted from the measured baseline (design §0.1)"
    );
    assert_eq!(
        stamped_count, 216,
        "every operation must carry x-atlas-component (design §0.1)"
    );
}
