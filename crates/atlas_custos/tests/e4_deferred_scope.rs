//! E4-S1 — pins the ADDITIVE shape of the `custos.principals` introduction,
//! replacing the S3-era deferral test whose premise (no principals module)
//! belonged to the rejected re-key design.
//!
//! - **`custos.principals` exists**: the pure crate declares and exports the
//!   `principals` entity, port and `PrincipalId`, so the shared identity is
//!   reachable without going through `atlas_core::ids::PrincipalId` (an
//!   unrelated string-segment type for capability principal sets).
//! - **No FK move**: the principals migration touches only `principals`,
//!   `users` and `api_keys` (new table, new columns). The outbound FKs on
//!   `permission_grants`, `sessions`, `security_audit_log`, `group_members`
//!   and `user_activation_tokens` are not repointed. Because this pure crate
//!   has no database dependency, the no-rekey guarantee is enforced where it
//!   can actually inspect the catalog:
//!   `atlas_custos_postgres::tests::principals_repo_characterization
//!   ::the_principals_migration_leaves_the_fk_holding_tables_outbound_fks_unchanged`
//!   snapshots every outbound FK constraint on those five tables from
//!   `pg_constraint` before and after the migration and asserts the sets are
//!   identical.
//! - **`members_of`** (design D3): `PrincipalFactsSource` keeps the
//!   `workspace_memberships` join in `QUERY_B_PRINCIPAL_FACTS` rather than
//!   resolving membership through `atlas_core::capabilities::ResourceProvider
//!   ::members_of`, already named as deferred debt by
//!   `atlas_server::authz::batch_authorization_db_tests::membership_join_debt_tests
//!   ::query_b_still_joins_workspace_memberships_as_named_e4_debt`.

#![allow(clippy::expect_used)]

#[test]
fn atlas_custos_declares_and_exports_the_principals_module() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let src_dir = std::path::Path::new(manifest_dir).join("src");

    let entities_mod =
        std::fs::read_to_string(src_dir.join("entities/mod.rs")).expect("read entities/mod.rs");
    let ports_mod =
        std::fs::read_to_string(src_dir.join("ports/mod.rs")).expect("read ports/mod.rs");

    assert!(
        entities_mod.contains("pub mod principals;"),
        "atlas_custos::entities must declare the `principals` module (E4-S1)"
    );
    assert!(
        ports_mod.contains("pub mod principals;"),
        "atlas_custos::ports must declare the `principals` module (E4-S1)"
    );
    assert!(
        src_dir.join("entities/principals.rs").exists(),
        "atlas_custos::entities::principals must exist (E4-S1)"
    );
    assert!(
        src_dir.join("ports/principals.rs").exists(),
        "atlas_custos::ports::principals must exist (E4-S1)"
    );

    let ids_rs = std::fs::read_to_string(src_dir.join("ids.rs")).expect("read ids.rs");
    assert!(
        ids_rs.contains("define_id!(PrincipalId);"),
        "atlas_custos::ids must define its own `PrincipalId` (E4-S1)"
    );
}

// The no-rekey guarantee for E4-S1 cannot be proven from source text (a
// text-absence check over the migration file proves nothing about the
// catalog), and this pure crate has no database dependency to inspect
// `pg_constraint`. The real guard is the DB-backed constraint inspection in
// `atlas_custos_postgres::tests::principals_repo_characterization
// ::the_principals_migration_leaves_the_fk_holding_tables_outbound_fks_unchanged`,
// which snapshots the five tables' outbound FKs before/after the migration
// and asserts they are identical. Nothing to assert here by design.
