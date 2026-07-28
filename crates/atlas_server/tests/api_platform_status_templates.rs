#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateUserApiKeyRequest,
    status_templates::{
        CreateStatusTemplateRequest, PlatformStatusTemplateDto, UpdateStatusTemplateRequest,
    },
};
use atlas_client::{AtlasClient, ClientError};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn create_default(
    client: &AtlasClient,
    name: &str,
    color: Option<&str>,
) -> PlatformStatusTemplateDto {
    client
        .create_platform_status_template(CreateStatusTemplateRequest {
            name: name.to_string(),
            color: color.map(str::to_string),
            before: None,
            after: None,
        })
        .await
        .expect("create platform status template")
}

/// Names of the status templates a workspace owns, in board order.
async fn workspace_status_names(client: &AtlasClient, ws: &str) -> Vec<String> {
    client
        .list_status_templates(ws)
        .await
        .expect("list workspace status templates")
        .into_iter()
        .map(|t| t.name)
        .collect()
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn platform_defaults_crud_round_trip() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;

    let first = create_default(&root, "Inbox", None).await;
    let second = create_default(&root, "Shipped", Some("green")).await;

    assert!(first.color.is_none());
    assert_eq!(second.color.as_deref(), Some("green"));

    let listed = root
        .list_platform_status_templates()
        .await
        .expect("list platform defaults");
    assert_eq!(
        listed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Inbox", "Shipped"],
        "creation appends, so the list follows insertion order"
    );

    let renamed = root
        .update_platform_status_template(
            first.id,
            UpdateStatusTemplateRequest {
                name: Some("Backlog".to_string()),
                color: Some(serde_json::Value::String("blue".to_string())),
                ..Default::default()
            },
        )
        .await
        .expect("rename platform default");
    assert_eq!(renamed.name, "Backlog");
    assert_eq!(renamed.color.as_deref(), Some("blue"));

    root.update_platform_status_template(
        second.id,
        UpdateStatusTemplateRequest {
            after: Some(renamed.position_key.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("reorder platform default");

    let reordered = root
        .list_platform_status_templates()
        .await
        .expect("list platform defaults");
    assert_eq!(
        reordered
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Shipped", "Backlog"],
        "moving Shipped before Backlog must reorder the list"
    );

    root.delete_platform_status_template(second.id)
        .await
        .expect("delete platform default");

    let remaining = root
        .list_platform_status_templates()
        .await
        .expect("list platform defaults");
    assert_eq!(
        remaining
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Backlog"]
    );

    db.teardown().await;
}

#[tokio::test]
async fn invalid_color_on_create_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;

    let err = root
        .create_platform_status_template(CreateStatusTemplateRequest {
            name: "Broken".to_string(),
            color: Some("not-a-swatch".to_string()),
            before: None,
            after: None,
        })
        .await
        .expect_err("invalid swatch must be rejected");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 422),
        "expected 422, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn deleting_an_unknown_default_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;

    let err = root
        .delete_platform_status_template(uuid::Uuid::now_v7())
        .await
        .expect_err("unknown id must not delete");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 404),
        "expected 404, got {err:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_plain_user_cannot_read_or_write_platform_defaults() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "pstpl-outsider").await;

    let read_err = client
        .list_platform_status_templates()
        .await
        .expect_err("a workspace owner is not an Atlas admin");
    assert!(
        matches!(read_err, ClientError::Api(ref p) if p.status == 403),
        "expected 403 on read, got {read_err:?}"
    );

    let write_err = client
        .create_platform_status_template(CreateStatusTemplateRequest {
            name: "Sneaky".to_string(),
            color: None,
            before: None,
            after: None,
        })
        .await
        .expect_err("a workspace owner must not write platform defaults");
    assert!(
        matches!(write_err, ClientError::Api(ref p) if p.status == 403),
        "expected 403 on write, got {write_err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn an_api_key_cannot_list_platform_defaults() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "pstpl-agent-forbidden").await;
    let api_key = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "platform-defaults-agent".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
            scopes: None,
        })
        .await
        .expect("create API key");
    let agent = AtlasClient::new(server.base_url()).with_token(api_key.secret);

    let err = agent
        .list_platform_status_templates()
        .await
        .expect_err("API keys must not list platform defaults");

    assert!(
        matches!(err, ClientError::Api(ref problem) if problem.status == 403),
        "expected 403, got {err:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Workspace seeding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_workspace_is_seeded_from_the_platform_defaults() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (creator, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "pstpl-seed-owner").await;

    create_default(&root, "Inbox", Some("neutral")).await;
    create_default(&root, "Shipped", Some("green")).await;

    let created = creator
        .create_workspace("Seeded From Platform")
        .await
        .expect("create_workspace");

    assert_eq!(
        workspace_status_names(&creator, &created.slug).await,
        vec!["Inbox", "Shipped"],
        "the new workspace copies the Atlas defaults, in order"
    );

    let projects = creator
        .list_projects(&created.slug, None, None)
        .await
        .expect("list_projects");
    let boards = creator
        .list_boards(&created.slug, &projects.items[0].slug, None, None)
        .await
        .expect("list_boards");
    let columns = creator
        .list_columns(&created.slug, boards.items[0].id)
        .await
        .expect("list_columns");

    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["Inbox", "Shipped"],
        "the seeded board derives its columns from the seeded templates"
    );

    db.teardown().await;
}

#[tokio::test]
async fn an_empty_platform_list_falls_back_to_the_compiled_defaults() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (creator, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "pstpl-fallback-owner").await;

    assert!(
        root.list_platform_status_templates()
            .await
            .expect("list platform defaults")
            .is_empty(),
        "a fresh instance starts with no Atlas defaults"
    );

    let created = creator
        .create_workspace("Fallback Target")
        .await
        .expect("create_workspace");

    assert_eq!(
        workspace_status_names(&creator, &created.slug).await,
        vec!["To Do", "In Progress", "Done"],
        "with no Atlas defaults the compiled fallback keeps the workspace usable"
    );

    db.teardown().await;
}

#[tokio::test]
async fn editing_platform_defaults_never_retro_updates_an_existing_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (creator, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "pstpl-existing-owner").await;

    let inbox = create_default(&root, "Inbox", None).await;

    let created = creator
        .create_workspace("Already Seeded")
        .await
        .expect("create_workspace");
    assert_eq!(
        workspace_status_names(&creator, &created.slug).await,
        vec!["Inbox"]
    );

    create_default(&root, "Shipped", None).await;
    root.update_platform_status_template(
        inbox.id,
        UpdateStatusTemplateRequest {
            name: Some("Triage".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("rename platform default");

    assert_eq!(
        workspace_status_names(&creator, &created.slug).await,
        vec!["Inbox"],
        "the platform list is a seed source, not a live link"
    );

    db.teardown().await;
}
