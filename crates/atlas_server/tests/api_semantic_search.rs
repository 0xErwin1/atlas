#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_api::{
    dtos::{
        ApiKeyScope, CreateProjectRequest, CreateUserApiKeyRequest,
        semantic_search::SemanticSearchHitDto,
    },
    pagination::Page,
};
use atlas_domain::{
    WorkspaceCtx,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        documents::NewDocument,
    },
    semantic_search::{ResourceKind, SemanticIndexChunk, SemanticSearchSource},
};
use atlas_server::{
    embeddings::DeterministicEmbeddingProvider,
    persistence::repos::{
        BoardRepo, DocumentRepo, PermissionGrantRepo, PgApiKeyRepo, PgBoardRepo, PgDocumentRepo,
        PgPermissionGrantRepo, PgSemanticIndexWriter, PgTaskRepo, TaskRepo,
    },
};
use serde_json::Value;
use std::sync::Arc;

fn semantic_search_url(base: &str, ws: &str, qs: &str) -> String {
    if qs.is_empty() {
        format!("{base}/api/workspaces/{ws}/semantic-search")
    } else {
        format!("{base}/api/workspaces/{ws}/semantic-search?{qs}")
    }
}

async fn get_semantic_search(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    ws: &str,
    qs: &str,
) -> reqwest::Response {
    http.get(semantic_search_url(base, ws, qs))
        .bearer_auth(token)
        .send()
        .await
        .expect("HTTP request must succeed")
}

#[tokio::test]
async fn semantic_search_absent_q_returns_422_on_dedicated_route() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "sem-abq").await;
    let token = client.token().expect("must be logged in");
    let http = reqwest::Client::new();

    let resp = get_semantic_search(&http, token, server.base_url(), &ws.slug, "").await;

    assert_eq!(resp.status().as_u16(), 422, "absent q must return 422");
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(
        body.get("type"),
        Some(&Value::String("urn:atlas:error:invalid-input".to_owned()))
    );

    db.teardown().await;
}

