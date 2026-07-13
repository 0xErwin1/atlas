use atlas_domain::{
    entities::identity::MemberRole,
    ids::{ApiKeyId, UserId, WorkspaceId},
    permissions::{
        Capability, CapabilityAction, CapabilityFamily, Principal, ResourceRef, ResourceRole,
    },
};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use sea_orm_migration::prelude::MigratorTrait;
use uuid::Uuid;

use super::batch_authorization::{
    BatchAuthorizationSource, PgBatchAuthorizationSource, PrincipalFacts, ProjectionAuthContext,
};

#[tokio::test]
async fn query_b_reloads_user_membership_and_active_state() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let context = user_context(workspace_id, user_id);

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("load active member facts");
    assert_user_facts(&facts, true, Some(MemberRole::Member));

    db.conn
        .execute_unprepared(&format!(
            "DELETE FROM workspace_memberships WHERE workspace_id = '{workspace_id}' AND user_id = '{user_id}'"
        ))
        .await
        .expect("remove membership");

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload removed membership");
    assert_user_facts(&facts, true, None);

    db.conn
        .execute_unprepared(&format!(
            "UPDATE users SET disabled_at = now() WHERE id = '{user_id}'"
        ))
        .await
        .expect("disable user");

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload disabled user");
    assert_user_facts(&facts, false, None);

    db.teardown().await;
}

#[tokio::test]
async fn query_b_reloads_key_creator_and_live_group_grant_facts() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let creator_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();
    let group_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, creator_id, true).await;
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, scopes) \
             VALUES ('{key_id}', NULL, '{creator_id}', 'key', 'hash', 'agent', now(), false, ARRAY['docs:read'])"
        ))
        .await
        .expect("insert key");
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO groups (id, workspace_id, name, created_by, created_at, updated_at) \
             VALUES ('{group_id}', '{workspace_id}', 'group', '{creator_id}', now(), now()); \
             INSERT INTO group_members (group_id, user_id, created_at) VALUES ('{group_id}', '{creator_id}', now()); \
             INSERT INTO permission_grants (id, workspace_id, api_key_id, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{key_id}', 'viewer', now(), now()); \
             INSERT INTO permission_grants (id, workspace_id, group_id, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{group_id}', 'editor', now(), now())",
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await
        .expect("insert grants");

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let context = ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::ApiKey(ApiKeyId(key_id)),
    );

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("load key facts");
    assert_key_facts(&facts, key_id, creator_id, false, true, true);

    db.conn
        .execute_unprepared(&format!(
            "UPDATE api_keys SET revoked_at = now(), expires_at = now() - interval '1 second', is_global = true, scopes = ARRAY['tasks:read'] WHERE id = '{key_id}'; \
             UPDATE groups SET deleted_at = now() WHERE id = '{group_id}'; \
             UPDATE users SET disabled_at = now() WHERE id = '{creator_id}'"
        ))
        .await
        .expect("mutate current authority facts");

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload changed key facts");
    let PrincipalFacts::ApiKey(facts) = facts else {
        panic!("expected API-key facts");
    };
    assert!(facts.is_revoked);
    assert!(facts.is_expired);
    assert!(facts.is_global);
    assert_eq!(
        facts.scopes,
        vec![Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        }]
    );
    assert!(!facts.creator.is_active);
    assert_eq!(
        facts.grants,
        vec![(ResourceRef::Workspace, ResourceRole::Viewer)]
    );
    assert!(facts.creator.grants.is_empty());

    db.conn
        .execute_unprepared(&format!(
            "DELETE FROM permission_grants WHERE api_key_id = '{key_id}'"
        ))
        .await
        .expect("remove direct key grant");
    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload removed direct key grant");
    let PrincipalFacts::ApiKey(facts) = facts else {
        panic!("expected API-key facts");
    };
    assert!(facts.grants.is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn query_b_rejects_unknown_scopes_and_propagates_sql_failures() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let creator_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, creator_id, true).await;
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, scopes) \
             VALUES ('{key_id}', NULL, '{creator_id}', 'key', 'hash', 'agent', now(), false, ARRAY['unknown:read'])"
        ))
        .await
        .expect("insert key with unknown scope");

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let context = ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::ApiKey(ApiKeyId(key_id)),
    );
    assert!(source.load_principal_facts(&context, &[]).await.is_err());

    db.conn
        .execute_unprepared("DROP TABLE permission_grants")
        .await
        .expect("remove query dependency");
    assert!(source.load_principal_facts(&context, &[]).await.is_err());

    db.teardown().await;
}

