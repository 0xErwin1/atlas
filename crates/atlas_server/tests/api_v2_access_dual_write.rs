//! Access dual-write (`v2-e7-s4`): every V1 access change (workspace
//! creation, membership add/update/remove, permission grant create/delete,
//! project visibility) leaves the equivalent V2 rows behind (`custos.roles`,
//! `custos.grants_v2`, `acta.workspaces.owner_principal_id`,
//! `acta.workspace_members`) while the V1 responses keep their shape.
//!
//! The V2 rows are read straight from the tables: no route reads them yet.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateGrantRequest, CreateProjectRequest, GrantPrincipal, UpdateProjectRequest,
};
use atlas_server::authz::v2_access::{ADMIN_ROLE_NAME, OWNER_ROLE_NAME, WOULD_REFUSE_TOTAL};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_core::capabilities::{CapabilityError, ProviderCatalog, ResourceFacts, ResourceProvider};
use atlas_core::ids::{PrincipalSetId, ResourceRef};
use atlas_custos::authorize::{AuthorizationService, AuthorizationSettings, ProviderSet};
use atlas_custos::eval::DenyMode;
use atlas_custos_postgres::repos::authorize::{PgAuthorizationFactsStore, PgGroupMembershipSource};
use atlas_server::authz::v2_service::{TokioSleeper, validation_catalog};
use atlas_server::state::AppState;
use metrics_exporter_prometheus::PrometheusBuilder;
use sea_orm::{DatabaseBackend, FromQueryResult, Statement};
use support::{TestDb, TestServer, login_user, login_user_with_workspace};

#[derive(Debug, FromQueryResult, PartialEq)]
struct GrantRow {
    subject_kind: String,
    subject_principal_id: Option<uuid::Uuid>,
    subject_principal_set: Option<String>,
    authority_kind: String,
    role_name: Option<String>,
    role_version: Option<i32>,
    role_id: Option<uuid::Uuid>,
    created_by: uuid::Uuid,
}

#[derive(Debug, FromQueryResult, PartialEq)]
struct MemberRow {
    principal_id: uuid::Uuid,
    role: String,
    source: String,
}

#[derive(Debug, FromQueryResult)]
struct RoleRow {
    id: uuid::Uuid,
    actions: Vec<String>,
}

#[derive(Debug, FromQueryResult)]
struct OwnerRow {
    owner_principal_id: Option<uuid::Uuid>,
}

/// Every V2 grant on `target`, oldest first.
async fn grants_on(db: &TestDb, target: &str) -> Vec<GrantRow> {
    GrantRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT subject_kind, subject_principal_id, subject_principal_set, authority_kind, \
         role_name, role_version, role_id, created_by \
         FROM custos.grants_v2 WHERE target = $1 ORDER BY created_at ASC, id ASC",
        [target.into()],
    ))
    .all(db.conn())
    .await
    .expect("query grants")
}

async fn members_of(db: &TestDb, workspace_id: uuid::Uuid) -> Vec<MemberRow> {
    MemberRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT principal_id, role, source FROM acta.workspace_members \
         WHERE workspace_id = $1 ORDER BY updated_at ASC, principal_id ASC",
        [workspace_id.into()],
    ))
    .all(db.conn())
    .await
    .expect("query projection")
}

async fn role_named(db: &TestDb, name: &str) -> Option<RoleRow> {
    RoleRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT id, actions FROM custos.roles WHERE product = 'acta' AND name = $1",
        [name.into()],
    ))
    .one(db.conn())
    .await
    .expect("query role")
}

async fn owner_role(db: &TestDb) -> Option<RoleRow> {
    role_named(db, OWNER_ROLE_NAME).await
}

async fn admin_role(db: &TestDb) -> Option<RoleRow> {
    role_named(db, ADMIN_ROLE_NAME).await
}

