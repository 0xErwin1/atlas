//! Container-backed characterization tests for the read-only adapters behind
//! the authorization service (E5 S6a): Custos resource existence per kind,
//! Custos's provider over Postgres, the actor's group memberships from the
//! V1 group tables, and the stored-facts load. Runs against a disposable
//! Postgres named by `ATLAS_TEST_DATABASE_URL` (see `atlas_test_db`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashSet;

use atlas_core::capabilities::{ResourceExistence, ResourceProvider};
use atlas_core::ids::{ActionId, PrincipalSetId, ResourcePath, ResourceRef};
use atlas_core::principal::UserId;
use atlas_core::registry::Authorization;
use atlas_custos::WorkspaceScope;
use atlas_custos::entities::authorization::{
    DenyRuleId, GrantAuthority, GrantId, NewCustomRole, NewDenyRecord, NewGrantRecord, RoleId,
    SubjectRecord, TargetRecord,
};
use atlas_custos::entities::groups::NewGroup;
use atlas_custos::entities::identity::{ApiKeyKind, ApiKeyType, NewApiKey, NewSession, NewUser};
use atlas_custos::ids::{GroupId, PrincipalId};
use atlas_custos::ports::authorization::{DenyRuleRepo, GrantV2Repo, RoleRepo};
use atlas_custos::ports::authorize::{
    AuthorizationFactsStore, DeclaredSet, GroupMembershipSource, ProductScope,
};
use atlas_custos::ports::group_repo::GroupRepo;
use atlas_custos::ports::identity::{AgentRepo, SessionRepo, UserRepo};
use atlas_custos::provider::{CustosKind, CustosResourceProvider, CustosResourceStore};
use atlas_custos_postgres::repos::authorization::{PgDenyRuleRepo, PgGrantV2Repo, PgRoleRepo};
use atlas_custos_postgres::repos::authorize::{
    PgAuthorizationFactsStore, PgCustosResourceStore, PgGroupMembershipSource,
};
use atlas_custos_postgres::repos::identity::{
    PgAgentRepo, PgApiKeyRepo, PgSessionRepo, PgUserRepo,
};
use atlas_custos_postgres::repos::permissions::PgGroupRepo;
use atlas_test_db::TestDb;
use chrono::Utc;
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

/// The prefix of `historical() ++ custos_new()`: these adapters only touch
/// Custos-owned tables.
fn custos_only_migration_steps() -> u32 {
    let historical = migration::Migrator::migrations().len();
    let custos = atlas_custos_postgres::migrations::custos_new().len();
    (historical + custos) as u32
}

async fn create_custos_only_db() -> TestDb {
    TestDb::create_with_migration_steps(Some(custos_only_migration_steps()))
        .await
        .expect("TestDb::create_with_migration_steps")
}

fn action(raw: &str) -> ActionId {
    raw.parse().unwrap()
}

/// Every seeded Custos row, one live instance per row-backed kind plus the
/// negative cases each kind must not confuse with its own.
struct Seeded {
    user: UserId,
    agent: PrincipalId,
    group: GroupId,
    deleted_group: GroupId,
    role: RoleId,
    grant: GrantId,
    deny: DenyRuleId,
    session: Uuid,
    personal_key: Uuid,
    agent_key: Uuid,
}

async fn seed_user(db: &TestDb, username: &str) -> UserId {
    PgUserRepo {
        conn: db.conn().clone(),
    }
    .create(NewUser {
        username: username.to_string(),
        display_name: username.to_string(),
        email: None,
        password_hash: None,
        is_root: false,
        is_system_admin: false,
    })
    .await
    .expect("seed user")
    .id
}

fn new_key(name: &str) -> NewApiKey {
    NewApiKey {
        name: name.to_string(),
        token_hash: format!("hash-{name}"),
        type_: ApiKeyType::Agent,
        expires_at: None,
        scopes: Vec::new(),
    }
}

