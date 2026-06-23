#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateProjectRequest,
    folders::{CreateFolderRequest, MoveFolderRequest, RenameFolderRequest},
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};

fn project_req(name: &str, slug: &str) -> CreateProjectRequest {
    let prefix: String = slug
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .take(10)
        .collect();
    let prefix = if prefix.len() >= 2 {
        prefix
    } else {
        format!("{prefix}XX")
    };
    CreateProjectRequest {
        name: name.to_string(),
        slug: slug.to_string(),
        task_prefix: prefix,
        visibility: None,
        visibility_role: None,
    }
}

// ---- REQ-F1: Create folder — editor gets 201 ------------------------------------

#[tokio::test]
async fn create_folder_editor_returns_201() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-create-1").await;

    let project = client
        .create_project(&ws.slug, project_req("FolderProj", "folder-proj-1"))
        .await
        .expect("create project");

    let folder = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "My Folder".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create folder");

    assert_eq!(folder.name, "My Folder");
    assert_eq!(folder.workspace_id, ws.id.0);
    assert_eq!(folder.project_id, Some(project.id));
    assert!(folder.parent_folder_id.is_none());

    db.teardown().await;
}

// ---- Duplicate name in the same location → 409 (not an opaque 500) ---------------

#[tokio::test]
async fn create_duplicate_folder_name_returns_409() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-dup-1").await;

    let project = client
        .create_project(&ws.slug, project_req("DupProj", "dup-proj-1"))
        .await
        .expect("create project");

    client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Same Name".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("first create folder");

    let err = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Same Name".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect_err("duplicate name should fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 409, "expected 409, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F2: Blank name → 422 ---------------------------------------------------

#[tokio::test]
async fn create_folder_blank_name_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-blank-1").await;

    let project = client
        .create_project(&ws.slug, project_req("BlankNameProj", "blank-name-proj"))
        .await
        .expect("create project");

    let err = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "   ".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect_err("blank name should fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 422, "expected 422, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F3: Foreign parent_folder_id → 422 --------------------------------------

#[tokio::test]
async fn create_folder_foreign_parent_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "folder-foreign-1").await;

    let project = client
        .create_project(
            &ws.slug,
            project_req("ForeignParent", "foreign-parent-proj"),
        )
        .await
        .expect("create project");

    let foreign_id = uuid::Uuid::new_v4();
    let err = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Orphan".to_string(),
                parent_folder_id: Some(foreign_id),
            },
        )
        .await
        .expect_err("foreign parent should fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 422, "expected 422, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F4: List folders is project-scoped only ---------------------------------

#[tokio::test]
async fn list_folders_scoped_to_project() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-list-1").await;

    let proj_a = client
        .create_project(&ws.slug, project_req("ProjA", "proj-a"))
        .await
        .expect("proj a");

    let proj_b = client
        .create_project(&ws.slug, project_req("ProjB", "proj-b"))
        .await
        .expect("proj b");

    client
        .create_folder(
            &ws.slug,
            &proj_a.slug,
            CreateFolderRequest {
                name: "FolderA".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("folder a");

    client
        .create_folder(
            &ws.slug,
            &proj_b.slug,
            CreateFolderRequest {
                name: "FolderB".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("folder b");

    let page = client
        .list_folders(&ws.slug, &proj_a.slug, None, None)
        .await
        .expect("list");

    assert_eq!(page.items.len(), 1, "should only see proj_a folders");
    assert_eq!(page.items[0].name, "FolderA");

    db.teardown().await;
}

// ---- Listing pages through the project's folders via the cursor -----------------

#[tokio::test]
async fn list_folders_paginates_with_cursor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-page-1").await;

    let project = client
        .create_project(&ws.slug, project_req("PageProj", "page-proj"))
        .await
        .expect("project");

    for n in 0..3 {
        client
            .create_folder(
                &ws.slug,
                &project.slug,
                CreateFolderRequest {
                    name: format!("Folder{n}"),
                    parent_folder_id: None,
                },
            )
            .await
            .expect("folder");
    }

    let page1 = client
        .list_folders(&ws.slug, &project.slug, None, Some(2))
        .await
        .expect("page 1");

    assert_eq!(page1.items.len(), 2, "first page honours the limit");
    assert!(page1.has_more, "more folders remain");
    let cursor = page1.next_cursor.expect("page 1 must carry a cursor");

    let page2 = client
        .list_folders(&ws.slug, &project.slug, Some(&cursor), Some(2))
        .await
        .expect("page 2");

    assert_eq!(page2.items.len(), 1, "second page holds the remainder");
    assert!(!page2.has_more, "no more pages");
    assert!(page2.next_cursor.is_none(), "last page has no cursor");

    let mut names: Vec<String> = page1
        .items
        .iter()
        .chain(page2.items.iter())
        .map(|f| f.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["Folder0", "Folder1", "Folder2"],
        "every folder appears exactly once"
    );

    db.teardown().await;
}

// ---- Listing returns nested folders, not only root-level ones -------------------

#[tokio::test]
async fn list_folders_includes_nested() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-nested-1").await;

    let project = client
        .create_project(&ws.slug, project_req("NestProj", "nest-proj-1"))
        .await
        .expect("create project");

    let parent = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Parent".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create parent");

    client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Child".to_string(),
                parent_folder_id: Some(parent.id),
            },
        )
        .await
        .expect("create child");

    let page = client
        .list_folders(&ws.slug, &project.slug, None, None)
        .await
        .expect("list");

    assert_eq!(
        page.items.len(),
        2,
        "list must include the nested child folder"
    );
    let child = page
        .items
        .iter()
        .find(|f| f.name == "Child")
        .expect("child present in list");
    assert_eq!(child.parent_folder_id, Some(parent.id));

    db.teardown().await;
}

