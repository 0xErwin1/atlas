#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! T6.3/T6.6 — the `GrantHygiene` replacement for the four dead-cascade
//! `permission_grants` target FKs the O1 migration drops (design §S3c).
//!
//! - Soft delete never calls `revoke_grants_for`: a grant on a soft-deleted
//!   resource stays exactly as resolvable as before.
//! - A hard purge revokes exactly the grants on rows the transitive closure
//!   deleted, and leaves everything outside that closure untouched.

mod support;

use atlas_acta::entities::boards_tasks::NewBoard;
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::entities::workspace_core::NewFolder;
use atlas_acta::entities::workspace_core::NewProject;
use atlas_acta::permissions::ResourceRef as ActaResourceRef;
use atlas_acta::permissions::Visibility;
use atlas_acta::permissions::VisibilityRole;
use atlas_acta::permissions::resource_ref_codec;
use atlas_acta::ports::boards_tasks::BoardRepo;
use atlas_acta::ports::documents::DocumentRepo;
use atlas_acta::ports::workspace_core::FolderRepo;
use atlas_acta::ports::workspace_core::ProjectRepo;
use atlas_server::authz::ResourceRole;
use atlas_server::authz::policy::NewPermissionGrant;
use atlas_server::persistence::repos::{PermissionGrantRepo, PgPermissionGrantRepo};

async fn grant_row_count(db: &support::TestDb, resource_ref: &str) -> i64 {
    use sea_orm::FromQueryResult;

    #[derive(sea_orm::FromQueryResult)]
    struct Row {
        count: i64,
    }

    Row::find_by_statement(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT count(*)::bigint AS count FROM permission_grants WHERE resource_ref = $1",
        [resource_ref.into()],
    ))
    .one(db.conn())
    .await
    .expect("count grants")
    .expect("count row")
    .count
}

#[tokio::test]
async fn soft_deleting_a_document_preserves_its_grant() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "hygiene-soft-delete").await;
    let ctx = support::ctx(&ws, &owner);

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Hygiene project".into(),
                slug: "hygiene-soft-delete".into(),
                task_prefix: "HYG".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");

    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Hygiene document".into(),
                slug: Some("hygiene-soft-delete-doc".into()),
                content: "body".into(),
                folder_id: None,
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    let document_ref = resource_ref_codec::to_core(&ActaResourceRef::Document(document.id), ws.id);

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: atlas_custos::WorkspaceScope(ws.id.0),
            user_id: Some(owner.id),
            api_key_id: None,
            group_id: None,
            resource_ref: document_ref.clone(),
            role: ResourceRole::Viewer,
            created_by_user_id: Some(owner.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert document grant");

    db.doc_repo()
        .soft_delete(&ctx, document.id)
        .await
        .expect("soft delete document");

    let remaining = grant_row_count(&db, &document_ref.to_string()).await;
    assert_eq!(
        remaining, 1,
        "soft delete must never call revoke_grants_for — the grant must survive"
    );

    let resolved = grant_repo
        .list_for_resource(
            atlas_custos::WorkspaceScope(ws.id.0),
            &document_ref,
            None,
            10,
        )
        .await
        .expect("list_for_resource");
    assert_eq!(resolved.len(), 1, "the grant must still resolve");

    db.teardown().await;
}

