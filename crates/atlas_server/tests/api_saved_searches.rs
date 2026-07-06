#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    ApiKeyScope, CreateUserApiKeyRequest, InitialGrantRequest,
    saved_searches::{CreateSavedSearchRequest, RenameSavedSearchRequest},
};
use atlas_client::ClientError;

// ---------------------------------------------------------------------------
// Create + list happy path (SS1, SS2, SS16, SS17, SS18, SS24)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_returns_201_and_appears_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-create-1").await;

    let ss = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Open Rust".to_string(),
                query: "status:open tag:rust".to_string(),
            },
        )
        .await
        .expect("create saved search");

    assert_eq!(ss.name, "Open Rust");
    assert_eq!(ss.query, "status:open tag:rust");
    assert_eq!(ss.workspace_id, ws.id.0);

    let listed = client
        .list_saved_searches(&ws.slug)
        .await
        .expect("list saved searches");

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, ss.id);
    assert_eq!(listed[0].name, "Open Rust");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Validation — blank name (SS4)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_rejects_blank_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-blank-name-1").await;

    let result = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "   ".to_string(),
                query: "x".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "blank name must be rejected as 422, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Validation — name too long (SS5)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_rejects_name_over_200_chars() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-long-name-1").await;

    let result = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "a".repeat(201),
                query: "x".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "name > 200 chars must be rejected as 422, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Validation — query too long (SS6)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_rejects_query_over_2000_chars() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-long-query-1").await;

    let result = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Long Query".to_string(),
                query: "q".repeat(2001),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "query > 2000 chars must be rejected as 422, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Empty query is allowed (SS2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_allows_empty_query() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-empty-query-1").await;

    let ss = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Everything".to_string(),
                query: "".to_string(),
            },
        )
        .await
        .expect("empty query must be accepted");

    assert_eq!(ss.query, "");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Duplicate name for same owner returns 409 (SS7)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_rejects_duplicate_name_for_same_owner() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-dup-name-1").await;

    client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Open Rust".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("first create");

    let result = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Open Rust".to_string(),
                query: "y".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "duplicate name must return 409, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Cross-owner: same name, different principals in same workspace (SS7, SS19)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_allows_same_name_for_different_owners() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (user_client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-cross-owner-1").await;

    user_client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Open Rust".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("user creates saved search");

    let key_created = user_client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "test-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![ApiKeyScope::SavedSearchesCreate]),
        })
        .await
        .expect("create api key");

    let key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    let result = key_client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Open Rust".to_string(),
                query: "y".to_string(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "api_key owner must be allowed same name as user owner, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// List: owner-scoped, alphabetical, excludes soft-deleted (SS16, SS17, SS18)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_saved_searches_is_owner_scoped_sorted_and_excludes_deleted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-list-scoped-1").await;

    for name in ["Gamma", "alpha", "Beta"] {
        client
            .create_saved_search(
                &ws.slug,
                CreateSavedSearchRequest {
                    name: name.to_string(),
                    query: "x".to_string(),
                },
            )
            .await
            .expect("create");
    }

    let key_created = client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "other-owner-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![ApiKeyScope::SavedSearchesCreate]),
        })
        .await
        .expect("create api key");

    let key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    key_client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "ZOther".to_string(),
                query: "z".to_string(),
            },
        )
        .await
        .expect("key owner creates");

    let to_delete = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "ToDelete".to_string(),
                query: "d".to_string(),
            },
        )
        .await
        .expect("create to-delete");

    client
        .delete_saved_search(&ws.slug, to_delete.id)
        .await
        .expect("delete");

    let listed = client.list_saved_searches(&ws.slug).await.expect("list");

    let names: Vec<String> = listed.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, vec!["alpha", "Beta", "Gamma"]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Rename happy path (SS9)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rename_saved_search_returns_200_with_new_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-rename-1").await;

    let ss = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Old".to_string(),
                query: "status:open".to_string(),
            },
        )
        .await
        .expect("create");

    let renamed = client
        .rename_saved_search(
            &ws.slug,
            ss.id,
            RenameSavedSearchRequest {
                name: "New".to_string(),
            },
        )
        .await
        .expect("rename");

    assert_eq!(renamed.name, "New");
    assert_eq!(renamed.query, "status:open");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Rename into duplicate name returns 409 (SS11)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rename_saved_search_rejects_duplicate_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-rename-dup-1").await;

    client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Alpha".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create alpha");

    let beta = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Beta".to_string(),
                query: "y".to_string(),
            },
        )
        .await
        .expect("create beta");

    let result = client
        .rename_saved_search(
            &ws.slug,
            beta.id,
            RenameSavedSearchRequest {
                name: "Alpha".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "rename into duplicate must return 409, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Rename non-owned id returns 404 (SS12)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rename_saved_search_returns_404_for_non_owned_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-rename-nonowned-1").await;

    let ss = client_a
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Mine".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create");

    let key_created = client_a
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "intruder-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![ApiKeyScope::SavedSearchesUpdate]),
        })
        .await
        .expect("create api key");

    let client_b = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    let result = client_b
        .rename_saved_search(
            &ws.slug,
            ss.id,
            RenameSavedSearchRequest {
                name: "Stolen".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "rename of non-owned id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Rename missing id returns 404 (SS12)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rename_saved_search_returns_404_for_missing_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-rename-missing-1").await;

    let missing_id = uuid::Uuid::now_v7();
    let result = client
        .rename_saved_search(
            &ws.slug,
            missing_id,
            RenameSavedSearchRequest {
                name: "Ghost".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "rename of missing id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Delete + name reuse (SS13)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_saved_search_returns_204_and_frees_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-delete-1").await;

    let ss = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Open Rust".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create");

    client
        .delete_saved_search(&ws.slug, ss.id)
        .await
        .expect("delete must return 204");

    let listed = client
        .list_saved_searches(&ws.slug)
        .await
        .expect("list after delete");

    assert!(listed.is_empty(), "deleted row must not appear in list");

    client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Open Rust".to_string(),
                query: "y".to_string(),
            },
        )
        .await
        .expect("re-creating with same name must succeed after soft-delete");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Delete non-owned returns 404 (SS14)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_saved_search_returns_404_for_non_owned_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-delete-nonowned-1").await;

    let ss = client_a
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Mine".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create");

    let key_created = client_a
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "intruder-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![ApiKeyScope::SavedSearchesDelete]),
        })
        .await
        .expect("create api key");

    let client_b = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret);

    let result = client_b.delete_saved_search(&ws.slug, ss.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "delete of non-owned id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Delete missing id returns 404 (SS14, SS15)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_saved_search_returns_404_for_missing_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-delete-missing-1").await;

    let missing_id = uuid::Uuid::now_v7();
    let result = client.delete_saved_search(&ws.slug, missing_id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "delete of missing id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Cross-workspace isolation (SS21)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn saved_searches_are_isolated_per_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws_a, _) = support::login_user_with_workspace(&server, &db, "ss-ws-iso-a").await;
    let (client_b, ws_b, _) = support::login_user_with_workspace(&server, &db, "ss-ws-iso-b").await;

    client_a
        .create_saved_search(
            &ws_a.slug,
            CreateSavedSearchRequest {
                name: "OnlyInA".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create in A");

    let listed_b = client_b
        .list_saved_searches(&ws_b.slug)
        .await
        .expect("list in B");

    assert!(
        listed_b.is_empty(),
        "workspace B must not see workspace A's saved searches"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Unauthenticated returns 401 (SS22)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn saved_searches_endpoints_reject_unauthenticated_requests() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner_client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-unauth-1").await;

    let anon = server.client();

    let result = anon.list_saved_searches(&ws.slug).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated list must return 401, got {result:?}"
    );

    let result = anon
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "x".to_string(),
                query: "y".to_string(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated create must return 401, got {result:?}"
    );

    let ss = owner_client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Auth Test".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create for auth test");

    let result = anon
        .rename_saved_search(
            &ws.slug,
            ss.id,
            RenameSavedSearchRequest {
                name: "X".to_string(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated rename must return 401, got {result:?}"
    );

    let result = anon.delete_saved_search(&ws.slug, ss.id).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated delete must return 401, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Non-member returns 404 (SS23)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn saved_searches_return_404_for_non_member() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_, ws, _) = support::login_user_with_workspace(&server, &db, "ss-nonmember-ws-1").await;
    let (outsider, _, _) =
        support::login_user_with_workspace(&server, &db, "ss-nonmember-user-1").await;

    let result = outsider.list_saved_searches(&ws.slug).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member list must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Rename validation — blank / too-long name (SS10)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rename_saved_search_rejects_blank_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-rename-blank-1").await;

    let ss = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Before".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create");

    let result = client
        .rename_saved_search(
            &ws.slug,
            ss.id,
            RenameSavedSearchRequest {
                name: "   ".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "blank rename name must be rejected as 422, got {result:?}"
    );

    let listed = client.list_saved_searches(&ws.slug).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].name, "Before",
        "row must be unchanged after rejected rename"
    );

    db.teardown().await;
}