async fn seed_everything(db: &TestDb) -> Seeded {
    let conn = db.conn().clone();
    let user = seed_user(db, "alice").await;
    let agent = PgAgentRepo { conn: conn.clone() }
        .create(user, "helper".to_string())
        .await
        .expect("seed agent")
        .id;

    let groups = PgGroupRepo { conn: conn.clone() };
    let scope = WorkspaceScope(Uuid::now_v7());
    let group = groups
        .create(NewGroup {
            workspace_id: scope,
            name: "live".to_string(),
            created_by: user,
        })
        .await
        .expect("seed group")
        .id;
    let deleted_group = groups
        .create(NewGroup {
            workspace_id: scope,
            name: "gone".to_string(),
            created_by: user,
        })
        .await
        .expect("seed deleted group")
        .id;
    groups
        .soft_delete(deleted_group, scope)
        .await
        .expect("soft delete group");

    let role = PgRoleRepo { conn: conn.clone() }
        .create(NewCustomRole {
            id: RoleId::new(),
            product: "acta".to_string(),
            name: "reviewer".to_string(),
            actions: vec![action("acta::document::read")],
            created_by: PrincipalId(user.0),
        })
        .await
        .expect("seed role")
        .id;
    let grant = PgGrantV2Repo { conn: conn.clone() }
        .create(NewGrantRecord {
            id: GrantId::new(),
            subject: SubjectRecord::Principal(PrincipalId(user.0)),
            target: TargetRecord::Ref("acta::workspace::w1".parse().unwrap()),
            authority: GrantAuthority::CustomRole(role),
            created_by: PrincipalId(user.0),
        })
        .await
        .expect("seed grant")
        .id;
    let deny = PgDenyRuleRepo { conn: conn.clone() }
        .create(NewDenyRecord {
            id: DenyRuleId::new(),
            subject: SubjectRecord::Principal(PrincipalId(user.0)),
            target: TargetRecord::Ref("acta::workspace::w1".parse().unwrap()),
            actions: vec![action("acta::document::read")],
            created_by: PrincipalId(user.0),
        })
        .await
        .expect("seed deny")
        .id;

    let session = PgSessionRepo { conn: conn.clone() }
        .create(NewSession {
            user_id: user,
            token_hash: "session-hash".to_string(),
            expires_at: Utc::now() + chrono::Duration::days(1),
            root_reason: None,
        })
        .await
        .expect("seed session")
        .id
        .0;
    let personal_key = PgApiKeyRepo::create_for_user_in_with_kind(
        &conn,
        user,
        ApiKeyKind::Personal,
        new_key("mine"),
    )
    .await
    .expect("seed personal key")
    .id
    .0;
    let agent_key =
        PgApiKeyRepo::create_for_user_in_with_kind(&conn, user, ApiKeyKind::Agent, new_key("bot"))
            .await
            .expect("seed agent key")
            .id
            .0;

    Seeded {
        user,
        agent,
        group,
        deleted_group,
        role,
        grant,
        deny,
        session,
        personal_key,
        agent_key,
    }
}

async fn existing(store: &PgCustosResourceStore, kind: CustosKind, ids: &[Uuid]) -> HashSet<Uuid> {
    store.existing(kind, ids).await.expect("existence query")
}