async fn owner_of(db: &TestDb, workspace_id: uuid::Uuid) -> Option<uuid::Uuid> {
    OwnerRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT owner_principal_id FROM acta.workspaces WHERE id = $1",
        [workspace_id.into()],
    ))
    .one(db.conn())
    .await
    .expect("query workspace")
    .expect("workspace row")
    .owner_principal_id
}

#[derive(Debug, FromQueryResult)]
struct ProjectRow {
    id: uuid::Uuid,
}

async fn project_named(db: &TestDb, workspace_id: uuid::Uuid, slug: &str) -> uuid::Uuid {
    ProjectRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT id FROM acta.projects WHERE workspace_id = $1 AND slug = $2",
        [workspace_id.into(), slug.into()],
    ))
    .one(db.conn())
    .await
    .expect("query project")
    .expect("project row")
    .id
}

fn workspace_target(workspace_id: uuid::Uuid) -> String {
    format!("acta::workspace::{workspace_id}")
}

fn project_target(project_id: uuid::Uuid) -> String {
    format!("acta::project::{project_id}")
}

fn builtin(row: &GrantRow) -> (&str, Option<&str>, Option<i32>) {
    (
        row.authority_kind.as_str(),
        row.role_name.as_deref(),
        row.role_version,
    )
}

fn user_grant_req(user_id: uuid::Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "user".to_string(),
            id: user_id,
        },
        role: role.to_string(),
    }
}

/// Creating a workspace names its owner, creates the owner role on first
/// use (admin's Acta actions plus transfer and delete, no Custos action),
/// grants it on the workspace and projects the membership.
#[tokio::test]
async fn creating_a_workspace_writes_owner_metadata_role_grant_and_projection() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, user) = login_user(&server, &db, "dual-creator").await;

    assert!(
        owner_role(&db).await.is_none(),
        "no owner role before first use"
    );

    let workspace = client
        .acta()
        .create_workspace("Dual Write")
        .await
        .expect("create workspace");
    assert_eq!(workspace.slug, "dual-write", "V1 response unchanged");

    assert_eq!(owner_of(&db, workspace.id).await, Some(user.id.0));

    let role = owner_role(&db)
        .await
        .expect("owner role created on first use");
    assert!(
        role.actions
            .iter()
            .all(|action| action.starts_with("acta::"))
    );
    for expected in ["acta::workspace::transfer", "acta::workspace::delete"] {
        assert!(
            role.actions.iter().any(|a| a == expected),
            "missing {expected}"
        );
    }

    let grants = grants_on(&db, &workspace_target(workspace.id)).await;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].subject_kind, "principal");
    assert_eq!(grants[0].subject_principal_id, Some(user.id.0));
    assert_eq!(grants[0].authority_kind, "custom");
    assert_eq!(grants[0].role_id, Some(role.id));
    assert_eq!(grants[0].created_by, user.id.0);

    let general = project_named(&db, workspace.id, "general").await;
    let grants = grants_on(&db, &project_target(general)).await;
    assert_eq!(
        grants.len(),
        1,
        "the seeded General project carries its members visibility grant: {grants:?}"
    );
    assert_eq!(grants[0].subject_kind, "principal_set");
    assert_eq!(
        grants[0].subject_principal_set,
        Some(format!("acta::workspace::{}::members", workspace.id))
    );
    assert_eq!(builtin(&grants[0]), ("builtin", Some("editor"), Some(1)));
    assert_eq!(grants[0].created_by, user.id.0);

    assert_eq!(
        members_of(&db, workspace.id).await,
        vec![MemberRow {
            principal_id: user.id.0,
            role: "owner".to_string(),
            source: "membership".to_string(),
        }]
    );

    let second = client
        .acta()
        .create_workspace("Dual Write Two")
        .await
        .expect("second workspace");
    let again = owner_role(&db).await.expect("owner role");
    assert_eq!(
        again.id, role.id,
        "the owner role is created once per product"
    );
    assert_eq!(grants_on(&db, &workspace_target(second.id)).await.len(), 1);

    db.teardown().await;
}

