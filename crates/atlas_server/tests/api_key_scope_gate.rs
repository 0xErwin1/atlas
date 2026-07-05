#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{ApiKeyType, NewApiKey},
    permissions::{Capability, CapabilityAction, CapabilityFamily},
};
use atlas_server::{
    authz::authorized::enforce_api_key_scope, error::ApiError, persistence::repos::ApiKeyRepo,
};
use support::{TestDb, seed_workspace};

async fn create_key_with_scopes(db: &TestDb, scopes: Vec<Capability>) -> uuid::Uuid {
    let (ws, user) = seed_workspace(db, &format!("scope-gate-{}", uuid::Uuid::now_v7())).await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "scope-gate-key".to_string(),
                token_hash: format!("hash-{}", uuid::Uuid::now_v7()),
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes,
            },
        )
        .await
        .expect("create scoped api key");

    key.id.0
}

#[tokio::test]
async fn allows_when_key_holds_the_required_capability() {
    let db = TestDb::create().await.expect("TestDb::create");

    let key_id = create_key_with_scopes(
        &db,
        vec![Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        }],
    )
    .await;

    let result = enforce_api_key_scope(
        db.conn(),
        atlas_domain::ids::ApiKeyId(key_id),
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        },
    )
    .await;

    assert!(result.is_ok(), "expected allow, got {result:?}");

    db.teardown().await;
}

#[tokio::test]
async fn denies_when_key_lacks_the_required_capability() {
    let db = TestDb::create().await.expect("TestDb::create");

    let key_id = create_key_with_scopes(
        &db,
        vec![Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        }],
    )
    .await;

    let result = enforce_api_key_scope(
        db.conn(),
        atlas_domain::ids::ApiKeyId(key_id),
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Update,
        },
    )
    .await;

    assert!(matches!(result, Err(ApiError::Forbidden { .. })));

    db.teardown().await;
}

#[tokio::test]
async fn denies_when_key_has_zero_scopes() {
    let db = TestDb::create().await.expect("TestDb::create");

    let key_id = create_key_with_scopes(&db, vec![]).await;

    let result = enforce_api_key_scope(
        db.conn(),
        atlas_domain::ids::ApiKeyId(key_id),
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Delete,
        },
    )
    .await;

    assert!(matches!(result, Err(ApiError::Forbidden { .. })));

    db.teardown().await;
}

#[tokio::test]
async fn allows_every_capability_when_key_holds_all_twenty() {
    let db = TestDb::create().await.expect("TestDb::create");

    let key_id = create_key_with_scopes(&db, Capability::ALL.to_vec()).await;

    for cap in Capability::ALL {
        let result =
            enforce_api_key_scope(db.conn(), atlas_domain::ids::ApiKeyId(key_id), cap).await;
        assert!(result.is_ok(), "expected allow for {cap:?}, got {result:?}");
    }

    db.teardown().await;
}
