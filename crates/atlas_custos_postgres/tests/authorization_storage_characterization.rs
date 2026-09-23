//! Container-backed characterization tests for the V2 authorization storage
//! (`m20260923_000058_custos_v2_authorization`): the `custos.roles`,
//! `custos.grants_v2` and `custos.deny_rules` tables, their CHECK
//! constraints, and the `PgRoleRepo`/`PgGrantV2Repo`/`PgDenyRuleRepo`
//! adapters. Runs against a disposable Postgres named by
//! `ATLAS_TEST_DATABASE_URL` (see `atlas_test_db`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use atlas_test_db::TestDb;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

/// Number of migrations to apply so that the named Custos migration is the
/// last one applied (counted inclusively, after the frozen historical block).
/// Pinned by name so appending further Custos migrations cannot silently
/// shift these prefixes.
fn steps_through(migration_name: &str) -> u32 {
    let historical = migration::Migrator::migrations().len();
    let custos = atlas_custos_postgres::migrations::custos_new();
    let mut steps = historical as u32;
    for m in &custos {
        steps += 1;
        if m.name() == migration_name {
            return steps;
        }
    }
    panic!("custos migration {migration_name} not found in custos_new()");
}

const AUTHORIZATION_MIGRATION: &str = "m20260923_000058_custos_v2_authorization";
const PREDECESSOR_MIGRATION: &str = "m20260920_000057_custos_session_root_reason";

async fn exec(db: &TestDb, sql: String) {
    db.conn()
        .execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .expect("execute statement");
}

/// Executes `sql` and returns the database error message it was rejected
/// with, failing the test when the statement is accepted.
async fn expect_rejected(db: &TestDb, sql: String) -> String {
    db.conn()
        .execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .expect_err("statement must be rejected")
        .to_string()
}

#[derive(Debug, FromQueryResult)]
struct ExistsRow {
    present: bool,
}

async fn table_exists(db: &TestDb, table: &str) -> bool {
    ExistsRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT EXISTS ( \
                 SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = 'custos' AND table_name = '{table}' \
             ) AS present"
        ),
    ))
    .one(db.conn())
    .await
    .expect("query information_schema")
    .expect("exists row")
    .present
}

async fn index_exists(db: &TestDb, index: &str) -> bool {
    ExistsRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_indexes \
                 WHERE schemaname = 'custos' AND indexname = '{index}' \
             ) AS present"
        ),
    ))
    .one(db.conn())
    .await
    .expect("query pg_indexes")
    .expect("exists row")
    .present
}

async fn assert_authorization_schema_present(db: &TestDb) {
    for table in ["roles", "grants_v2", "deny_rules"] {
        assert!(table_exists(db, table).await, "custos.{table} must exist");
    }

    for index in [
        "custos_grants_v2_product_target_idx",
        "custos_grants_v2_subject_principal_idx",
        "custos_grants_v2_subject_group_idx",
        "custos_grants_v2_subject_principal_set_idx",
        "custos_grants_v2_role_id_idx",
        "custos_deny_rules_product_target_idx",
        "custos_deny_rules_subject_principal_idx",
        "custos_deny_rules_subject_group_idx",
        "custos_deny_rules_subject_principal_set_idx",
    ] {
        assert!(index_exists(db, index).await, "index {index} must exist");
    }
}