/// Adding, re-roling and removing a member replaces the principal's
/// workspace role grant, keeps the projection in step and moves the owner
/// metadata to a remaining owner when the named owner leaves.
#[tokio::test]
async fn membership_writes_replace_the_role_grant_and_the_projection() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner_client, ws, owner) = login_user_with_workspace(&server, &db, "dual-owner").await;
    let (_, member) = login_user(&server, &db, "dual-member").await;
    let target = workspace_target(ws.id.0);

    assert_eq!(
        owner_of(&db, ws.id.0).await,
        None,
        "a workspace seeded through the repository carries no owner metadata"
    );

    let added = owner_client
        .acta()
        .add_member(&ws.slug, member.id.0, "admin")
        .await
        .expect("add admin");
    assert_eq!(
        added.role.as_deref(),
        Some("admin"),
        "V1 response unchanged"
    );

    let admin = admin_role(&db)
        .await
        .expect("admin role created on first use");
    assert!(
        admin
            .actions
            .iter()
            .all(|action| action.starts_with("acta::")),
        "the admin membership role carries no Custos action"
    );
    assert!(
        !admin
            .actions
            .iter()
            .any(|action| action == "acta::workspace::transfer"
                || action == "acta::workspace::delete"),
        "transfer and delete stay owner-only"
    );
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].subject_principal_id, Some(member.id.0));
    assert_eq!(grants[0].authority_kind, "custom");
    assert_eq!(grants[0].role_id, Some(admin.id));
    assert_eq!(grants[0].created_by, owner.id.0);
    assert_eq!(
        members_of(&db, ws.id.0).await,
        vec![MemberRow {
            principal_id: member.id.0,
            role: "admin".to_string(),
            source: "membership".to_string(),
        }]
    );

    owner_client
        .acta()
        .update_member_role(&ws.slug, member.id.0, "member")
        .await
        .expect("demote to member");
    assert!(
        grants_on(&db, &target).await.is_empty(),
        "a plain member holds no workspace role grant"
    );
    assert_eq!(members_of(&db, ws.id.0).await[0].role, "member");

    owner_client
        .acta()
        .update_member_role(&ws.slug, member.id.0, "owner")
        .await
        .expect("promote to owner");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].authority_kind, "custom");
    assert_eq!(
        grants[0].role_id,
        Some(owner_role(&db).await.expect("owner role").id)
    );
    assert_eq!(owner_of(&db, ws.id.0).await, Some(member.id.0));
    assert_eq!(members_of(&db, ws.id.0).await[0].role, "owner");

    owner_client
        .acta()
        .remove_member(&ws.slug, member.id.0)
        .await
        .expect("remove member");
    assert!(grants_on(&db, &target).await.is_empty());
    assert!(members_of(&db, ws.id.0).await.is_empty());
    assert_eq!(
        owner_of(&db, ws.id.0).await,
        Some(owner.id.0),
        "the owner metadata falls back to the oldest remaining owner membership"
    );

    db.teardown().await;
}

