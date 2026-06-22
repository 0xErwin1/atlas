#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateProjectRequest,
    boards_tasks::{CreateBoardRequest, CreateTaskRequest},
    status_templates::{
        CreateStatusTemplateRequest, StatusTemplateDto, UpdateStatusTemplateRequest,
    },
};
use atlas_client::ClientError;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

async fn create_template(
    client: &atlas_client::AtlasClient,
    ws: &str,
    name: &str,
    color: Option<&str>,
) -> StatusTemplateDto {
    client
        .create_status_template(
            ws,
            CreateStatusTemplateRequest {
                name: name.to_string(),
                color: color.map(str::to_string),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create template")
}

// ---------------------------------------------------------------------------
// CRUD happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_template_returns_201_and_appears_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-crud-1").await;

    let tpl = create_template(&client, &ws.slug, "Todo", None).await;

    assert_eq!(tpl.name, "Todo");
    assert!(tpl.color.is_none());
    assert_eq!(tpl.workspace_id, ws.id.0);

    let list = client
        .list_status_templates(&ws.slug)
        .await
        .expect("list templates");

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, tpl.id);

    db.teardown().await;
}

#[tokio::test]
async fn create_template_with_hex_color_is_accepted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-hex-1").await;

    let tpl = create_template(&client, &ws.slug, "In Progress", Some("#1A2B3C")).await;

    assert_eq!(tpl.color.as_deref(), Some("#1A2B3C"));

    db.teardown().await;
}

#[tokio::test]
async fn create_template_with_named_swatch_is_accepted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-swatch-1").await;

    let tpl = create_template(&client, &ws.slug, "Done", Some("green")).await;

    assert_eq!(tpl.color.as_deref(), Some("green"));

    db.teardown().await;
}

#[tokio::test]
async fn list_templates_ordered_by_position_key() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-order-1").await;

    create_template(&client, &ws.slug, "Alpha", None).await;
    create_template(&client, &ws.slug, "Beta", None).await;
    create_template(&client, &ws.slug, "Gamma", None).await;

    let list = client
        .list_status_templates(&ws.slug)
        .await
        .expect("list templates");

    assert_eq!(list.len(), 3);
    assert_eq!(list[0].name, "Alpha");
    assert_eq!(list[1].name, "Beta");
    assert_eq!(list[2].name, "Gamma");

    db.teardown().await;
}

#[tokio::test]
async fn patch_template_rename() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-rename-1").await;

    let tpl = create_template(&client, &ws.slug, "Old Name", None).await;

    let updated = client
        .update_status_template(
            &ws.slug,
            tpl.id,
            UpdateStatusTemplateRequest {
                name: Some("New Name".to_string()),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("update template");

    assert_eq!(updated.name, "New Name");
    assert_eq!(updated.id, tpl.id);

    db.teardown().await;
}

#[tokio::test]
async fn patch_template_recolor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-recolor-1").await;

    let tpl = create_template(&client, &ws.slug, "Status", None).await;

    let updated = client
        .update_status_template(
            &ws.slug,
            tpl.id,
            UpdateStatusTemplateRequest {
                name: None,
                color: Some(serde_json::Value::String("blue".to_string())),
                before: None,
                after: None,
            },
        )
        .await
        .expect("update template");

    assert_eq!(updated.color.as_deref(), Some("blue"));

    db.teardown().await;
}

#[tokio::test]
async fn patch_template_reorder() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-reorder-1").await;

    let a = create_template(&client, &ws.slug, "A", None).await;
    let _b = create_template(&client, &ws.slug, "B", None).await;
    let c = create_template(&client, &ws.slug, "C", None).await;

    // Move C before A (i.e., after = A.position_key)
    client
        .update_status_template(
            &ws.slug,
            c.id,
            UpdateStatusTemplateRequest {
                name: None,
                color: None,
                before: None,
                after: Some(a.position_key.clone()),
            },
        )
        .await
        .expect("reorder template");

    let list = client
        .list_status_templates(&ws.slug)
        .await
        .expect("list templates");

    let names: Vec<&str> = list.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["C", "A", "B"]);

    db.teardown().await;
}