#[tokio::test]
async fn authorization_migration_applies_on_a_fresh_database() {
    let db = TestDb::create().await.expect("TestDb::create");

    assert_authorization_schema_present(&db).await;

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn authorization_migration_applies_after_the_root_reason_migration() {
    assert_eq!(
        steps_through(PREDECESSOR_MIGRATION) + 1,
        steps_through(AUTHORIZATION_MIGRATION),
        "the authorization migration must directly follow its named predecessor"
    );

    let db = TestDb::create_with_migration_steps(Some(steps_through(PREDECESSOR_MIGRATION)))
        .await
        .expect("TestDb::create_with_migration_steps");
    assert!(!table_exists(&db, "roles").await);
    assert!(!table_exists(&db, "grants_v2").await);
    assert!(!table_exists(&db, "deny_rules").await);

    db.run_remaining_migrations()
        .await
        .expect("apply remaining migrations");

    assert_authorization_schema_present(&db).await;

    db.teardown().await.expect("teardown");
}

fn grant_insert(columns: &str, values: &str) -> String {
    format!(
        "INSERT INTO custos.grants_v2 \
            (id, target_kind, target, product, created_by, {columns}) \
         VALUES ('{}', 'ref', 'acta::document::d1', 'acta', '{}', {values})",
        Uuid::now_v7(),
        Uuid::now_v7()
    )
}

fn deny_insert(columns: &str, values: &str) -> String {
    format!(
        "INSERT INTO custos.deny_rules \
            (id, target_kind, target, product, created_by, {columns}) \
         VALUES ('{}', 'ref', 'acta::document::d1', 'acta', '{}', {values})",
        Uuid::now_v7(),
        Uuid::now_v7()
    )
}

const ACTIONS_AUTHORITY: &str = "authority_kind, actions";
const ACTIONS_VALUES: &str = "'actions', ARRAY['acta::document::read']";
const PRINCIPAL_SUBJECT: &str = "subject_kind, subject_principal_id";

fn principal_values() -> String {
    format!("'principal', '{}'", Uuid::now_v7())
}

#[tokio::test]
async fn grants_v2_rejects_a_row_with_two_subjects() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        grant_insert(
            &format!("subject_kind, subject_principal_id, subject_group_id, {ACTIONS_AUTHORITY}"),
            &format!(
                "'principal', '{}', '{}', {ACTIONS_VALUES}",
                Uuid::now_v7(),
                Uuid::now_v7()
            ),
        ),
    )
    .await;
    assert!(
        error.contains("custos_grants_v2_subject_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_rejects_a_row_with_zero_subjects() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        grant_insert(
            &format!("subject_kind, {ACTIONS_AUTHORITY}"),
            &format!("'principal', {ACTIONS_VALUES}"),
        ),
    )
    .await;
    assert!(
        error.contains("custos_grants_v2_subject_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_rejects_a_subject_column_that_disagrees_with_its_kind() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        grant_insert(
            &format!("subject_kind, subject_group_id, {ACTIONS_AUTHORITY}"),
            &format!("'principal', '{}', {ACTIONS_VALUES}", Uuid::now_v7()),
        ),
    )
    .await;
    assert!(
        error.contains("custos_grants_v2_subject_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_rejects_a_row_with_two_authorities() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        grant_insert(
            &format!("{PRINCIPAL_SUBJECT}, authority_kind, role_name, role_version, actions"),
            &format!(
                "{}, 'builtin', 'editor', 1, ARRAY['acta::document::read']",
                principal_values()
            ),
        ),
    )
    .await;
    assert!(
        error.contains("custos_grants_v2_authority_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_rejects_a_row_with_zero_authorities() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        grant_insert(
            &format!("{PRINCIPAL_SUBJECT}, authority_kind"),
            &format!("{}, 'actions'", principal_values()),
        ),
    )
    .await;
    assert!(
        error.contains("custos_grants_v2_authority_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_rejects_an_empty_explicit_action_list() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        grant_insert(
            &format!("{PRINCIPAL_SUBJECT}, {ACTIONS_AUTHORITY}"),
            &format!("{}, 'actions', ARRAY[]::TEXT[]", principal_values()),
        ),
    )
    .await;
    assert!(
        error.contains("custos_grants_v2_authority_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_rejects_a_custom_role_reference_that_does_not_exist() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        grant_insert(
            &format!("{PRINCIPAL_SUBJECT}, authority_kind, role_id"),
            &format!("{}, 'custom', '{}'", principal_values(), Uuid::now_v7()),
        ),
    )
    .await;
    assert!(
        error.contains("custos_grants_v2_role_id_fkey"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn deny_rules_rejects_a_row_with_two_subjects() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        deny_insert(
            "subject_kind, subject_group_id, subject_principal_set, actions",
            &format!(
                "'group', '{}', 'acta::workspace::w1::members', ARRAY['acta::document::read']",
                Uuid::now_v7()
            ),
        ),
    )
    .await;
    assert!(
        error.contains("custos_deny_rules_subject_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn deny_rules_rejects_a_row_with_zero_subjects() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        deny_insert(
            "subject_kind, actions",
            "'group', ARRAY['acta::document::read']",
        ),
    )
    .await;
    assert!(
        error.contains("custos_deny_rules_subject_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn deny_rules_rejects_an_empty_action_list() {
    let db = TestDb::create().await.expect("TestDb::create");

    let error = expect_rejected(
        &db,
        deny_insert(
            &format!("{PRINCIPAL_SUBJECT}, actions"),
            &format!("{}, ARRAY[]::TEXT[]", principal_values()),
        ),
    )
    .await;
    assert!(
        error.contains("custos_deny_rules_actions_check"),
        "got: {error}"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn roles_rejects_an_empty_action_list_and_a_duplicate_product_name() {
    let db = TestDb::create().await.expect("TestDb::create");

    let empty = expect_rejected(
        &db,
        format!(
            "INSERT INTO custos.roles (id, product, name, actions, created_by) \
             VALUES ('{}', 'acta', 'reviewer', ARRAY[]::TEXT[], '{}')",
            Uuid::now_v7(),
            Uuid::now_v7()
        ),
    )
    .await;
    assert!(empty.contains("custos_roles_actions_check"), "got: {empty}");

    exec(
        &db,
        format!(
            "INSERT INTO custos.roles (id, product, name, actions, created_by) \
             VALUES ('{}', 'acta', 'reviewer', ARRAY['acta::document::read'], '{}')",
            Uuid::now_v7(),
            Uuid::now_v7()
        ),
    )
    .await;
    let duplicate = expect_rejected(
        &db,
        format!(
            "INSERT INTO custos.roles (id, product, name, actions, created_by) \
             VALUES ('{}', 'acta', 'reviewer', ARRAY['acta::document::update'], '{}')",
            Uuid::now_v7(),
            Uuid::now_v7()
        ),
    )
    .await;
    assert!(
        duplicate.contains("custos_roles_product_name_key"),
        "got: {duplicate}"
    );

    db.teardown().await.expect("teardown");
}

// ---------------------------------------------------------------------------
// Repository adapters
// ---------------------------------------------------------------------------

use atlas_core::error::DomainError;
use atlas_core::ids::{ActionId, PrincipalSetId};
use atlas_custos::entities::authorization::{
    DenyRuleId, GrantAuthority, GrantId, NewCustomRole, NewDenyRecord, NewGrantRecord,
    ROLE_IN_USE_CONFLICT, RoleId, SubjectRecord, SubjectSet, TargetRecord,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use atlas_custos::ports::authorization::{DenyRuleRepo, GrantV2Repo, RoleRepo};
use atlas_custos_postgres::repos::authorization::{PgDenyRuleRepo, PgGrantV2Repo, PgRoleRepo};

fn role_repo(db: &TestDb) -> PgRoleRepo {
    PgRoleRepo {
        conn: db.conn().clone(),
    }
}

fn grant_repo(db: &TestDb) -> PgGrantV2Repo {
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
}

fn deny_repo(db: &TestDb) -> PgDenyRuleRepo {
    PgDenyRuleRepo {
        conn: db.conn().clone(),
    }
}

fn action(raw: &str) -> ActionId {
    raw.parse().expect("valid action id")
}

fn principal_set(raw: &str) -> PrincipalSetId {
    raw.parse().expect("valid principal set id")
}

fn every_subject() -> Vec<SubjectRecord> {
    vec![
        SubjectRecord::Principal(PrincipalId::new()),
        SubjectRecord::Group(GroupId::new()),
        SubjectRecord::PrincipalSet(principal_set("acta::workspace::w1::members")),
    ]
}

fn every_target() -> Vec<TargetRecord> {
    vec![
        TargetRecord::Ref("acta::document::d1".parse().unwrap()),
        TargetRecord::Path(
            "acta::workspace::w1/project::p1/document::d1"
                .parse()
                .unwrap(),
        ),
        TargetRecord::Selector("acta::workspace::w1/project::p1/**".parse().unwrap()),
    ]
}

fn new_role(product: &str, name: &str) -> NewCustomRole {
    NewCustomRole {
        id: RoleId::new(),
        product: product.to_string(),
        name: name.to_string(),
        actions: vec![
            action(&format!("{product}::document::read")),
            action(&format!("{product}::document::update")),
        ],
        created_by: PrincipalId::new(),
    }
}

fn new_grant(
    subject: SubjectRecord,
    target: TargetRecord,
    authority: GrantAuthority,
) -> NewGrantRecord {
    NewGrantRecord {
        id: GrantId::new(),
        subject,
        target,
        authority,
        created_by: PrincipalId::new(),
    }
}

fn new_deny(subject: SubjectRecord, target: TargetRecord) -> NewDenyRecord {
    NewDenyRecord {
        id: DenyRuleId::new(),
        subject,
        target,
        actions: vec![action("acta::document::delete")],
        created_by: PrincipalId::new(),
    }
}

fn explicit_actions() -> GrantAuthority {
    GrantAuthority::Actions(vec![
        action("acta::document::read"),
        action("acta::document::update"),
    ])
}

#[derive(Debug, FromQueryResult)]
struct StoredTargetRow {
    target_kind: String,
    target: String,
    product: String,
}

async fn stored_target(db: &TestDb, table: &str, id: Uuid) -> StoredTargetRow {
    StoredTargetRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!("SELECT target_kind, target, product FROM custos.{table} WHERE id = '{id}'"),
    ))
    .one(db.conn())
    .await
    .expect("query stored target")
    .expect("row exists")
}

#[tokio::test]
async fn roles_round_trip_and_list_by_product() {
    let db = TestDb::create().await.expect("TestDb::create");
    let repo = role_repo(&db);

    let acta_role = repo
        .create(new_role("acta", "reviewer"))
        .await
        .expect("create acta role");
    let custos_role = repo
        .create(new_role("custos", "reviewer"))
        .await
        .expect("create custos role with the same name in another product");

    let found = repo
        .get(acta_role.id)
        .await
        .expect("get role")
        .expect("role exists");
    assert_eq!(found, acta_role);
    assert_eq!(
        found.actions,
        vec![
            action("acta::document::read"),
            action("acta::document::update")
        ]
    );

    let listed = repo.list_by_product("acta").await.expect("list roles");
    assert_eq!(listed, vec![acta_role.clone()]);
    assert_eq!(
        repo.list_by_product("custos").await.expect("list roles"),
        vec![custos_role]
    );

    let duplicate = repo
        .create(NewCustomRole {
            id: RoleId::new(),
            ..new_role("acta", "reviewer")
        })
        .await
        .expect_err("a duplicate (product, name) must be rejected");
    assert!(
        matches!(duplicate, DomainError::AlreadyExists { .. }),
        "got: {duplicate:?}"
    );

    assert!(repo.delete(acta_role.id).await.expect("delete role"));
    assert!(!repo.delete(acta_role.id).await.expect("delete again"));
    assert!(repo.get(acta_role.id).await.expect("get").is_none());

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_round_trip_every_subject_target_and_authority_shape() {
    let db = TestDb::create().await.expect("TestDb::create");
    let repo = grant_repo(&db);
    let role = role_repo(&db)
        .create(new_role("acta", "reviewer"))
        .await
        .expect("create role");

    let authorities = vec![
        GrantAuthority::Builtin {
            name: "editor".to_string(),
            version: 3,
        },
        GrantAuthority::CustomRole(role.id),
        explicit_actions(),
    ];

    let mut created_ids = Vec::new();
    for subject in every_subject() {
        for target in every_target() {
            for authority in &authorities {
                let new = new_grant(subject.clone(), target.clone(), authority.clone());
                let created = repo.create(new.clone()).await.expect("create grant");
                assert_eq!(created.id, new.id);
                assert_eq!(created.subject, new.subject);
                assert_eq!(created.target, new.target);
                assert_eq!(created.authority, new.authority);
                assert_eq!(created.created_by, new.created_by);

                let found = repo
                    .get(created.id)
                    .await
                    .expect("get grant")
                    .expect("grant exists");
                assert_eq!(found, created);

                let stored = stored_target(&db, "grants_v2", created.id.0).await;
                assert_eq!(stored.target_kind, target.kind().as_str());
                assert_eq!(stored.target, target.canonical());
                assert_eq!(stored.product, "acta");

                created_ids.push(created.id);
            }
        }
    }
    assert_eq!(created_ids.len(), 27);

    let listed = repo.list_by_product("acta").await.expect("list grants");
    assert_eq!(
        listed.iter().map(|g| g.id).collect::<Vec<_>>(),
        created_ids,
        "list_by_product returns every grant in insertion order"
    );
    assert!(
        repo.list_by_product("custos")
            .await
            .expect("list grants")
            .is_empty()
    );

    let first = created_ids[0];
    assert!(repo.delete(first).await.expect("delete grant"));
    assert!(!repo.delete(first).await.expect("delete again"));
    assert!(repo.get(first).await.expect("get").is_none());
    assert!(repo.get(GrantId::new()).await.expect("get").is_none());

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn deny_rules_round_trip_every_subject_and_target_shape() {
    let db = TestDb::create().await.expect("TestDb::create");
    let repo = deny_repo(&db);

    let mut created_ids = Vec::new();
    for subject in every_subject() {
        for target in every_target() {
            let new = new_deny(subject.clone(), target.clone());
            let created = repo.create(new.clone()).await.expect("create deny rule");
            assert_eq!(created.id, new.id);
            assert_eq!(created.subject, new.subject);
            assert_eq!(created.target, new.target);
            assert_eq!(created.actions, new.actions);
            assert_eq!(created.created_by, new.created_by);

            let found = repo
                .get(created.id)
                .await
                .expect("get deny rule")
                .expect("deny rule exists");
            assert_eq!(found, created);

            let stored = stored_target(&db, "deny_rules", created.id.0).await;
            assert_eq!(stored.target_kind, target.kind().as_str());
            assert_eq!(stored.target, target.canonical());
            assert_eq!(stored.product, "acta");

            created_ids.push(created.id);
        }
    }
    assert_eq!(created_ids.len(), 9);

    let listed = repo.list_by_product("acta").await.expect("list deny rules");
    assert_eq!(listed.iter().map(|d| d.id).collect::<Vec<_>>(), created_ids);
    assert!(
        repo.list_by_product("custos")
            .await
            .expect("list deny rules")
            .is_empty()
    );

    let first = created_ids[0];
    assert!(repo.delete(first).await.expect("delete deny rule"));
    assert!(!repo.delete(first).await.expect("delete again"));
    assert!(repo.get(first).await.expect("get").is_none());

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn deleting_a_role_referenced_by_a_grant_fails_until_the_grant_is_gone() {
    let db = TestDb::create().await.expect("TestDb::create");
    let roles = role_repo(&db);
    let grants = grant_repo(&db);

    let role = roles
        .create(new_role("acta", "reviewer"))
        .await
        .expect("create role");
    let grant = grants
        .create(new_grant(
            SubjectRecord::Principal(PrincipalId::new()),
            TargetRecord::Ref("acta::document::d1".parse().unwrap()),
            GrantAuthority::CustomRole(role.id),
        ))
        .await
        .expect("create grant");

    let blocked = roles
        .delete(role.id)
        .await
        .expect_err("a referenced role must not be deletable");
    assert!(
        matches!(
            blocked,
            DomainError::ComponentConflict {
                code: ROLE_IN_USE_CONFLICT,
                ..
            }
        ),
        "got: {blocked:?}"
    );
    assert!(
        roles.get(role.id).await.expect("get role").is_some(),
        "the role survives the blocked delete"
    );

    assert!(grants.delete(grant.id).await.expect("delete grant"));
    assert!(
        roles
            .delete(role.id)
            .await
            .expect("delete unreferenced role")
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn grants_v2_list_for_subjects_returns_only_rows_addressed_to_the_subject_set() {
    let db = TestDb::create().await.expect("TestDb::create");
    let repo = grant_repo(&db);

    let subjects = SubjectSet {
        principal: PrincipalId::new(),
        groups: vec![GroupId::new(), GroupId::new()],
        principal_sets: vec![principal_set("acta::workspace::w1::members")],
    };
    let target = TargetRecord::Ref("acta::document::d1".parse().unwrap());

    let mut expected = Vec::new();
    for subject in [
        SubjectRecord::Principal(subjects.principal),
        SubjectRecord::Group(subjects.groups[1]),
        SubjectRecord::PrincipalSet(subjects.principal_sets[0].clone()),
    ] {
        let created = repo
            .create(new_grant(subject, target.clone(), explicit_actions()))
            .await
            .expect("create matching grant");
        expected.push(created.id);
    }

    for other in [
        SubjectRecord::Principal(PrincipalId::new()),
        SubjectRecord::Group(GroupId::new()),
        SubjectRecord::PrincipalSet(principal_set("acta::workspace::w1::admins")),
    ] {
        repo.create(new_grant(other, target.clone(), explicit_actions()))
            .await
            .expect("create non-matching grant");
    }
    repo.create(new_grant(
        SubjectRecord::Principal(subjects.principal),
        TargetRecord::Ref("custos::group::g1".parse().unwrap()),
        GrantAuthority::Actions(vec![action("custos::group::read")]),
    ))
    .await
    .expect("create grant in another product");

    let listed = repo
        .list_for_subjects("acta", &subjects)
        .await
        .expect("list for subjects");
    assert_eq!(listed.iter().map(|g| g.id).collect::<Vec<_>>(), expected);

    let alone = SubjectSet {
        principal: PrincipalId::new(),
        groups: vec![],
        principal_sets: vec![],
    };
    assert!(
        repo.list_for_subjects("acta", &alone)
            .await
            .expect("list for an unknown principal")
            .is_empty()
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn deny_rules_list_for_subjects_returns_only_rows_addressed_to_the_subject_set() {
    let db = TestDb::create().await.expect("TestDb::create");
    let repo = deny_repo(&db);

    let subjects = SubjectSet {
        principal: PrincipalId::new(),
        groups: vec![GroupId::new()],
        principal_sets: vec![
            principal_set("acta::workspace::w1::members"),
            principal_set("acta::workspace::w1::reviewers"),
        ],
    };
    let target = TargetRecord::Selector("acta::workspace::w1/**".parse().unwrap());

    let mut expected = Vec::new();
    for subject in [
        SubjectRecord::Principal(subjects.principal),
        SubjectRecord::Group(subjects.groups[0]),
        SubjectRecord::PrincipalSet(subjects.principal_sets[1].clone()),
    ] {
        let created = repo
            .create(new_deny(subject, target.clone()))
            .await
            .expect("create matching deny rule");
        expected.push(created.id);
    }

    for other in [
        SubjectRecord::Principal(PrincipalId::new()),
        SubjectRecord::Group(GroupId::new()),
        SubjectRecord::PrincipalSet(principal_set("acta::workspace::w2::members")),
    ] {
        repo.create(new_deny(other, target.clone()))
            .await
            .expect("create non-matching deny rule");
    }
    repo.create(NewDenyRecord {
        actions: vec![action("custos::group::read")],
        ..new_deny(
            SubjectRecord::Principal(subjects.principal),
            TargetRecord::Ref("custos::group::g1".parse().unwrap()),
        )
    })
    .await
    .expect("create deny rule in another product");

    let listed = repo
        .list_for_subjects("acta", &subjects)
        .await
        .expect("list for subjects");
    assert_eq!(listed.iter().map(|d| d.id).collect::<Vec<_>>(), expected);

    db.teardown().await.expect("teardown");
}
