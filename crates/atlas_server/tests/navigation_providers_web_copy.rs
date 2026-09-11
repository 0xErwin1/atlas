#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! E11-S8 PR2 — the web navigation manifest audit
//! (`navigationManifest.test.ts`) checks `NAVIGATION_MANIFEST` against a
//! hand-written copy of `reg5.rs`'s `navigation_providers` declarations,
//! `apps/web/src/shell/declaredNavigationProviders.ts`. Without this test
//! that copy could drift from the registry silently, and the web audit
//! would keep passing against a stale list.
//!
//! Mirrors `openapi_committed_document_drift.rs`'s reverse-check pattern:
//! read the committed file from disk, tolerantly extract the values it
//! declares, and assert set equality against what the real registry
//! builds. Not a full TS/JS parser — the web file's shape is deliberately
//! pinned to a single flat `as const` string array so this extraction
//! stays simple and has teeth ([`extraction_detects_a_fabricated_id`]
//! proves it).

use std::collections::BTreeSet;
use std::path::PathBuf;

use atlas_core::registry::build;
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root lies two directories above this crate's manifest")
}

/// Every `navigation_providers` id declared across every REG-5 component
/// entry (design D-S8-4), deduplicated and sorted for a deterministic
/// comparison. The storage backend is irrelevant here — `navigation_providers`
/// is declared on `platform`/`custos`/`acta`, none of which vary by backend.
fn registry_declared_ids() -> BTreeSet<String> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .unwrap_or_else(|errors| panic!("reg5 entries must build a valid registry: {errors:?}"));

    registry
        .entries()
        .iter()
        .flat_map(|entry| entry.experience.navigation_providers.iter().cloned())
        .collect()
}

/// Tolerant extraction of the string literals inside
/// `DECLARED_NAVIGATION_PROVIDERS = [...]`: locates the array literal's
/// brackets after the constant's name, then every single- or
/// double-quoted string literal between them.
fn extract_declared_ids(source: &str) -> BTreeSet<String> {
    let marker = "DECLARED_NAVIGATION_PROVIDERS";
    let marker_at = source
        .find(marker)
        .unwrap_or_else(|| panic!("`{marker}` not found in declaredNavigationProviders.ts"));
    let after_marker = &source[marker_at..];

    let open = after_marker
        .find('[')
        .unwrap_or_else(|| panic!("no `[` found after `{marker}`"));
    let close = after_marker[open..]
        .find(']')
        .map(|offset| open + offset)
        .unwrap_or_else(|| panic!("no closing `]` found for `{marker}`'s array literal"));
    let array_body = &after_marker[open + 1..close];

    let mut ids = BTreeSet::new();
    let mut chars = array_body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' || c == '"' {
            let quote = c;
            let mut literal = String::new();
            for next in chars.by_ref() {
                if next == quote {
                    break;
                }
                literal.push(next);
            }
            ids.insert(literal);
        }
    }
    ids
}

fn web_copy_source() -> String {
    std::fs::read_to_string(repo_root().join("apps/web/src/shell/declaredNavigationProviders.ts"))
        .expect("apps/web/src/shell/declaredNavigationProviders.ts is committed to the repository")
}

/// The main drift assertion: every id the registry declares on
/// `navigation_providers` has a matching entry in the web's hand-written
/// copy, and vice versa — so `navigationManifest.test.ts`'s bidirectional
/// manifest audit is checking against a value that itself cannot silently
/// drift from `reg5.rs`.
#[test]
fn web_copy_matches_registry_declared_navigation_providers() {
    let registry_ids = registry_declared_ids();
    let web_ids = extract_declared_ids(&web_copy_source());

    let missing_from_web: Vec<_> = registry_ids.difference(&web_ids).collect();
    let extra_in_web: Vec<_> = web_ids.difference(&registry_ids).collect();

    assert!(
        missing_from_web.is_empty() && extra_in_web.is_empty(),
        "declaredNavigationProviders.ts has drifted from reg5.rs's navigation_providers \
         declarations — missing from the web copy: {missing_from_web:?}, extra in the web \
         copy (fabricated or stale): {extra_in_web:?}"
    );
}

/// Proves [`extract_declared_ids`] has teeth: it must not silently return
/// the same set when a fabricated id is added to the source, and it must
/// extract exactly the ids a real `declaredNavigationProviders.ts` carries.
#[test]
fn extraction_detects_a_fabricated_id() {
    let source =
        "export const DECLARED_NAVIGATION_PROVIDERS = ['acta.workspace', 'custos.admin'] as const;";
    let ids = extract_declared_ids(source);
    assert_eq!(
        ids,
        BTreeSet::from(["acta.workspace".to_string(), "custos.admin".to_string()])
    );

    let fabricated_source = "export const DECLARED_NAVIGATION_PROVIDERS = \
         ['acta.workspace', 'custos.admin', 'fabricated.provider'] as const;";
    let fabricated_ids = extract_declared_ids(fabricated_source);

    assert_ne!(ids, fabricated_ids, "extraction must detect an added id");
    assert!(fabricated_ids.contains("fabricated.provider"));
}