#[tokio::test]
async fn soft_delete_template_removes_from_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-delete-1").await;

    let tpl = create_template(&client, &ws.slug, "Disposable", None).await;

    client
        .delete_status_template(&ws.slug, tpl.id)
        .await
        .expect("delete template");

    let list = client
        .list_status_templates(&ws.slug)
        .await
        .expect("list templates");

    assert!(list.is_empty(), "deleted template must not appear in list");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Color validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_template_invalid_color_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-badcolor-1").await;

    let err = client
        .create_status_template(
            &ws.slug,
            CreateStatusTemplateRequest {
                name: "Bad".to_string(),
                color: Some("hotpink".to_string()),
                before: None,
                after: None,
            },
        )
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 422),
        "expected 422, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn patch_template_invalid_color_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-badcolor-2").await;

    let tpl = create_template(&client, &ws.slug, "X", None).await;

    let err = client
        .update_status_template(
            &ws.slug,
            tpl.id,
            UpdateStatusTemplateRequest {
                name: None,
                color: Some(serde_json::Value::String("#ZZZ111".to_string())),
                before: None,
                after: None,
            },
        )
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 422),
        "expected 422, got {err:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Board-create seed: templates present → new board has matching columns
// ---------------------------------------------------------------------------

#[tokio::test]
async fn board_create_with_templates_seeds_matching_columns() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-seed-1").await;

    create_template(&client, &ws.slug, "Backlog", Some("neutral")).await;
    create_template(&client, &ws.slug, "In Progress", Some("blue")).await;
    create_template(&client, &ws.slug, "Done", Some("green")).await;

    let project = client
        .create_project(&ws.slug, project_req("seed-proj-1", "SEED"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                name: "Seeded Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let cols = client
        .list_columns(&ws.slug, board.id)
        .await
        .expect("list columns");

    assert_eq!(cols.len(), 3, "board should have 3 seeded columns");

    let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Backlog", "In Progress", "Done"]);

    assert_eq!(cols[0].color.as_deref(), Some("neutral"));
    assert_eq!(cols[1].color.as_deref(), Some("blue"));
    assert_eq!(cols[2].color.as_deref(), Some("green"));

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Board-create seed: no templates → board stays empty
// ---------------------------------------------------------------------------

#[tokio::test]
async fn board_create_without_templates_stays_empty() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-empty-1").await;

    let project = client
        .create_project(&ws.slug, project_req("empty-proj-1", "EMPT"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                name: "Empty Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let cols = client
        .list_columns(&ws.slug, board.id)
        .await
        .expect("list columns");

    assert!(
        cols.is_empty(),
        "board should have no columns when no templates exist"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Apply-status-templates action
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apply_status_templates_adds_missing_columns() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-apply-1").await;

    create_template(&client, &ws.slug, "Todo", Some("neutral")).await;
    create_template(&client, &ws.slug, "Doing", Some("blue")).await;
    create_template(&client, &ws.slug, "Done", Some("green")).await;

    let _project = client
        .create_project(&ws.slug, project_req("apply-proj-1", "APL"))
        .await
        .expect("create project");

    // Create board without templates seeded (templates created after board for a fresh scenario)
    // Actually templates already exist so board will be seeded. Let's instead create a
    // workspace without templates, create the board, then add templates and apply.
    // Easier: create a second workspace with a board + one existing column, then apply.
    let (client2, ws2, _) = support::login_user_with_workspace(&server, &db, "stpl-apply-1b").await;

    let _tmpl_a = create_template(&client2, &ws2.slug, "Todo", Some("neutral")).await;
    let _tmpl_b = create_template(&client2, &ws2.slug, "Doing", Some("blue")).await;
    let _tmpl_c = create_template(&client2, &ws2.slug, "Done", Some("green")).await;

    let proj2 = client2
        .create_project(&ws2.slug, project_req("apply-proj-1b", "AP2"))
        .await
        .expect("create project");

    // Board is seeded with all 3 columns
    let board2 = client2
        .create_board(
            &ws2.slug,
            &proj2.slug,
            CreateBoardRequest {
                name: "Board2".to_string(),
            },
        )
        .await
        .expect("create board");

    // Verify 3 seeded columns
    let cols_before = client2
        .list_columns(&ws2.slug, board2.id)
        .await
        .expect("list");
    assert_eq!(cols_before.len(), 3);

    // Apply again — should be idempotent (no new columns added)
    let cols_after = client2
        .apply_status_templates(&ws2.slug, board2.id)
        .await
        .expect("apply");
    assert_eq!(
        cols_after.len(),
        3,
        "re-applying should not add duplicate columns"
    );

    // Now test a board that has only 'Todo' column manually and 'Doing'+'Done' from apply
    let (client3, ws3, _) = support::login_user_with_workspace(&server, &db, "stpl-apply-1c").await;

    let _tmpl3_a = create_template(&client3, &ws3.slug, "Todo", Some("neutral")).await;
    let _tmpl3_b = create_template(&client3, &ws3.slug, "Doing", Some("blue")).await;

    let proj3 = client3
        .create_project(&ws3.slug, project_req("apply-proj-1c", "AP3"))
        .await
        .expect("create project");

    // Board seeded with Todo + Doing
    let board3 = client3
        .create_board(
            &ws3.slug,
            &proj3.slug,
            CreateBoardRequest {
                name: "Board3".to_string(),
            },
        )
        .await
        .expect("create board");

    let cols3 = client3
        .list_columns(&ws3.slug, board3.id)
        .await
        .expect("list");
    assert_eq!(cols3.len(), 2);

    // Add a new template
    let _new_tpl = create_template(&client3, &ws3.slug, "Review", Some("amber")).await;

    // Apply adds only "Review"
    let result3 = client3
        .apply_status_templates(&ws3.slug, board3.id)
        .await
        .expect("apply");

    let names3: Vec<&str> = result3.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names3.contains(&"Todo"),
        "Todo already existed, should remain"
    );
    assert!(
        names3.contains(&"Doing"),
        "Doing already existed, should remain"
    );
    assert!(names3.contains(&"Review"), "Review must be added");
    assert_eq!(result3.len(), 3);

    db.teardown().await;
}

#[tokio::test]
async fn apply_status_templates_is_idempotent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-idem-1").await;

    create_template(&client, &ws.slug, "Open", None).await;
    create_template(&client, &ws.slug, "Closed", None).await;

    let project = client
        .create_project(&ws.slug, project_req("idem-proj-1", "IDM"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                name: "Idem Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let first = client
        .apply_status_templates(&ws.slug, board.id)
        .await
        .expect("first apply");

    let second = client
        .apply_status_templates(&ws.slug, board.id)
        .await
        .expect("second apply");

    assert_eq!(first.len(), second.len(), "idempotent: same count");

    let first_ids: Vec<_> = first.iter().map(|c| c.id).collect();
    let second_ids: Vec<_> = second.iter().map(|c| c.id).collect();
    assert_eq!(first_ids, second_ids, "idempotent: same column IDs");

    db.teardown().await;
}

#[tokio::test]
async fn apply_status_templates_case_insensitive_matching() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-case-1").await;

    create_template(&client, &ws.slug, "todo", None).await;

    let project = client
        .create_project(&ws.slug, project_req("case-proj-1", "CSE"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                name: "Case Board".to_string(),
            },
        )
        .await
        .expect("create board");

    // Board seeded with "todo". Manually add "TODO" via column API.
    // The board is already seeded with "todo" from the template.
    // Apply again — "todo" template should not add a second column named "Todo".
    let cols_before = client.list_columns(&ws.slug, board.id).await.expect("list");
    assert_eq!(cols_before.len(), 1);

    let result = client
        .apply_status_templates(&ws.slug, board.id)
        .await
        .expect("apply");

    assert_eq!(result.len(), 1, "case-insensitive match: no duplicate");

    db.teardown().await;
}

#[tokio::test]
async fn apply_keeps_existing_columns_with_tasks_intact() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-tasks-1").await;

    create_template(&client, &ws.slug, "Existing", None).await;
    create_template(&client, &ws.slug, "New One", None).await;

    let project = client
        .create_project(&ws.slug, project_req("tasks-proj-1", "TSK"))
        .await
        .expect("create project");

    // Board seeded with "Existing" and "New One"
    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                name: "Tasks Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let cols = client.list_columns(&ws.slug, board.id).await.expect("list");
    assert_eq!(cols.len(), 2);

    // Add a task to the first column
    let existing_col = &cols[0];
    client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: existing_col.id,
                title: "Task 1".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    // Apply templates — should not disturb the column that has a task
    let result = client
        .apply_status_templates(&ws.slug, board.id)
        .await
        .expect("apply");

    assert_eq!(
        result.len(),
        2,
        "apply should not remove or duplicate columns"
    );

    let col_ids: Vec<_> = result.iter().map(|c| c.id).collect();
    assert!(
        col_ids.contains(&existing_col.id),
        "column with tasks must be preserved"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Copy semantics: editing a template does NOT change existing board columns
// ---------------------------------------------------------------------------

#[tokio::test]
async fn template_edit_does_not_change_board_columns() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "stpl-copy-1").await;

    let tpl = create_template(&client, &ws.slug, "Original", Some("neutral")).await;

    let project = client
        .create_project(&ws.slug, project_req("copy-proj-1", "CPY"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                name: "Copy Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let cols_before = client.list_columns(&ws.slug, board.id).await.expect("list");
    assert_eq!(cols_before.len(), 1);
    assert_eq!(cols_before[0].name, "Original");

    // Rename and recolor the template
    client
        .update_status_template(
            &ws.slug,
            tpl.id,
            UpdateStatusTemplateRequest {
                name: Some("Changed".to_string()),
                color: Some(serde_json::Value::String("red".to_string())),
                before: None,
                after: None,
            },
        )
        .await
        .expect("update template");

    // The board column must be unchanged (copy semantics)
    let cols_after = client.list_columns(&ws.slug, board.id).await.expect("list");
    assert_eq!(cols_after.len(), 1);
    assert_eq!(
        cols_after[0].name, "Original",
        "template rename must not affect board column"
    );
    assert_eq!(
        cols_after[0].color.as_deref(),
        Some("neutral"),
        "template recolor must not affect board column"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Isolation: per-workspace; non-member gets 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn templates_are_isolated_per_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client1, ws1, _) = support::login_user_with_workspace(&server, &db, "stpl-iso-1").await;
    let (client2, ws2, _) = support::login_user_with_workspace(&server, &db, "stpl-iso-2").await;

    create_template(&client1, &ws1.slug, "WS1 Template", None).await;

    let list1 = client1
        .list_status_templates(&ws1.slug)
        .await
        .expect("list ws1");
    let list2 = client2
        .list_status_templates(&ws2.slug)
        .await
        .expect("list ws2");

    assert_eq!(list1.len(), 1);
    assert_eq!(list2.len(), 0, "ws2 must not see ws1 templates");

    db.teardown().await;
}

#[tokio::test]
async fn non_member_cannot_access_status_templates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client1, ws1, _) =
        support::login_user_with_workspace(&server, &db, "stpl-nonmember-1").await;
    let (client2, _ws2, _) =
        support::login_user_with_workspace(&server, &db, "stpl-nonmember-2").await;

    let err = client2
        .list_status_templates(&ws1.slug)
        .await
        .expect_err("should be forbidden");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 403 || p.status == 404),
        "expected 403 or 404, got {err:?}"
    );

    db.teardown().await;
}