// ---- REQ-F5: Get folder cross-tenant → 404 (concealment) ------------------------

#[tokio::test]
async fn get_folder_cross_tenant_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client_a, ws_a, _) = support::login_user_with_workspace(&server, &db, "folder-ct-a").await;
    let (client_b, ws_b, _) = support::login_user_with_workspace(&server, &db, "folder-ct-b").await;

    let proj_a = client_a
        .create_project(&ws_a.slug, project_req("CTProj", "ct-proj"))
        .await
        .expect("proj a");

    let folder_a = client_a
        .create_folder(
            &ws_a.slug,
            &proj_a.slug,
            CreateFolderRequest {
                name: "SecretFolder".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("folder a");

    let err = client_b
        .get_folder(&ws_b.slug, folder_a.id)
        .await
        .expect_err("cross-tenant should be 404");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 404, "expected 404, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F6: Rename folder -------------------------------------------------------

#[tokio::test]
async fn rename_folder_returns_updated_dto() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-rename-1").await;

    let project = client
        .create_project(&ws.slug, project_req("RenameProj", "rename-proj"))
        .await
        .expect("create project");

    let folder = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "OldName".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create folder");

    let updated = client
        .rename_folder(
            &ws.slug,
            folder.id,
            RenameFolderRequest {
                name: "NewName".to_string(),
            },
        )
        .await
        .expect("rename folder");

    assert_eq!(updated.name, "NewName");
    assert_eq!(updated.id, folder.id);

    db.teardown().await;
}

// ---- REQ-F6b: Rename blank → 422 -------------------------------------------------

#[tokio::test]
async fn rename_folder_blank_name_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "folder-rename-blank").await;

    let project = client
        .create_project(&ws.slug, project_req("RenameBlank", "rename-blank-proj"))
        .await
        .expect("create project");

    let folder = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Valid".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create folder");

    let err = client
        .rename_folder(
            &ws.slug,
            folder.id,
            RenameFolderRequest {
                name: "".to_string(),
            },
        )
        .await
        .expect_err("blank rename should fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 422, "expected 422, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F7: Move folder — cycle → 422 ------------------------------------------

#[tokio::test]
async fn move_folder_self_cycle_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-cycle-1").await;

    let project = client
        .create_project(&ws.slug, project_req("CycleProj", "cycle-proj"))
        .await
        .expect("create project");

    let folder = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "SelfCycle".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create folder");

    let err = client
        .move_folder(
            &ws.slug,
            folder.id,
            MoveFolderRequest {
                parent_folder_id: Some(folder.id),
            },
        )
        .await
        .expect_err("self-cycle should fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 422, "expected 422, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F7b: Move folder — descendant cycle → 422 -------------------------------

#[tokio::test]
async fn move_folder_descendant_cycle_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "folder-desc-cycle").await;

    let project = client
        .create_project(&ws.slug, project_req("DescCycle", "desc-cycle-proj"))
        .await
        .expect("create project");

    let parent = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Parent".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("parent");

    let child = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Child".to_string(),
                parent_folder_id: Some(parent.id),
            },
        )
        .await
        .expect("child");

    let err = client
        .move_folder(
            &ws.slug,
            parent.id,
            MoveFolderRequest {
                parent_folder_id: Some(child.id),
            },
        )
        .await
        .expect_err("descendant cycle should fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 422, "expected 422, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F7c: Move folder to null parent = project-root -------------------------

#[tokio::test]
async fn move_folder_null_parent_moves_to_root() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-to-root").await;

    let project = client
        .create_project(&ws.slug, project_req("ToRoot", "to-root-proj"))
        .await
        .expect("create project");

    let parent = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Parent".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("parent");

    let child = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Child".to_string(),
                parent_folder_id: Some(parent.id),
            },
        )
        .await
        .expect("child");

    let moved = client
        .move_folder(
            &ws.slug,
            child.id,
            MoveFolderRequest {
                parent_folder_id: None,
            },
        )
        .await
        .expect("move to root");

    assert!(
        moved.parent_folder_id.is_none(),
        "parent should be null after move to root"
    );

    db.teardown().await;
}