/// A project's creator grant, its visibility and every share made on it
/// are mirrored as V2 grants on the project reference and revoked with
/// their V1 twin.
#[tokio::test]
async fn project_grants_and_visibility_are_mirrored_and_revoked() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner_client, ws, owner) = login_user_with_workspace(&server, &db, "dual-sharer").await;
    let (_, viewer) = login_user(&server, &db, "dual-viewer").await;
    let members_set = format!("acta::workspace::{}::members", ws.id.0);
    owner_client
        .acta()
        .add_member(&ws.slug, viewer.id.0, "member")
        .await
        .expect("a grantee must be a workspace member");

    let project = owner_client
        .acta()
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Dual Project".to_string(),
                slug: "dual-project".to_string(),
                task_prefix: "DUAL".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    assert_eq!(project.visibility, "workspace", "V1 response unchanged");
    let target = project_target(project.id);

    let grants = grants_on(&db, &target).await;
    assert_eq!(
        grants.len(),
        2,
        "creator grant and members visibility grant"
    );
    assert_eq!(grants[0].subject_principal_id, Some(owner.id.0));
    assert_eq!(builtin(&grants[0]), ("builtin", Some("admin"), Some(1)));
    assert_eq!(grants[1].subject_kind, "principal_set");
    assert_eq!(
        grants[1].subject_principal_set.as_deref(),
        Some(members_set.as_str())
    );
    assert_eq!(builtin(&grants[1]), ("builtin", Some("editor"), Some(1)));

    let shared = owner_client
        .custos()
        .create_project_grant(
            &ws.slug,
            &project.slug,
            user_grant_req(viewer.id.0, "viewer"),
        )
        .await
        .expect("share with viewer");
    assert_eq!(shared.role, "viewer", "V1 response unchanged");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 3);
    assert_eq!(grants[2].subject_principal_id, Some(viewer.id.0));
    assert_eq!(builtin(&grants[2]), ("builtin", Some("viewer"), Some(1)));
    assert_eq!(grants[2].created_by, owner.id.0);

    owner_client
        .custos()
        .create_project_grant(
            &ws.slug,
            &project.slug,
            user_grant_req(viewer.id.0, "editor"),
        )
        .await
        .expect("re-share as editor");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 3, "a repeated V1 upsert replaces the V2 row");
    assert_eq!(builtin(&grants[2]), ("builtin", Some("editor"), Some(1)));

    owner_client
        .acta()
        .update_project(
            &ws.slug,
            &project.slug,
            UpdateProjectRequest {
                name: None,
                visibility: Some("private".to_string()),
                visibility_role: None,
                task_prefix: None,
            },
        )
        .await
        .expect("make private");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 2, "private revokes the members set grant");
    assert!(grants.iter().all(|g| g.subject_kind == "principal"));

    owner_client
        .acta()
        .update_project(
            &ws.slug,
            &project.slug,
            UpdateProjectRequest {
                name: None,
                visibility: Some("workspace".to_string()),
                visibility_role: Some("viewer".to_string()),
                task_prefix: None,
            },
        )
        .await
        .expect("workspace viewer visibility");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 3);
    let set_grant = grants
        .iter()
        .find(|g| g.subject_kind == "principal_set")
        .expect("members set grant");
    assert_eq!(builtin(set_grant), ("builtin", Some("viewer"), Some(1)));

    owner_client
        .custos()
        .delete_project_grant(&ws.slug, &project.slug, shared.id)
        .await
        .expect("revoke share");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 2);
    assert!(
        grants
            .iter()
            .all(|g| g.subject_principal_id != Some(viewer.id.0))
    );

    db.teardown().await;
}

/// An admin membership and a share of the workspace to the same principal
/// are disjoint V2 rows: the membership is the custom role
/// `acta:workspace-admin`, the share is a built-in role, so creating and
/// deleting the share never touches the membership grant.
#[tokio::test]
async fn a_workspace_share_never_touches_the_admin_membership_grant() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner_client, ws, _owner) =
        login_user_with_workspace(&server, &db, "disjoint-owner").await;
    let (_, user) = login_user(&server, &db, "disjoint-admin").await;
    let target = workspace_target(ws.id.0);

    owner_client
        .acta()
        .add_member(&ws.slug, user.id.0, "admin")
        .await
        .expect("add admin");
    let admin = admin_role(&db).await.expect("admin role");
    let membership_grant = |rows: &[GrantRow]| {
        rows.iter()
            .filter(|row| row.subject_principal_id == Some(user.id.0))
            .filter(|row| row.authority_kind == "custom" && row.role_id == Some(admin.id))
            .count()
    };
    assert_eq!(membership_grant(&grants_on(&db, &target).await), 1);

    let share = owner_client
        .custos()
        .create_workspace_grant(&ws.slug, user_grant_req(user.id.0, "viewer"))
        .await
        .expect("share the workspace as viewer");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 2, "membership grant and share: {grants:?}");
    assert_eq!(
        membership_grant(&grants),
        1,
        "the share left the membership"
    );
    let shared = grants
        .iter()
        .find(|row| row.authority_kind == "builtin")
        .expect("the share is a built-in grant");
    assert_eq!(shared.subject_principal_id, Some(user.id.0));
    assert_eq!(builtin(shared), ("builtin", Some("viewer"), Some(1)));

    owner_client
        .custos()
        .delete_workspace_grant(&ws.slug, share.id)
        .await
        .expect("delete the share");
    let grants = grants_on(&db, &target).await;
    assert_eq!(
        grants.len(),
        1,
        "only the membership grant is left: {grants:?}"
    );
    assert_eq!(membership_grant(&grants), 1);

    db.teardown().await;
}

