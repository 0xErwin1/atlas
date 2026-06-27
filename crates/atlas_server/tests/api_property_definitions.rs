#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    boards_tasks::{CreateBoardRequest, CreateColumnRequest, CreateTaskRequest, TaskPropertiesDto},
    property_definitions::CreatePropertyDefinitionRequest,
};
use atlas_client::{AtlasClient, ClientError};
use serde_json::json;

async fn seed_board_column(client: &AtlasClient, ws_slug: &str) -> (uuid::Uuid, uuid::Uuid) {
    let project = client
        .create_project(
            ws_slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj".into(),
                task_prefix: "PR".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = client
        .create_board(
            ws_slug,
            &project.slug,
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            ws_slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".into(),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    (board.id, col.id)
}

fn def(
    name: &str,
    kind: &str,
    options: Option<serde_json::Value>,
) -> CreatePropertyDefinitionRequest {
    CreatePropertyDefinitionRequest {
        name: name.to_string(),
        kind: kind.to_string(),
        options,
        applies_to: None,
    }
}

// ---------------------------------------------------------------------------
// Create + list across kinds, key derivation, applies_to filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_property_definitions() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "propdef-list").await;

    let severity = client
        .create_property_definition(
            &ws.slug,
            def(
                "Severity",
                "select",
                Some(json!(["low", "high", "critical"])),
            ),
        )
        .await
        .expect("create select definition");

    assert_eq!(severity.key, "severity", "key is derived from the name");
    assert_eq!(severity.kind, "select");
    assert_eq!(severity.applies_to, "task", "applies_to defaults to task");

    client
        .create_property_definition(&ws.slug, def("Story Points!", "number", None))
        .await
        .map(|d| assert_eq!(d.key, "story_points", "non-alphanumerics collapse to one _"))
        .expect("create number definition");

    for (name, kind) in [("Reviewed", "boolean"), ("Notes", "text"), ("Due", "date")] {
        client
            .create_property_definition(&ws.slug, def(name, kind, None))
            .await
            .unwrap_or_else(|e| panic!("create {kind} definition failed: {e:?}"));
    }

    // A document-only field must be excluded from the task-applicability filter.
    client
        .create_property_definition(
            &ws.slug,
            CreatePropertyDefinitionRequest {
                name: "Doc Only".into(),
                kind: "text".into(),
                options: None,
                applies_to: Some("document".into()),
            },
        )
        .await
        .expect("create document-only definition");

    let all = client
        .list_property_definitions(&ws.slug, None)
        .await
        .expect("list all");
    assert_eq!(
        all.len(),
        6,
        "all six definitions are returned without a filter"
    );

    let task_scoped = client
        .list_property_definitions(&ws.slug, Some("task"))
        .await
        .expect("list task-applicable");
    assert_eq!(
        task_scoped.len(),
        5,
        "applies_to=task excludes the document-only field"
    );
    assert!(
        task_scoped.iter().all(|d| d.key != "doc_only"),
        "document-only field must not appear under applies_to=task"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Validation: key collision, options rules, undeducible name
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_property_definition_validation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "propdef-valid").await;

    client
        .create_property_definition(&ws.slug, def("Severity", "select", Some(json!(["low"]))))
        .await
        .expect("create first severity");

    let collision = client
        .create_property_definition(&ws.slug, def("severity", "text", None))
        .await;
    assert!(
        matches!(collision, Err(ClientError::Api(ref p)) if p.status == 409),
        "duplicate derived key must return 409, got {collision:?}"
    );

    let symbol_only = client
        .create_property_definition(&ws.slug, def("!!!", "text", None))
        .await;
    assert!(
        matches!(symbol_only, Err(ClientError::Api(ref p)) if p.status == 422),
        "symbol-only name must return 422, got {symbol_only:?}"
    );

    let unknown_kind = client
        .create_property_definition(&ws.slug, def("Thing", "rating", None))
        .await;
    assert!(
        matches!(unknown_kind, Err(ClientError::Api(ref p)) if p.status == 422),
        "unknown kind must return 422, got {unknown_kind:?}"
    );

    let select_without_options = client
        .create_property_definition(&ws.slug, def("Level", "select", None))
        .await;
    assert!(
        matches!(select_without_options, Err(ClientError::Api(ref p)) if p.status == 422),
        "select without options must return 422, got {select_without_options:?}"
    );

    let select_dup_options = client
        .create_property_definition(&ws.slug, def("Level", "select", Some(json!(["a", "a"]))))
        .await;
    assert!(
        matches!(select_dup_options, Err(ClientError::Api(ref p)) if p.status == 422),
        "select with duplicate options must return 422, got {select_dup_options:?}"
    );

    let text_with_options = client
        .create_property_definition(&ws.slug, def("Note", "text", Some(json!(["a"]))))
        .await;
    assert!(
        matches!(text_with_options, Err(ClientError::Api(ref p)) if p.status == 422),
        "non-select with options must return 422, got {text_with_options:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Task custom values validated against definitions on create + update
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_custom_values_validated_on_create_and_update() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "propdef-task").await;

    for d in [
        def(
            "Severity",
            "select",
            Some(json!(["low", "high", "critical"])),
        ),
        def("Score", "number", None),
        def("Reviewed", "boolean", None),
        def("Tags", "multi_select", Some(json!(["a", "b", "c"]))),
    ] {
        client
            .create_property_definition(&ws.slug, d)
            .await
            .expect("create definition");
    }

    let (board_id, col_id) = seed_board_column(&client, &ws.slug).await;

    // Valid custom values on create.
    let task = client
        .create_task(
            &ws.slug,
            board_id,
            CreateTaskRequest {
                column_id: col_id,
                title: "Has custom values".into(),
                description: None,
                properties: Some(TaskPropertiesDto {
                    custom: Some(json!({
                        "severity": "high",
                        "score": 5,
                        "reviewed": true,
                        "tags": ["a", "c"]
                    })),
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task with valid custom values");

    // Valid update replaces the whole custom map.
    client
        .update_task(
            &ws.slug,
            &task.readable_id,
            atlas_api::dtos::boards_tasks::UpdateTaskRequest {
                properties: Some(json!({ "severity": "low" })),
                ..Default::default()
            },
        )
        .await
        .expect("update task with valid custom value");

    // Unknown key rejected.
    let unknown = client
        .update_task(
            &ws.slug,
            &task.readable_id,
            atlas_api::dtos::boards_tasks::UpdateTaskRequest {
                properties: Some(json!({ "made_up": 1 })),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(unknown, Err(ClientError::Api(ref p)) if p.status == 422),
        "unknown custom key must return 422, got {unknown:?}"
    );

    // Wrong-typed value rejected.
    let wrong_type = client
        .update_task(
            &ws.slug,
            &task.readable_id,
            atlas_api::dtos::boards_tasks::UpdateTaskRequest {
                properties: Some(json!({ "score": "not a number" })),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(wrong_type, Err(ClientError::Api(ref p)) if p.status == 422),
        "wrong-typed custom value must return 422, got {wrong_type:?}"
    );

    // Invalid select option rejected.
    let bad_option = client
        .update_task(
            &ws.slug,
            &task.readable_id,
            atlas_api::dtos::boards_tasks::UpdateTaskRequest {
                properties: Some(json!({ "severity": "nope" })),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(bad_option, Err(ClientError::Api(ref p)) if p.status == 422),
        "invalid select option must return 422, got {bad_option:?}"
    );

    // Invalid create is rejected too (validation runs on both paths).
    let bad_create = client
        .create_task(
            &ws.slug,
            board_id,
            CreateTaskRequest {
                column_id: col_id,
                title: "Bad".into(),
                description: None,
                properties: Some(TaskPropertiesDto {
                    custom: Some(json!({ "tags": ["a", "a"] })),
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await;
    assert!(
        matches!(bad_create, Err(ClientError::Api(ref p)) if p.status == 422),
        "duplicate multi_select value on create must return 422, got {bad_create:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Soft delete removes the definition from the list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn soft_delete_property_definition() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "propdef-delete").await;

    let created = client
        .create_property_definition(&ws.slug, def("Severity", "select", Some(json!(["low"]))))
        .await
        .expect("create definition");

    client
        .delete_property_definition(&ws.slug, created.id)
        .await
        .expect("delete definition");

    let listed = client
        .list_property_definitions(&ws.slug, None)
        .await
        .expect("list after delete");
    assert!(
        listed.iter().all(|d| d.id != created.id),
        "soft-deleted definition must not appear in the list"
    );

    db.teardown().await;
}