// ---- REQ-F8: Soft-delete → 204, then 404 ----------------------------------------

#[tokio::test]
async fn delete_folder_then_get_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-delete-1").await;

    let project = client
        .create_project(&ws.slug, project_req("DeleteProj", "delete-proj"))
        .await
        .expect("create project");

    let folder = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "ToDelete".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create folder");

    client
        .delete_folder(&ws.slug, folder.id)
        .await
        .expect("delete folder");

    let err = client
        .get_folder(&ws.slug, folder.id)
        .await
        .expect_err("get after delete should be 404");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 404, "expected 404, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F9: Non-UUID folder_id → 422 (not 500) ---------------------------------

#[tokio::test]
async fn get_folder_non_uuid_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "folder-non-uuid").await;

    let response = client
        .http_client()
        .get(format!(
            "{}/v1/workspaces/{}/folders/not-a-uuid",
            client.base_url(),
            ws.slug
        ))
        .bearer_auth(client.token().unwrap_or(""))
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status().as_u16(),
        422,
        "non-UUID folder_id must return 422"
    );

    db.teardown().await;
}

// ---- REQ-F5 (move authz): cross-workspace destination → 404 (no cross-tenant write) ----------

#[tokio::test]
async fn move_folder_cross_workspace_destination_returns_404() {
    use atlas_domain::entities::workspace_core::NewFolder;
    use atlas_server::persistence::repos::FolderRepo;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client_a, ws_a, user_a) =
        support::login_user_with_workspace(&server, &db, "mv-authz-a").await;
    let (_, ws_b, user_b) = support::login_user_with_workspace(&server, &db, "mv-authz-b").await;

    let proj_a = client_a
        .create_project(&ws_a.slug, project_req("MvProjA", "mv-proj-a"))
        .await
        .expect("proj a");

    let ctx_b = WorkspaceCtx::new(ws_b.id, Actor::User(user_b.id));
    let folder_b = db
        .folder_repo()
        .create(
            &ctx_b,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "WsBFolder".to_string(),
            },
        )
        .await
        .expect("seed ws_b folder");

    let _ = user_a;

    let source = client_a
        .create_folder(
            &ws_a.slug,
            &proj_a.slug,
            CreateFolderRequest {
                name: "Source".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("source folder");

    let err = client_a
        .move_folder(
            &ws_a.slug,
            source.id,
            MoveFolderRequest {
                parent_folder_id: Some(folder_b.id.0),
            },
        )
        .await
        .expect_err("cross-workspace destination must fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 404, "expected 404, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    let still_same = client_a
        .get_folder(&ws_a.slug, source.id)
        .await
        .expect("get source after attempted move");

    assert!(
        still_same.parent_folder_id.is_none(),
        "source folder parent must not have changed after rejected cross-workspace move"
    );

    db.teardown().await;
}

// ---- REQ-F5 (move authz): under-privileged destination → 404 (conceal) --------------
//
// The source folder lives in a workspace-visible project (caller has editor via visibility).
// The destination folder lives in a PRIVATE project the caller has no access to.
// After the fix, move must return 404 instead of succeeding.

#[tokio::test]
async fn move_folder_underprivileged_destination_returns_404() {
    use atlas_api::dtos::LoginRequest;
    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_server::auth::password;
    use atlas_server::persistence::repos::{
        MembershipRepo, NewUser, PermissionGrantRepo, PgPermissionGrantRepo, UserRepo,
    };

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "mv-unpriv-owner").await;

    // Source lives in a workspace-visible project so the caller gets editor through visibility.
    let src_proj = owner
        .create_project(&ws.slug, project_req("MvUnprivSrcProj", "mv-unpriv-src"))
        .await
        .expect("source project");

    // Destination lives in a PRIVATE project — the caller has no access to it.
    let dst_proj = owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "MvUnprivDstProj".to_string(),
                slug: "mv-unpriv-dst".to_string(),
                task_prefix: "MUD".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("private destination project");

    let source = owner
        .create_folder(
            &ws.slug,
            &src_proj.slug,
            CreateFolderRequest {
                name: "Source".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("source folder");

    let dest = owner
        .create_folder(
            &ws.slug,
            &dst_proj.slug,
            CreateFolderRequest {
                name: "Destination".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("destination folder");

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");
    let caller_domain_user = db
        .user_repo()
        .create(NewUser {
            username: "mv-unpriv-caller".to_string(),
            display_name: "mv-unpriv-caller".to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create caller user");

    support::activate_user_in_db(&db, caller_domain_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(caller_domain_user.id));
    db.membership_repo()
        .add(&ctx, caller_domain_user.id, MemberRole::Member)
        .await
        .expect("membership");

    // Explicit editor grant on the source folder so the Authorized extractor lets the caller through.
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(caller_domain_user.id),
            api_key_id: None,
            project_id: None,
            folder_id: Some(atlas_domain::ids::FolderId(source.id)),
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("editor grant on source folder");

    let mut caller_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    caller_client
        .login(LoginRequest {
            username: "mv-unpriv-caller".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login caller");

    let err = caller_client
        .move_folder(
            &ws.slug,
            source.id,
            MoveFolderRequest {
                parent_folder_id: Some(dest.id),
            },
        )
        .await
        .expect_err("move into private destination must fail with 404");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 404, "expected 404, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- REQ-F5 (move authz): happy-path — workspace owner moves to any dest → 200 -----

#[tokio::test]
async fn move_folder_owner_to_any_dest_returns_200() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "mv-happy-owner").await;

    let proj = client
        .create_project(&ws.slug, project_req("MvHappyProj", "mv-happy-proj"))
        .await
        .expect("project");

    let dest = client
        .create_folder(
            &ws.slug,
            &proj.slug,
            CreateFolderRequest {
                name: "Destination".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("dest");

    let source = client
        .create_folder(
            &ws.slug,
            &proj.slug,
            CreateFolderRequest {
                name: "Source".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("source");

    let moved = client
        .move_folder(
            &ws.slug,
            source.id,
            MoveFolderRequest {
                parent_folder_id: Some(dest.id),
            },
        )
        .await
        .expect("move should succeed for workspace owner");

    assert_eq!(
        moved.parent_folder_id,
        Some(dest.id),
        "parent should be updated to dest"
    );

    db.teardown().await;
}

// ---- REQ-F1 (registry): all folder routes declared in ROUTE_REGISTRY -----------

#[test]
fn folder_routes_wired_in_registry_and_router() {
    use atlas_server::routes::registry::ROUTE_REGISTRY;

    let expected_openapi_paths = [
        "/v1/workspaces/{ws}/projects/{project_slug}/folders",
        "/v1/workspaces/{ws}/folders/{folder_id}",
        "/v1/workspaces/{ws}/folders/{folder_id}/move",
    ];

    for path in &expected_openapi_paths {
        let found = ROUTE_REGISTRY.iter().any(|e| e.openapi_path == Some(path));
        assert!(
            found,
            "folder OpenAPI path {path} missing from ROUTE_REGISTRY"
        );
    }

    let expected_methods = [
        (
            "POST",
            "/v1/workspaces/{ws}/projects/{project_slug}/folders",
        ),
        ("GET", "/v1/workspaces/{ws}/projects/{project_slug}/folders"),
        ("GET", "/v1/workspaces/{ws}/folders/{folder_id}"),
        ("PATCH", "/v1/workspaces/{ws}/folders/{folder_id}"),
        ("DELETE", "/v1/workspaces/{ws}/folders/{folder_id}"),
        ("PATCH", "/v1/workspaces/{ws}/folders/{folder_id}/move"),
    ];

    for (method, openapi_path) in &expected_methods {
        let found = ROUTE_REGISTRY
            .iter()
            .any(|e| e.method == *method && e.openapi_path == Some(openapi_path));
        assert!(found, "registry missing {method} {openapi_path}");
    }
}
