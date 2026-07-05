#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    ChangePasswordRequest, CreateUserRequest, LoginRequest, MeResponse, ServerMetaDto,
    UpdateMeRequest, UserDto,
};
use atlas_client::AtlasClient;
use atlas_domain::{Actor, WorkspaceCtx};
use atlas_server::persistence::repos::{ApiKeyRepo, NewApiKey};
use support::{TestDb, TestServer, login_root_user, login_user_with_workspace};

#[tokio::test]
async fn list_users_returns_all_users_for_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = login_root_user(&server, &db).await;
    let (_member, _, member_user) =
        login_user_with_workspace(&server, &db, "settings-member").await;

    let users: Vec<UserDto> = root.list_users().await.expect("list_users");

    assert!(
        users.iter().any(|u| u.id == member_user.id.0),
        "the created member should appear in the user list"
    );
    assert!(
        users.iter().any(|u| u.is_root),
        "the calling root user should appear in the user list"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_users_includes_disabled_users() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = login_root_user(&server, &db).await;
    let (_victim, _, victim) = login_user_with_workspace(&server, &db, "settings-disabled").await;

    root.disable_user(victim.id.0).await.expect("disable_user");

    let users: Vec<UserDto> = root.list_users().await.expect("list_users");

    let listed = users
        .iter()
        .find(|u| u.id == victim.id.0)
        .expect("disabled user must still be listed");
    assert!(
        listed.disabled_at.is_some(),
        "disabled user should carry a disabled_at timestamp"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_users_rejects_non_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (member, _, _) = login_user_with_workspace(&server, &db, "settings-nonadmin").await;

    let err = member.list_users().await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 for non-admin, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_users_rejects_unauthenticated() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let anon = AtlasClient::new(server.base_url().to_string());

    let err = anon.list_users().await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 for unauthenticated, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn change_password_succeeds_and_rotates_credentials() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "settings-rotate").await;

    client
        .change_password(ChangePasswordRequest {
            current_password: "TestPassword1!".to_string(),
            new_password: "BrandNewPass2@".to_string(),
        })
        .await
        .expect("change_password");

    // Old password no longer works.
    let mut old_login = AtlasClient::new(server.base_url().to_string());
    let old_err = old_login
        .login(LoginRequest {
            username: "settings-rotate".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await;
    assert!(
        matches!(old_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "old password should be rejected, got {old_err:?}"
    );

    // New password works.
    let mut new_login = AtlasClient::new(server.base_url().to_string());
    new_login
        .login(LoginRequest {
            username: "settings-rotate".to_string(),
            password: "BrandNewPass2@".to_string(),
        })
        .await
        .expect("login with new password");

    db.teardown().await;
}

#[tokio::test]
async fn change_password_rejects_wrong_current_password() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "settings-wrongpw").await;

    let err = client
        .change_password(ChangePasswordRequest {
            current_password: "NotMyPassword9!".to_string(),
            new_password: "BrandNewPass2@".to_string(),
        })
        .await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 for wrong current password, got {err:?}"
    );

    // Password unchanged: original still works.
    let mut relogin = AtlasClient::new(server.base_url().to_string());
    relogin
        .login(LoginRequest {
            username: "settings-wrongpw".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("original password must still work");

    db.teardown().await;
}

#[tokio::test]
async fn change_password_rejects_api_key_principal() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) = login_user_with_workspace(&server, &db, "settings-agent").await;

    let raw_secret = "atlas_settings_agent_secret_token";
    let token_hash = atlas_server::auth::tokens::hash_token(raw_secret);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    db.api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "settings-bot".to_string(),
                token_hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key");

    let agent = AtlasClient::new(server.base_url().to_string()).with_token(raw_secret);

    let err = agent
        .change_password(ChangePasswordRequest {
            current_password: "irrelevant".to_string(),
            new_password: "irrelevant2".to_string(),
        })
        .await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 for api-key principal, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_user_with_email_persists_and_returns_it() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = login_root_user(&server, &db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "email-ws-owner").await;

    let resp = root
        .create_user(CreateUserRequest {
            username: "email-user".to_string(),
            display_name: "Email User".to_string(),
            email: Some("email-user@example.com".to_string()),
            workspace: ws.slug.clone(),
            role: "member".to_string(),
        })
        .await
        .expect("create_user");

    let created = resp.user;
    assert_eq!(created.email.as_deref(), Some("email-user@example.com"));

    let listed: Vec<UserDto> = root.list_users().await.expect("list_users");
    let found = listed
        .iter()
        .find(|u| u.id == created.id)
        .expect("created user must be listed");
    assert_eq!(found.email.as_deref(), Some("email-user@example.com"));

    db.teardown().await;
}

#[tokio::test]
async fn create_user_without_email_defaults_to_none() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = login_root_user(&server, &db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "no-email-ws-owner").await;

    let resp = root
        .create_user(CreateUserRequest {
            username: "no-email-user".to_string(),
            display_name: "No Email".to_string(),
            email: None,
            workspace: ws.slug.clone(),
            role: "member".to_string(),
        })
        .await
        .expect("create_user");

    assert_eq!(resp.user.email, None);

    db.teardown().await;
}