struct HangingProvider;

#[async_trait]
impl ResourceProvider for HangingProvider {
    async fn validate_ref(&self, _resource: &ResourceRef) -> Result<bool, CapabilityError> {
        std::future::pending().await
    }

    async fn path_of(&self, _resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        std::future::pending().await
    }

    async fn ancestors(
        &self,
        _resource: &ResourceRef,
    ) -> Result<Vec<ResourceRef>, CapabilityError> {
        std::future::pending().await
    }

    async fn members_of(
        &self,
        _set: &PrincipalSetId,
    ) -> Result<Vec<atlas_core::ids::PrincipalId>, CapabilityError> {
        std::future::pending().await
    }

    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
        std::future::pending().await
    }

    async fn resource_facts(
        &self,
        _resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        std::future::pending().await
    }
}

/// The audit-mode delegation check runs after the response: an evaluation
/// that never answers (a hanging provider with a long provider timeout)
/// does not delay the membership write, and its own timeout counts it as
/// `unavailable`.
#[tokio::test]
async fn a_hanging_delegation_check_does_not_delay_the_write() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let settings = AuthorizationSettings {
        catalog: validation_catalog(&state.registry).expect("catalog"),
        deny_mode: DenyMode::Disabled,
        provider_timeout: Duration::from_secs(60),
    };
    let hanging = AuthorizationService::new(
        ProviderSet::new()
            .with("acta", Arc::new(HangingProvider))
            .with("custos", Arc::new(HangingProvider)),
        PgAuthorizationFactsStore {
            conn: db.conn().clone(),
        },
        PgGroupMembershipSource {
            conn: db.conn().clone(),
        },
        Arc::new(TokioSleeper),
        settings,
    );
    let recorder = PrometheusBuilder::new().build_recorder();
    let metrics = recorder.handle();
    let _recorder = metrics::set_default_local_recorder(&recorder);
    let server =
        TestServer::spawn_with_state(state.with_authorization_service(Arc::new(hanging))).await;
    let (owner_client, ws, _owner) = login_user_with_workspace(&server, &db, "prompt-owner").await;
    let (_, user) = login_user(&server, &db, "prompt-admin").await;

    let started = std::time::Instant::now();
    let added = tokio::time::timeout(
        Duration::from_secs(10),
        owner_client.acta().add_member(&ws.slug, user.id.0, "admin"),
    )
    .await
    .expect("the write must not wait for the delegation check")
    .expect("add admin");
    let elapsed = started.elapsed();

    assert_eq!(added.role.as_deref(), Some("admin"));
    assert!(
        elapsed < Duration::from_secs(2),
        "the response must not wait for the audit check, took {elapsed:?}"
    );
    assert_eq!(grants_on(&db, &workspace_target(ws.id.0)).await.len(), 1);

    let unavailable =
        format!("{WOULD_REFUSE_TOTAL}{{operation=\"add_member\",reason=\"unavailable\"}} 1");
    let mut rendered = metrics.render();
    for _ in 0..40 {
        if rendered.contains(&unavailable) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        rendered = metrics.render();
    }
    assert!(
        rendered.contains(&unavailable),
        "the timed-out check is counted as unavailable, got:\n{rendered}"
    );

    db.teardown().await;
}