#[tokio::test]
async fn rename_saved_search_rejects_name_over_200_chars() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-rename-long-1").await;

    let ss = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Before".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create");

    let result = client
        .rename_saved_search(
            &ws.slug,
            ss.id,
            RenameSavedSearchRequest {
                name: "a".repeat(201),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "name > 200 chars must be rejected as 422, got {result:?}"
    );

    let listed = client.list_saved_searches(&ws.slug).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].name, "Before",
        "row must be unchanged after rejected rename"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Double-delete: soft-deleted row returns 404 on second attempt (SS15)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_saved_search_returns_404_on_second_delete() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ss-double-delete-1").await;

    let ss = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "ToDelete".to_string(),
                query: "x".to_string(),
            },
        )
        .await
        .expect("create");

    client
        .delete_saved_search(&ws.slug, ss.id)
        .await
        .expect("first delete must return 204");

    let result = client.delete_saved_search(&ws.slug, ss.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "second delete of soft-deleted id must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Per-owner cap of 100 (SS8)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_saved_search_rejects_over_cap() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ss-cap-1").await;

    for i in 0..100 {
        client
            .create_saved_search(
                &ws.slug,
                CreateSavedSearchRequest {
                    name: format!("Search {i}"),
                    query: "x".to_string(),
                },
            )
            .await
            .expect("create within cap");
    }

    let result = client
        .create_saved_search(
            &ws.slug,
            CreateSavedSearchRequest {
                name: "Over Cap".to_string(),
                query: "x".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "101st saved search must return 422, got {result:?}"
    );

    db.teardown().await;
}