#[tokio::test]
async fn update_me_updates_email_and_display_name() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "profile-update").await;

    let updated: UserDto = client
        .update_me(UpdateMeRequest {
            email: Some("new@example.com".to_string()),
            display_name: Some("Renamed Person".to_string()),
        })
        .await
        .expect("update_me");

    assert_eq!(updated.email.as_deref(), Some("new@example.com"));
    assert_eq!(updated.display_name, "Renamed Person");

    db.teardown().await;
}

#[tokio::test]
async fn update_me_only_overwrites_provided_fields() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "profile-partial").await;

    client
        .update_me(UpdateMeRequest {
            email: Some("first@example.com".to_string()),
            display_name: None,
        })
        .await
        .expect("update_me email only");

    let after: UserDto = client
        .update_me(UpdateMeRequest {
            email: None,
            display_name: Some("Only Name".to_string()),
        })
        .await
        .expect("update_me name only");

    assert_eq!(
        after.email.as_deref(),
        Some("first@example.com"),
        "email must survive an update that omits it"
    );
    assert_eq!(after.display_name, "Only Name");

    db.teardown().await;
}

#[tokio::test]
async fn me_returns_the_users_email() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "me-email").await;

    client
        .update_me(UpdateMeRequest {
            email: Some("me-email@example.com".to_string()),
            display_name: None,
        })
        .await
        .expect("update_me");

    let me: MeResponse = client.me().await.expect("me");
    assert_eq!(me.email.as_deref(), Some("me-email@example.com"));

    db.teardown().await;
}

#[tokio::test]
async fn update_me_rejects_api_key_principal() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) = login_user_with_workspace(&server, &db, "profile-agent").await;

    let raw_secret = "atlas_profile_agent_secret_token";
    let token_hash = atlas_server::auth::tokens::hash_token(raw_secret);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    db.api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "profile-bot".to_string(),
                token_hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key");

    let agent = AtlasClient::new(server.base_url().to_string()).with_token(raw_secret);

    let err = agent
        .update_me(UpdateMeRequest {
            email: Some("agent@example.com".to_string()),
            display_name: None,
        })
        .await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 for api-key principal, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn reset_password_rotates_credentials_and_revokes_sessions() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = login_root_user(&server, &db).await;
    let (victim, _, victim_user) = login_user_with_workspace(&server, &db, "reset-victim").await;

    // The victim has an active session right now.
    victim
        .me()
        .await
        .expect("victim session active before reset");

    root.reset_user_password(victim_user.id.0, "FreshSecret9!")
        .await
        .expect("reset_user_password");

    // Existing session is revoked.
    let after = victim.me().await;
    assert!(
        matches!(after, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "victim's existing session must be revoked, got {after:?}"
    );

    // Old password no longer logs in.
    let mut old_login = AtlasClient::new(server.base_url().to_string());
    let old_err = old_login
        .login(LoginRequest {
            username: "reset-victim".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await;
    assert!(
        matches!(old_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "old password must be rejected, got {old_err:?}"
    );

    // New password works.
    let mut new_login = AtlasClient::new(server.base_url().to_string());
    new_login
        .login(LoginRequest {
            username: "reset-victim".to_string(),
            password: "FreshSecret9!".to_string(),
        })
        .await
        .expect("login with reset password");

    db.teardown().await;
}

#[tokio::test]
async fn reset_password_rejects_non_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (member, _, _) = login_user_with_workspace(&server, &db, "reset-nonadmin").await;
    let (_other, _, target) = login_user_with_workspace(&server, &db, "reset-target").await;

    let err = member
        .reset_user_password(target.id.0, "WhateverPass1!")
        .await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 for non-admin, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn change_password_revokes_other_sessions() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (session_a, _, _) = login_user_with_workspace(&server, &db, "pw-revoke-sessions").await;

    // Log in a second independent session for the same user.
    let mut session_b = AtlasClient::new(server.base_url().to_string());
    session_b
        .login(LoginRequest {
            username: "pw-revoke-sessions".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("second login");

    // Both sessions are active before the password change.
    session_a
        .me()
        .await
        .expect("session_a active before change");
    session_b
        .me()
        .await
        .expect("session_b active before change");

    // Session A changes the password.
    session_a
        .change_password(ChangePasswordRequest {
            current_password: "TestPassword1!".to_string(),
            new_password: "NewSecurePass2@".to_string(),
        })
        .await
        .expect("change_password");

    // Session B (the other session) must now be revoked.
    let after_b = session_b.me().await;
    assert!(
        matches!(after_b, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "session_b must be revoked after password change, got {after_b:?}"
    );

    // Session A (the calling session) must still be alive.
    session_a
        .me()
        .await
        .expect("session_a must survive its own password change");

    db.teardown().await;
}

#[tokio::test]
async fn meta_returns_the_crate_version() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "meta-reader").await;

    let meta: ServerMetaDto = client.server_meta().await.expect("server_meta");
    assert_eq!(meta.version, env!("CARGO_PKG_VERSION"));

    db.teardown().await;
}

#[tokio::test]
async fn meta_requires_authentication() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let anon = AtlasClient::new(server.base_url().to_string());

    let err = anon.server_meta().await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 for unauthenticated, got {err:?}"
    );

    db.teardown().await;
}
