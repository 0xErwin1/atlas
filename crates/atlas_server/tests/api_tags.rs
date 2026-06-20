#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::tags::CreateTagRequest;
use atlas_client::ClientError;

// ---------------------------------------------------------------------------
// Create + list happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_tag_returns_201_and_appears_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-crud-1").await;

    let tag = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "Epic".to_string(),
            },
        )
        .await
        .expect("create tag");

    assert_eq!(tag.name, "Epic");
    assert_eq!(tag.workspace_id, ws.id.0);

    let listed = client.list_tags(&ws.slug).await.expect("list tags");

    assert_eq!(listed.len(), 1, "the created tag must appear in the list");
    assert_eq!(listed[0].id, tag.id);
    assert_eq!(listed[0].name, "Epic");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Idempotency by case-insensitive name
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_tag_is_idempotent_by_case_insensitive_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-idem-1").await;

    let first = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "Epic".to_string(),
            },
        )
        .await
        .expect("create tag");

    let same = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "Epic".to_string(),
            },
        )
        .await
        .expect("create same tag");

    let different_case = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "epic".to_string(),
            },
        )
        .await
        .expect("create different-case tag");

    assert_eq!(same.id, first.id, "same name must return the same tag");
    assert_eq!(
        different_case.id, first.id,
        "case-insensitive name must return the same tag"
    );

    let listed = client.list_tags(&ws.slug).await.expect("list tags");

    assert_eq!(listed.len(), 1, "idempotent creates must not duplicate");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Listing order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tags_is_sorted_by_name_ascending() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-sort-1").await;

    for name in ["Gamma", "alpha", "Beta"] {
        client
            .create_tag(
                &ws.slug,
                CreateTagRequest {
                    name: name.to_string(),
                },
            )
            .await
            .expect("create tag");
    }

    let listed = client.list_tags(&ws.slug).await.expect("list tags");

    let names: Vec<String> = listed.into_iter().map(|t| t.name).collect();
    assert_eq!(names, vec!["alpha", "Beta", "Gamma"]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Cross-tenant isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tags_are_isolated_per_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "tag-tenant-a").await;
    let (client_b, ws_b, _) =
        support::login_user_with_workspace(&server, &db, "tag-tenant-b").await;

    client_a
        .create_tag(
            &ws_a.slug,
            CreateTagRequest {
                name: "OnlyInA".to_string(),
            },
        )
        .await
        .expect("create tag in A");

    let listed_b = client_b
        .list_tags(&ws_b.slug)
        .await
        .expect("list tags in B");

    assert!(
        listed_b.is_empty(),
        "workspace B must not see workspace A's tags"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_tag_rejects_blank_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-blank-1").await;

    let result = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "   ".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "blank name must be rejected as invalid input, got {result:?}"
    );

    db.teardown().await;
}