fn user_context(workspace_id: Uuid, user_id: Uuid) -> ProjectionAuthContext {
    ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::User(UserId(user_id)),
    )
}

fn assert_user_facts(facts: &PrincipalFacts, is_active: bool, membership: Option<MemberRole>) {
    let PrincipalFacts::User(facts) = facts else {
        panic!("expected user facts");
    };
    assert_eq!(facts.is_active, is_active);
    assert_eq!(facts.effective_membership, membership);
}

fn assert_key_facts(
    facts: &PrincipalFacts,
    key_id: Uuid,
    creator_id: Uuid,
    is_global: bool,
    creator_active: bool,
    creator_group_grant_present: bool,
) {
    let PrincipalFacts::ApiKey(facts) = facts else {
        panic!("expected API-key facts");
    };
    assert_eq!(facts.key_id, ApiKeyId(key_id));
    assert_eq!(facts.creator.user_id, UserId(creator_id));
    assert_eq!(facts.is_global, is_global);
    assert_eq!(facts.creator.is_active, creator_active);
    assert_eq!(
        facts
            .creator
            .grants
            .contains(&(ResourceRef::Workspace, ResourceRole::Editor)),
        creator_group_grant_present,
    );
}

async fn seed_workspace_user(
    conn: &DatabaseConnection,
    workspace_id: Uuid,
    user_id: Uuid,
    member: bool,
) {
    conn.execute_unprepared(&format!(
        "INSERT INTO users (id, username, display_name, is_root, is_system_admin, created_at, updated_at) \
         VALUES ('{user_id}', 'user-{user_id}', 'User', false, false, now(), now()); \
         INSERT INTO workspaces (id, name, slug, created_at, updated_at) \
         VALUES ('{workspace_id}', 'Workspace', 'workspace-{workspace_id}', now(), now())"
    ))
    .await
    .expect("seed user and workspace");

    if member {
        conn.execute_unprepared(&format!(
            "INSERT INTO workspace_memberships (id, workspace_id, user_id, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{user_id}', 'member', now(), now())",
            Uuid::now_v7(),
        ))
        .await
        .expect("seed membership");
    }
}

struct BatchAuthorizationDb {
    conn: DatabaseConnection,
    name: String,
    admin_url: String,
}

impl BatchAuthorizationDb {
    async fn create() -> Self {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://atlas:atlas@localhost:5432/atlas_dev".to_string());
        let admin_url = replace_database_name(&database_url, "postgres");
        let name = format!("atlas_batch_authz_{}", Uuid::now_v7().as_simple());
        let admin = Database::connect(&admin_url)
            .await
            .expect("connect admin database");
        admin
            .execute_unprepared(&format!("CREATE DATABASE \"{name}\""))
            .await
            .expect("create test database");
        drop(admin);

        let conn = Database::connect(replace_database_name(&database_url, &name))
            .await
            .expect("connect test database");
        migration::Migrator::up(&conn, None)
            .await
            .expect("migrate test database");

        Self {
            conn,
            name,
            admin_url,
        }
    }

    async fn teardown(self) {
        drop(self.conn);
        let admin = Database::connect(&self.admin_url)
            .await
            .expect("connect admin database");
        admin
            .execute_unprepared(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.name
            ))
            .await
            .expect("drop test database");
    }
}

fn replace_database_name(url: &str, name: &str) -> String {
    let Some(index) = url.rfind('/') else {
        panic!("database URL has no database name");
    };
    let (prefix, suffix) = url.split_at(index + 1);
    let query_start = suffix.find('?').unwrap_or(suffix.len());
    format!("{prefix}{name}{}", &suffix[query_start..])
}