#[tokio::test]
async fn the_resource_store_reports_live_rows_of_each_kind_only() {
    let db = create_custos_only_db().await;
    let seeded = seed_everything(&db).await;
    let store = PgCustosResourceStore {
        conn: db.conn().clone(),
    };
    let unknown = Uuid::now_v7();
    let cases = [
        (CustosKind::User, seeded.user.0),
        (CustosKind::Agent, seeded.agent.0),
        (CustosKind::Group, seeded.group.0),
        (CustosKind::Role, seeded.role.0),
        (CustosKind::Grant, seeded.grant.0),
        (CustosKind::Deny, seeded.deny.0),
        (CustosKind::Session, seeded.session),
        (CustosKind::PersonalApiKey, seeded.personal_key),
        (CustosKind::AgentApiKey, seeded.agent_key),
    ];

    for (kind, present) in cases {
        assert_eq!(
            existing(&store, kind, &[present, unknown]).await,
            HashSet::from([present]),
            "{kind:?}"
        );
    }

    let confusions = [
        (CustosKind::Agent, seeded.user.0),
        (CustosKind::User, seeded.agent.0),
        (CustosKind::Group, seeded.deleted_group.0),
        (CustosKind::AgentApiKey, seeded.personal_key),
        (CustosKind::PersonalApiKey, seeded.agent_key),
        (CustosKind::Grant, seeded.deny.0),
    ];

    for (kind, other) in confusions {
        assert!(
            existing(&store, kind, &[other]).await.is_empty(),
            "{kind:?}"
        );
    }

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn the_custos_provider_answers_a_mixed_batch_from_postgres() {
    let db = create_custos_only_db().await;
    let seeded = seed_everything(&db).await;
    let provider = CustosResourceProvider::new(
        PgCustosResourceStore {
            conn: db.conn().clone(),
        },
        &Authorization {
            resource_kinds: vec!["user".to_string(), "group".to_string()],
            actions: Vec::new(),
            role_definitions: Vec::new(),
            role_definitions_v2: vec![],
            principal_sets: Vec::new(),
            provider: true,
        },
    );
    let custos = |kind: &str, id: String| -> ResourceRef {
        format!("custos::{kind}::{id}").parse().unwrap()
    };
    let resources = [
        custos("user", seeded.user.0.to_string()),
        custos("user", Uuid::now_v7().to_string()),
        custos("platform", "atlas".to_string()),
        custos("audit", Uuid::now_v7().to_string()),
        custos("widget", Uuid::now_v7().to_string()),
        custos("group", seeded.group.0.to_string()),
        custos("agent_api_key", seeded.agent_key.to_string()),
    ];

    let facts = provider.resource_facts(&resources).await.unwrap();

    let existence: Vec<ResourceExistence> = facts.iter().map(|fact| fact.existence).collect();
    assert_eq!(
        existence,
        vec![
            ResourceExistence::Exists,
            ResourceExistence::Missing,
            ResourceExistence::Exists,
            ResourceExistence::Missing,
            ResourceExistence::Missing,
            ResourceExistence::Exists,
            ResourceExistence::Exists,
        ]
    );
    for fact in facts {
        let expected = (fact.existence == ResourceExistence::Exists)
            .then(|| ResourcePath::from(fact.resource.clone()));
        assert_eq!(fact.path, expected);
    }

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn group_membership_lists_the_live_groups_of_a_user_only() {
    let db = create_custos_only_db().await;
    let seeded = seed_everything(&db).await;
    let groups = PgGroupRepo {
        conn: db.conn().clone(),
    };
    groups
        .add_member(seeded.group, seeded.user)
        .await
        .expect("join live group");
    groups
        .add_member(seeded.deleted_group, seeded.user)
        .await
        .expect("join deleted group");
    let loner = seed_user(&db, "loner").await;
    let source = PgGroupMembershipSource {
        conn: db.conn().clone(),
    };

    assert_eq!(
        source.groups_of(PrincipalId(seeded.user.0)).await.unwrap(),
        vec![seeded.group]
    );
    assert!(source.groups_of(seeded.agent).await.unwrap().is_empty());
    assert!(
        source
            .groups_of(PrincipalId(loner.0))
            .await
            .unwrap()
            .is_empty()
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn the_facts_store_loads_rows_that_can_reach_the_actor_in_scope() {
    let db = create_custos_only_db().await;
    let alice = PrincipalId(seed_user(&db, "alice").await.0);
    let bob = PrincipalId(seed_user(&db, "bob").await.0);
    let member_group = GroupId::new();
    let other_group = GroupId::new();
    let members: PrincipalSetId = "acta::workspace::w1::members".parse().unwrap();
    let conn = db.conn().clone();
    let roles = PgRoleRepo { conn: conn.clone() };
    let grants = PgGrantV2Repo { conn: conn.clone() };
    let denies = PgDenyRuleRepo { conn: conn.clone() };

    let referenced_role = roles
        .create(NewCustomRole {
            id: RoleId::new(),
            product: "acta".to_string(),
            name: "referenced".to_string(),
            actions: vec![action("acta::document::read")],
            created_by: alice,
        })
        .await
        .unwrap()
        .id;
    roles
        .create(NewCustomRole {
            id: RoleId::new(),
            product: "acta".to_string(),
            name: "unreferenced".to_string(),
            actions: vec![action("acta::document::read")],
            created_by: alice,
        })
        .await
        .unwrap();

    let mut reaching = Vec::new();
    let subjects = [
        (SubjectRecord::Principal(alice), true),
        (SubjectRecord::Group(member_group), true),
        (SubjectRecord::PrincipalSet(members.clone()), true),
        (SubjectRecord::Group(other_group), false),
        (SubjectRecord::Principal(bob), false),
    ];
    for (subject, reaches) in subjects {
        let id = grants
            .create(NewGrantRecord {
                id: GrantId::new(),
                subject: subject.clone(),
                target: TargetRecord::Ref("acta::workspace::w1".parse().unwrap()),
                authority: GrantAuthority::Actions(vec![action("acta::document::read")]),
                created_by: alice,
            })
            .await
            .unwrap()
            .id;
        if reaches {
            reaching.push(id);
        }
        denies
            .create(NewDenyRecord {
                id: DenyRuleId::new(),
                subject,
                target: TargetRecord::Ref("acta::folder::f1".parse().unwrap()),
                actions: vec![action("acta::document::update")],
                created_by: alice,
            })
            .await
            .unwrap();
    }
    let role_grant = grants
        .create(NewGrantRecord {
            id: GrantId::new(),
            subject: SubjectRecord::Principal(alice),
            target: TargetRecord::Ref("acta::workspace::w2".parse().unwrap()),
            authority: GrantAuthority::CustomRole(referenced_role),
            created_by: alice,
        })
        .await
        .unwrap()
        .id;
    reaching.push(role_grant);
    let custos_grant = grants
        .create(NewGrantRecord {
            id: GrantId::new(),
            subject: SubjectRecord::Principal(alice),
            target: TargetRecord::Ref("custos::platform::atlas".parse().unwrap()),
            authority: GrantAuthority::Actions(vec![action("custos::grant::create")]),
            created_by: alice,
        })
        .await
        .unwrap()
        .id;

    let store = PgAuthorizationFactsStore { conn };
    let declared = [declared_set("acta", "members")];
    let acta = store
        .load(
            &ProductScope::Products(vec!["acta".to_string()]),
            alice,
            &[member_group],
            &declared,
        )
        .await
        .unwrap();
    let everything = store
        .load(&ProductScope::All, alice, &[member_group], &declared)
        .await
        .unwrap();
    let nothing = store
        .load(
            &ProductScope::Products(Vec::new()),
            alice,
            &[member_group],
            &declared,
        )
        .await
        .unwrap();

    let ids = |records: &[atlas_custos::entities::authorization::GrantRecord]| -> HashSet<GrantId> {
        records.iter().map(|record| record.id).collect()
    };
    assert_eq!(ids(&acta.grants), reaching.iter().copied().collect());
    assert_eq!(acta.denies.len(), 3);
    assert!(acta.denies.iter().all(|rule| matches!(
        &rule.subject,
        SubjectRecord::Principal(id) if *id == alice
    ) || matches!(&rule.subject, SubjectRecord::Group(id) if *id == member_group)
        || matches!(&rule.subject, SubjectRecord::PrincipalSet(_))));
    assert_eq!(
        acta.custom_roles
            .iter()
            .map(|role| role.id)
            .collect::<Vec<_>>(),
        vec![referenced_role]
    );

    let mut all_ids = reaching.iter().copied().collect::<HashSet<_>>();
    all_ids.insert(custos_grant);
    assert_eq!(ids(&everything.grants), all_ids);
    assert!(nothing.grants.is_empty() && nothing.denies.is_empty());

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn the_custos_provider_answers_only_canonical_ids_from_postgres() {
    let db = create_custos_only_db().await;
    let seeded = seed_everything(&db).await;
    let provider = CustosResourceProvider::new(
        PgCustosResourceStore {
            conn: db.conn().clone(),
        },
        &Authorization {
            resource_kinds: Vec::new(),
            actions: Vec::new(),
            role_definitions: Vec::new(),
            role_definitions_v2: vec![],
            principal_sets: Vec::new(),
            provider: true,
        },
    );

    for (kind, id) in [("user", seeded.user.0), ("group", seeded.group.0)] {
        let spellings = [
            id.hyphenated().to_string(),
            id.hyphenated().to_string().to_uppercase(),
            id.simple().to_string(),
            id.braced().to_string(),
        ];
        let resources: Vec<ResourceRef> = spellings
            .iter()
            .map(|spelling| format!("custos::{kind}::{spelling}").parse().unwrap())
            .collect();

        let facts = provider.resource_facts(&resources).await.unwrap();

        assert_eq!(
            facts.iter().map(|fact| fact.existence).collect::<Vec<_>>(),
            vec![
                ResourceExistence::Exists,
                ResourceExistence::Missing,
                ResourceExistence::Missing,
                ResourceExistence::Missing,
            ],
            "{kind}"
        );
    }

    db.teardown().await.expect("teardown");
}

fn declared_set(product: &str, name: &str) -> DeclaredSet {
    DeclaredSet {
        product: product.to_string(),
        name: name.to_string(),
    }
}

#[tokio::test]
async fn the_facts_store_loads_set_rows_only_for_declared_sets() {
    let db = create_custos_only_db().await;
    let alice = PrincipalId(seed_user(&db, "alice").await.0);
    let conn = db.conn().clone();
    let grants = PgGrantV2Repo { conn: conn.clone() };
    let denies = PgDenyRuleRepo { conn: conn.clone() };
    let mut by_set = std::collections::HashMap::new();

    for raw in [
        "acta::workspace::w1::members",
        "acta::workspace::w2::guests",
        "acta::workspace::w3::memXbers",
    ] {
        let set: PrincipalSetId = raw.parse().unwrap();
        let grant = grants
            .create(NewGrantRecord {
                id: GrantId::new(),
                subject: SubjectRecord::PrincipalSet(set.clone()),
                target: TargetRecord::Ref("acta::workspace::w1".parse().unwrap()),
                authority: GrantAuthority::Actions(vec![action("acta::document::read")]),
                created_by: alice,
            })
            .await
            .unwrap()
            .id;
        denies
            .create(NewDenyRecord {
                id: DenyRuleId::new(),
                subject: SubjectRecord::PrincipalSet(set),
                target: TargetRecord::Ref("acta::workspace::w1".parse().unwrap()),
                actions: vec![action("acta::document::update")],
                created_by: alice,
            })
            .await
            .unwrap();
        by_set.insert(raw, grant);
    }

    let store = PgAuthorizationFactsStore { conn };
    let load = |declared: Vec<DeclaredSet>| {
        let store = &store;
        async move {
            store
                .load(&ProductScope::All, alice, &[], &declared)
                .await
                .unwrap()
        }
    };

    let members = load(vec![declared_set("acta", "members")]).await;
    assert_eq!(
        members
            .grants
            .iter()
            .map(|grant| grant.id)
            .collect::<Vec<_>>(),
        vec![by_set["acta::workspace::w1::members"]]
    );
    assert_eq!(members.denies.len(), 1);

    for declared in [
        Vec::new(),
        vec![declared_set("custos", "members")],
        vec![declared_set("acta", "mem_bers")],
    ] {
        let loaded = load(declared.clone()).await;

        assert!(
            loaded.grants.is_empty() && loaded.denies.is_empty(),
            "{declared:?}"
        );
    }

    db.teardown().await.expect("teardown");
}
