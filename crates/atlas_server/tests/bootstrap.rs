#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_server::persistence::{
    bootstrap::{BootstrapConfig, run_bootstrap, run_dev_seed},
    repos::{ProjectRepo, UserRepo, WorkspaceRepo},
};

#[tokio::test]
async fn bootstrap_fail_fast_when_no_root_password() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let cfg = BootstrapConfig {
        root_password: None,
    };
    let result = run_bootstrap(&cfg, db.conn()).await;
    assert!(result.is_err(), "must fail fast without root_password");
    let msg = result.expect_err("error").to_string();
    assert!(
        msg.contains("ATLAS_ROOT_PASSWORD"),
        "error must name the missing variable"
    );
    db.teardown().await;
}

#[tokio::test]
async fn bootstrap_creates_root_and_workspace_idempotently() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let cfg = BootstrapConfig {
        root_password: Some("s3cret".into()),
    };
    run_bootstrap(&cfg, db.conn())
        .await
        .expect("first bootstrap");
    run_bootstrap(&cfg, db.conn())
        .await
        .expect("second bootstrap must be idempotent");

    let repo = db.user_repo();
    let root = repo
        .find_root()
        .await
        .expect("find root")
        .expect("root must exist");
    assert!(root.is_root);

    db.teardown().await;
}

#[tokio::test]
async fn bootstrap_password_not_stored_as_plaintext() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let cfg = BootstrapConfig {
        root_password: Some("my-secret-pw".into()),
    };
    run_bootstrap(&cfg, db.conn()).await.expect("bootstrap");

    let repo = db.user_repo();
    let root = repo
        .find_root()
        .await
        .expect("find root")
        .expect("root must exist");

    assert_ne!(
        root.password_hash, "my-secret-pw",
        "password must be hashed"
    );
    assert!(
        root.password_hash.starts_with("$argon2"),
        "must be argon2id PHC string"
    );
    db.teardown().await;
}

#[tokio::test]
async fn dev_seeder_is_idempotent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let cfg = BootstrapConfig {
        root_password: Some("test-pw".into()),
    };

    run_dev_seed(&cfg, db.conn()).await.expect("first seed run");
    run_dev_seed(&cfg, db.conn())
        .await
        .expect("second seed run must be idempotent");

    let project_repo = db.project_repo();
    let user_repo = db.user_repo();

    let root = user_repo
        .find_root()
        .await
        .expect("find root")
        .expect("root must exist");

    let root_ws_id = atlas_domain::ids::WorkspaceId::new();
    let _ = root_ws_id;

    let ws_repo = db.workspace_repo();
    let workspaces = ws_repo
        .list_for_user(root.id)
        .await
        .expect("list workspaces for root");
    assert_eq!(
        workspaces.len(),
        1,
        "exactly one workspace after double seed"
    );

    let ctx = support::ctx(workspaces.first().expect("workspace"), &root);
    let projects = project_repo.list(&ctx).await.expect("list projects");
    assert_eq!(projects.len(), 1, "exactly one project after double seed");

    db.teardown().await;
}