#[tokio::test]
async fn semantic_search_returns_compact_page_without_lexical_route_change() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "sem-hit").await;
    let token = client.token().expect("must be logged in");
    let http = reqwest::Client::new();
    let ctx = WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(user.id));

    let doc = PgDocumentRepo::new(db.conn().clone(), 50)
        .create(
            &ctx,
            NewDocument {
                title: "Semantic Incident Runbook".to_owned(),
                slug: None,
                content: "response procedure".to_owned(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");
    let provider = Arc::new(
        DeterministicEmbeddingProvider::new("atlas-test-embedding", 1536).expect("provider"),
    );
    PgSemanticIndexWriter::new(db.conn().clone(), provider)
        .index_chunks(&[SemanticIndexChunk {
            workspace_id: ws.id,
            kind: ResourceKind::Document,
            resource_id: doc.id.0,
            source: SemanticSearchSource::Content,
            chunk_ordinal: 0,
            content_hash: "semantic incident response".to_owned(),
            text: "semantic incident response".to_owned(),
            excerpt: "semantic incident response".to_owned(),
        }])
        .await
        .expect("index chunk");

    let semantic_resp = get_semantic_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        "q=semantic%20incident&type=document&limit=5",
    )
    .await;
    assert_eq!(semantic_resp.status().as_u16(), 200);
    let page: Page<SemanticSearchHitDto> = semantic_resp.json().await.expect("semantic page");
    assert_eq!(page.items.len(), 1);
    let hit = page.items.first().expect("one semantic hit");
    assert_eq!(hit.id, doc.id.0);
    assert_eq!(hit.excerpt, "semantic incident response");

    let raw_page: Value = get_semantic_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        "q=semantic%20incident&type=document&limit=5",
    )
    .await
    .json()
    .await
    .expect("semantic json page");
    let raw_items = raw_page
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    let raw_hit = raw_items.first().expect("first semantic hit");
    for bulky_field in ["content", "body", "comments", "created_at", "updated_at"] {
        assert!(
            raw_hit.get(bulky_field).is_none(),
            "semantic API hit must omit bulky/default field {bulky_field}"
        );
    }
    assert_eq!(raw_hit.get("kind"), Some(&Value::String("document".into())));
    assert_eq!(
        raw_hit.get("source"),
        Some(&Value::String("content".into()))
    );
    assert_eq!(
        raw_hit.get("excerpt"),
        Some(&Value::String("semantic incident response".into()))
    );

    let lexical_resp = http
        .get(format!(
            "{}/api/workspaces/{}/search?q=semantic%20incident",
            server.base_url(),
            ws.slug
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("lexical request");
    assert_eq!(lexical_resp.status().as_u16(), 200);

    db.teardown().await;
}

#[tokio::test]
async fn semantic_search_api_key_scope_filters_hit_families() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, user) = support::login_user_with_workspace(&server, &db, "sem-scope").await;
    let owner_token = owner.token().expect("must be logged in");
    let http = reqwest::Client::new();
    let ctx = WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(user.id));

    let doc = PgDocumentRepo::new(db.conn().clone(), 50)
        .create(
            &ctx,
            NewDocument {
                title: "Document scope result".to_owned(),
                slug: None,
                content: "shared semantic scope".to_owned(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");
    let project = owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Scope Project".to_owned(),
                slug: "scope-project".to_owned(),
                task_prefix: "SCP".to_owned(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("seed project");
    let project_id = atlas_domain::ids::ProjectId(project.id);
    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(
            &ctx,
            NewBoard {
                project_id,
                name: "Scope Board".to_owned(),
            },
        )
        .await
        .expect("seed board");
    let column = PgBoardRepo::new(db.conn().clone())
        .add_column(
            &ctx,
            board.id,
            "Todo".to_owned(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed column");
    let task = PgTaskRepo::new(db.conn().clone())
        .create(
            &ctx,
            NewTask {
                project_id,
                board_id: board.id,
                column_id: column.id,
                title: "Task scope result".to_owned(),
                description: "shared semantic scope".to_owned(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("seed task");

    let provider = Arc::new(
        DeterministicEmbeddingProvider::new("atlas-test-embedding", 1536).expect("provider"),
    );
    PgSemanticIndexWriter::new(db.conn().clone(), provider)
        .index_chunks(&[
            SemanticIndexChunk {
                workspace_id: ws.id,
                kind: ResourceKind::Document,
                resource_id: doc.id.0,
                source: SemanticSearchSource::Content,
                chunk_ordinal: 0,
                content_hash: "shared semantic scope doc".to_owned(),
                text: "shared semantic scope doc".to_owned(),
                excerpt: "document excerpt".to_owned(),
            },
            SemanticIndexChunk {
                workspace_id: ws.id,
                kind: ResourceKind::Task,
                resource_id: task.id.0,
                source: SemanticSearchSource::Aggregate,
                chunk_ordinal: 0,
                content_hash: "shared semantic scope task".to_owned(),
                text: "shared semantic scope task".to_owned(),
                excerpt: "task excerpt".to_owned(),
            },
        ])
        .await
        .expect("index chunks");

    let key = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "docs-only-semantic".to_owned(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
            scopes: Some(vec![ApiKeyScope::DocsRead]),
        })
        .await
        .expect("create api key");
    let key_id = atlas_domain::ids::ApiKeyId(key.id);
    PgApiKeyRepo::set_global_for_user_in(db.conn(), user.id, key_id, true)
        .await
        .expect("make key global");
    PgPermissionGrantRepo {
        conn: db.conn().clone(),
    }
    .upsert(atlas_domain::entities::permissions::NewPermissionGrant {
        workspace_id: ws.id,
        user_id: None,
        api_key_id: Some(key_id),
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: None,
        board_id: None,
        role: atlas_domain::permissions::ResourceRole::Viewer,
        created_by_user_id: Some(user.id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant key workspace viewer");

    let owner_resp = get_semantic_search(
        &http,
        owner_token,
        server.base_url(),
        &ws.slug,
        "q=shared%20semantic%20scope&type=all&limit=10",
    )
    .await;
    assert_eq!(owner_resp.status().as_u16(), 200);
    let owner_page: Page<SemanticSearchHitDto> = owner_resp.json().await.expect("owner page");
    assert!(
        owner_page.items.iter().any(|hit| hit.id == doc.id.0),
        "owner semantic search should include the indexed document"
    );
    assert!(
        owner_page.items.iter().any(|hit| hit.id == task.id.0),
        "owner semantic search should include the indexed task"
    );

    let api_key_resp = get_semantic_search(
        &http,
        &key.secret,
        server.base_url(),
        &ws.slug,
        "q=shared%20semantic%20scope&type=all&limit=10",
    )
    .await;
    assert_eq!(api_key_resp.status().as_u16(), 200);
    let api_key_page: Page<SemanticSearchHitDto> = api_key_resp.json().await.expect("api key page");
    assert!(
        api_key_page.items.iter().any(|hit| hit.id == doc.id.0),
        "docs:read API key should see document semantic hits"
    );
    assert!(
        api_key_page.items.iter().all(|hit| hit.id != task.id.0),
        "API key without tasks:read must not receive task semantic hits"
    );

    db.teardown().await;
}