/// A V1 share of `admin` on the workspace to a principal that is also an
/// admin member is a second, built-in row beside the membership's custom
/// role; deleting either leaves the other.
#[tokio::test]
async fn an_admin_share_coexists_with_an_admin_membership() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner_client, ws, _owner) = login_user_with_workspace(&server, &db, "coexist-owner").await;
    let (_, user) = login_user(&server, &db, "coexist-admin").await;
    let target = workspace_target(ws.id.0);

    owner_client
        .acta()
        .add_member(&ws.slug, user.id.0, "admin")
        .await
        .expect("add admin");
    let admin = admin_role(&db).await.expect("admin role");
    let share = owner_client
        .custos()
        .create_workspace_grant(&ws.slug, user_grant_req(user.id.0, "admin"))
        .await
        .expect("share the workspace as admin");

    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 2, "{grants:?}");
    assert!(
        grants
            .iter()
            .all(|g| g.subject_principal_id == Some(user.id.0))
    );
    assert!(
        grants
            .iter()
            .any(|g| g.authority_kind == "custom" && g.role_id == Some(admin.id))
    );
    assert!(
        grants
            .iter()
            .any(|g| builtin(g) == ("builtin", Some("admin"), Some(1)))
    );

    owner_client
        .acta()
        .remove_member(&ws.slug, user.id.0)
        .await
        .expect("remove the membership");
    let grants = grants_on(&db, &target).await;
    assert_eq!(
        grants.len(),
        1,
        "the share outlives the membership: {grants:?}"
    );
    assert_eq!(builtin(&grants[0]), ("builtin", Some("admin"), Some(1)));

    owner_client
        .custos()
        .delete_workspace_grant(&ws.slug, share.id)
        .await
        .expect("delete the share");
    assert!(grants_on(&db, &target).await.is_empty());

    db.teardown().await;
}

/// Re-roling a member who also holds a share on the workspace replaces the
/// membership grant only; the share stays as it was.
#[tokio::test]
async fn re_roling_a_member_keeps_their_share() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner_client, ws, _owner) = login_user_with_workspace(&server, &db, "rerole-owner").await;
    let (_, user) = login_user(&server, &db, "rerole-member").await;
    let target = workspace_target(ws.id.0);

    owner_client
        .acta()
        .add_member(&ws.slug, user.id.0, "member")
        .await
        .expect("add member");
    owner_client
        .custos()
        .create_workspace_grant(&ws.slug, user_grant_req(user.id.0, "editor"))
        .await
        .expect("share the workspace as editor");
    let only_share = |rows: &[GrantRow]| {
        rows.iter()
            .filter(|g| builtin(g) == ("builtin", Some("editor"), Some(1)))
            .count()
    };
    assert_eq!(grants_on(&db, &target).await.len(), 1);

    owner_client
        .acta()
        .update_member_role(&ws.slug, user.id.0, "admin")
        .await
        .expect("promote to admin");
    let admin = admin_role(&db).await.expect("admin role");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 2, "{grants:?}");
    assert_eq!(only_share(&grants), 1);
    assert!(grants.iter().any(|g| g.role_id == Some(admin.id)));

    owner_client
        .acta()
        .update_member_role(&ws.slug, user.id.0, "owner")
        .await
        .expect("promote to owner");
    let owner = owner_role(&db).await.expect("owner role");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 2, "{grants:?}");
    assert_eq!(only_share(&grants), 1);
    assert!(grants.iter().any(|g| g.role_id == Some(owner.id)));
    assert!(!grants.iter().any(|g| g.role_id == Some(admin.id)));

    owner_client
        .acta()
        .update_member_role(&ws.slug, user.id.0, "member")
        .await
        .expect("demote to member");
    let grants = grants_on(&db, &target).await;
    assert_eq!(grants.len(), 1, "only the share is left: {grants:?}");
    assert_eq!(only_share(&grants), 1);

    db.teardown().await;
}
