//! T6.10 — names the two S3-deferred CUSTOS-DB items as explicit E4 scope, so
//! neither is silently introduced by a later S3 patch.
//!
//! - **`custos.principals`** (design D6): S3's FK-inventory gate is
//!   outbound-only; introducing a `principals` table would be a data-model
//!   change (a backfill union of users and api keys, repointing nine Acta
//!   FKs) with observable identity semantics, out of scope for a
//!   behavior-preserving refactor. This crate's `entities` module never
//!   declares a `principals` submodule.
//! - **`members_of`** (design D3): `PrincipalFactsSource` keeps the
//!   `workspace_memberships` join in `QUERY_B_PRINCIPAL_FACTS` rather than
//!   resolving membership through `atlas_core::capabilities::ResourceProvider
//!   ::members_of`, already named as deferred debt by
//!   `atlas_server::authz::batch_authorization_db_tests::membership_join_debt_tests
//!   ::query_b_still_joins_workspace_memberships_as_named_e4_debt`.

#![allow(clippy::expect_used)]

#[test]
fn atlas_custos_declares_no_principals_module() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let entities_dir = std::path::Path::new(manifest_dir).join("src/entities");

    let mod_rs =
        std::fs::read_to_string(entities_dir.join("mod.rs")).expect("read entities/mod.rs");

    assert!(
        !mod_rs.contains("principals"),
        "atlas_custos::entities must not declare a `principals` module before E4 (design D6)"
    );
    assert!(
        !entities_dir.join("principals.rs").exists(),
        "atlas_custos::entities::principals must not exist before E4 (design D6)"
    );
}
