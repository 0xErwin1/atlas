#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization test for `PgAutomationRuleRepo`/`PgWebhookSubscriptionRepo`/
//! `PgWebhookDeliveryRepo`'s current query shapes, ported from
//! `atlas_server::persistence::repos::{automation_rule, webhook_subscription,
//! webhook_delivery}` before that code moves into this crate (S4 PR8, T8.1).
//! Must keep passing unmodified once the move lands (T8.4). All three are
//! inherent-impl-only structs (no `atlas_acta::ports` trait), so this test
//! calls their associated functions directly against a connection, the same
//! shape the pre-move code used.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId};
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta_postgres::repos::automation_rule::PgAutomationRuleRepo;
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_acta_postgres::repos::webhook_delivery::PgWebhookDeliveryRepo;
use atlas_acta_postgres::repos::webhook_subscription::PgWebhookSubscriptionRepo;
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

async fn seed_workspace(db: &TestDb, slug: &str) -> WorkspaceId {
    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: slug.to_string(),
            slug: slug.to_string(),
        })
        .await
        .expect("workspace must be created");
    workspace_id
}

// `webhook_subscriptions.created_by_user_id` is a foreign key into
// `custos.users`: seed a real user rather than a random UUID, matching PR6's
// `identity_workspace_repos_characterization.rs` seed pattern.
async fn seed_user(db: &TestDb, username: &str) -> UserId {
    let repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let user = repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user must be created");
    user.id
}

#[tokio::test]
async fn automation_rule_repo_create_list_get_patch_and_soft_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "automation-rule-test").await;
    let user_id = seed_user(&db, "automation-rule-owner").await;

    let created = PgAutomationRuleRepo::create(
        db.conn(),
        workspace_id.0,
        "Notify on external event".to_string(),
        "external.issue.created".to_string(),
        None,
        None,
        "create_task".to_string(),
        serde_json::json!({ "title": "New task from event" }),
        user_id.0,
    )
    .await
    .expect("automation rule must be created");
    assert_eq!(created.name, "Notify on external event");

    let fetched = PgAutomationRuleRepo::get(db.conn(), workspace_id.0, created.id)
        .await
        .expect("get must not error")
        .expect("automation rule must exist");
    assert_eq!(fetched.id, created.id);

    let listed = PgAutomationRuleRepo::list(db.conn(), workspace_id.0, None, 10)
        .await
        .expect("list must not error");
    assert_eq!(listed.len(), 1);

    let patched = PgAutomationRuleRepo::patch(
        db.conn(),
        workspace_id.0,
        created.id,
        atlas_acta_postgres::repos::automation_rule::AutomationRulePatch {
            name: Some("Renamed rule".to_string()),
            is_active: Some(false),
            trigger_filter: None,
            action_params: None,
        },
    )
    .await
    .expect("patch must succeed");
    assert_eq!(patched.name, "Renamed rule");
    assert!(!patched.is_active);

    PgAutomationRuleRepo::soft_delete(db.conn(), workspace_id.0, created.id)
        .await
        .expect("soft_delete must succeed");
    let listed_after_delete = PgAutomationRuleRepo::list(db.conn(), workspace_id.0, None, 10)
        .await
        .expect("list must not error");
    assert!(listed_after_delete.is_empty());

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn webhook_subscription_and_delivery_repos_create_list_and_update_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "webhook-repo-test").await;
    let user_id = seed_user(&db, "webhook-subscription-owner").await;
    let actor = Actor::User(UserAttributionId(user_id.0));

    let subscription = PgWebhookSubscriptionRepo::create(
        db.conn(),
        workspace_id.0,
        "https://example.com/webhook".to_string(),
        vec!["task.created".to_string()],
        "workspace".to_string(),
        None,
        vec![0u8; 16],
        vec![0u8; 12],
        Some("primary".to_string()),
        &actor,
    )
    .await
    .expect("subscription must be created");
    assert_eq!(subscription.target_url, "https://example.com/webhook");

    let fetched = PgWebhookSubscriptionRepo::get_by_id(db.conn(), workspace_id.0, subscription.id)
        .await
        .expect("get_by_id must not error")
        .expect("subscription must exist");
    assert_eq!(fetched.id, subscription.id);

    let listed = PgWebhookSubscriptionRepo::list_active(db.conn(), workspace_id.0, None, 10)
        .await
        .expect("list_active must not error");
    assert_eq!(listed.len(), 1);

    let updated = PgWebhookSubscriptionRepo::update(
        db.conn(),
        workspace_id.0,
        subscription.id,
        atlas_acta_postgres::repos::webhook_subscription::WebhookSubscriptionPatch {
            target_url: Some("https://example.com/webhook-v2".to_string()),
            event_types: None,
            scope_type: None,
            scope_id: None,
            encrypted_secret: None,
            secret_nonce: None,
            is_active: Some(false),
            label: None,
        },
    )
    .await
    .expect("update must succeed");
    assert_eq!(updated.target_url, "https://example.com/webhook-v2");
    assert!(!updated.is_active);

    // `events_outbox`/`webhook_delivery_log` need a real outbox event row for
    // the delivery log's FK: seed one directly, mirroring the pre-move test
    // fixture style used elsewhere in this crate for FK-only seed rows.
    let outbox_event_id = Uuid::now_v7();
    let aggregate_id = Uuid::now_v7();
    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        &format!(
            "INSERT INTO events_outbox \
             (id, workspace_id, event_type, event_version, aggregate_type, aggregate_id, \
              payload) \
             VALUES ('{outbox_event_id}', '{}', 'task.created', 1, 'task', '{aggregate_id}', \
                     '{{}}')",
            workspace_id.0
        ),
    )
    .await
    .expect("seed outbox event");

    let logged = PgWebhookDeliveryRepo::append_log(
        db.conn(),
        workspace_id.0,
        subscription.id,
        outbox_event_id,
        1,
        "success".to_string(),
        Some(200),
        Some("ok".to_string()),
        None,
        Some(42),
    )
    .await
    .expect("append_log must succeed");
    assert_eq!(logged.outcome, "success");

    let logs = PgWebhookDeliveryRepo::list_for_subscription(
        db.conn(),
        workspace_id.0,
        subscription.id,
        None,
        10,
    )
    .await
    .expect("list_for_subscription must not error");
    assert_eq!(logs.len(), 1);

    let succeeded =
        PgWebhookDeliveryRepo::succeeded_subscription_ids_for_event(db.conn(), outbox_event_id)
            .await
            .expect("succeeded_subscription_ids_for_event must not error");
    assert_eq!(succeeded, vec![subscription.id]);

    db.teardown().await.expect("teardown must succeed");
}