#[tokio::test]
async fn project_purge_revokes_exactly_the_transitively_affected_grants() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (ws, owner) = support::seed_workspace(&db, "hygiene-purge-closure").await;
    let ctx = support::ctx(&ws, &owner);

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Purge closure project".into(),
                slug: "hygiene-purge-closure".into(),
                task_prefix: "PGC".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");
    let folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: Some(project.id),
                parent_folder_id: None,
                name: "Purge closure folder".into(),
            },
        )
        .await
        .expect("create folder");
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Purge closure document".into(),
                slug: Some("hygiene-purge-closure-doc".into()),
                content: "body".into(),
                folder_id: Some(folder.id),
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let board = db
        .board_repo()
        .create_board(
            &ctx,
            NewBoard {
                project_id: project.id,
                folder_id: Some(folder.id),
                name: "Purge closure board".into(),
            },
        )
        .await
        .expect("create board");

    // A control project + grant outside the closure that must survive.
    let other_project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Control project".into(),
                slug: "hygiene-purge-closure-control".into(),
                task_prefix: "CTL".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create control project");

    let refs = [
        resource_ref_codec::to_core(&ActaResourceRef::Project(project.id), ws.id),
        resource_ref_codec::to_core(&ActaResourceRef::Folder(folder.id), ws.id),
        resource_ref_codec::to_core(&ActaResourceRef::Document(document.id), ws.id),
        resource_ref_codec::to_core(&ActaResourceRef::Board(board.id), ws.id),
    ];
    let control_ref =
        resource_ref_codec::to_core(&ActaResourceRef::Project(other_project.id), ws.id);

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    for resource_ref in refs
        .iter()
        .cloned()
        .chain(std::iter::once(control_ref.clone()))
    {
        grant_repo
            .upsert(NewPermissionGrant {
                workspace_id: atlas_custos::WorkspaceScope(ws.id.0),
                user_id: Some(owner.id),
                api_key_id: None,
                group_id: None,
                resource_ref,
                role: ResourceRole::Viewer,
                created_by_user_id: Some(owner.id),
                created_by_api_key_id: None,
            })
            .await
            .expect("upsert grant");
    }

    db.project_repo()
        .soft_delete(&ctx, project.id)
        .await
        .expect("soft delete project");

    let response = reqwest::Client::new()
        .post(format!("{}/api/admin/trash/purge", server.base_url()))
        .bearer_auth(admin.token().expect("admin token"))
        .json(&serde_json::json!({
            "kind": "project",
            "target_id": project.id.0,
            "confirm": true,
        }))
        .send()
        .await
        .expect("purge request");
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    for resource_ref in &refs {
        let remaining = grant_row_count(&db, &resource_ref.to_string()).await;
        assert_eq!(
            remaining, 0,
            "grant on {resource_ref} must be revoked by the purge closure"
        );
    }

    let control_remaining = grant_row_count(&db, &control_ref.to_string()).await;
    assert_eq!(
        control_remaining, 1,
        "a grant outside the purge closure must never be touched"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoke_survives_a_purge_closure_larger_than_one_statement_chunk() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "hygiene-chunked-revoke").await;

    let seeded: Vec<atlas_core::ids::ResourceRef> = (0..3)
        .map(|_| {
            format!("acta::document::{}", uuid::Uuid::now_v7())
                .parse()
                .expect("valid resource ref")
        })
        .collect();

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    for resource_ref in seeded.iter().cloned() {
        grant_repo
            .upsert(NewPermissionGrant {
                workspace_id: atlas_custos::WorkspaceScope(ws.id.0),
                user_id: Some(owner.id),
                api_key_id: None,
                group_id: None,
                resource_ref,
                role: ResourceRole::Viewer,
                created_by_user_id: Some(owner.id),
                created_by_api_key_id: None,
            })
            .await
            .expect("upsert grant");
    }

    // 2050 refs spans three 1024-ref chunks; the seeded refs sit in the
    // first, middle, and last chunk so every chunk's DELETE is exercised.
    let mut refs: Vec<atlas_core::ids::ResourceRef> = (0..2047)
        .map(|_| {
            format!("acta::document::{}", uuid::Uuid::now_v7())
                .parse()
                .expect("valid resource ref")
        })
        .collect();
    let [first, middle, last]: [atlas_core::ids::ResourceRef; 3] = seeded
        .clone()
        .try_into()
        .expect("exactly three seeded refs");
    refs.insert(0, first);
    refs.insert(1500, middle);
    refs.push(last);

    atlas_custos_postgres::repos::grant_hygiene::PgGrantHygiene::revoke_grants_for_in(
        db.conn(),
        &refs,
    )
    .await
    .expect("chunked revoke must not exceed the bind-parameter limit");

    for resource_ref in &seeded {
        let remaining = grant_row_count(&db, &resource_ref.to_string()).await;
        assert_eq!(
            remaining, 0,
            "grant on {resource_ref} must be revoked regardless of which chunk carries it"
        );
    }

    db.teardown().await;
}
