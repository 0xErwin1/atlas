#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Byte-identity gate for the OpenAPI document, frozen for the V2-E2 epic.
//!
//! `crates/atlas_postgres` and the slices that follow it are refactors that
//! must not change any observable route, status code, error body, or schema.
//! This test hashes the exact bytes `bin/dump_openapi.rs` writes and compares
//! them against a checked-in baseline captured before the epic started. If
//! this test fails, the change altered the OpenAPI document — revert the
//! change or, if the document is meant to change, that belongs to E3, not to
//! a runtime-extraction slice.

use atlas_server::routes::openapi::openapi;
use sha2::{Digest, Sha256};

const BASELINE_FIXTURE: &str = include_str!("fixtures/openapi_baseline.sha256");

#[test]
fn openapi_document_digest_matches_epic_baseline() {
    let json = serde_json::to_string_pretty(&openapi()).expect("serialize OpenAPI document");
    let bytes = format!("{json}\n");

    let digest = Sha256::digest(bytes.as_bytes());
    let digest_hex = format!("{digest:x}");

    let expected = BASELINE_FIXTURE.trim();

    assert_eq!(
        digest_hex, expected,
        "OpenAPI document digest changed. The V2-E2 epic freezes the OpenAPI \
         document as a neutral-runtime-extraction invariant — no route, status \
         code, error body, or schema may change in these slices. If this \
         change is intentional, it belongs to E3, not here."
    );
}
