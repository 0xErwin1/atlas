#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::NewBoard,
    entities::identity::MemberRole,
    entities::permissions::NewPermissionGrant,
    entities::workspace_core::{NewFolder, NewProject},
    ids::{BoardId, FolderId, ProjectId},
    permissions::{Principal, ResolutionInput, ResourceRef, ResourceRole, Visibility},
    ports::permission_grant_repo::ResolutionQuery,
};
use atlas_server::{
    authz::authorized::{BoardRes, ResolvedResource},
    persistence::repos::{
        BoardRepo, FolderRepo, MembershipRepo, PermissionGrantRepo, PgBoardRepo, PgFolderRepo,
        PgMembershipRepo, PgPermissionGrantRepo, PgProjectRepo, ProjectRepo, UserRepo,
    },
};

fn board_params(id: BoardId) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("board_id".to_string(), id.0.to_string());
    map
}

async fn create_project(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    slug: &str,
) -> ProjectId {
    let repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(ws, user);
    let project = repo
        .create(
            &ctx,
            NewProject {
                name: format!("Project {slug}"),
                slug: slug.to_string(),
                task_prefix: "BRD".to_string(),
                visibility: Visibility::Private,
            },
        )
        .await
        .expect("create project");
    project.id
}

async fn create_folder(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
) -> atlas_domain::entities::workspace_core::Folder {
    let repo = PgFolderRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(ws, user);
    repo.create(
        &ctx,
        NewFolder {
            project_id: None,
            parent_folder_id: None,
            name: "test-folder".into(),
        },
    )
    .await
    .expect("create folder")
}

async fn create_board_in_folder(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    project_id: ProjectId,
    folder_id: Option<FolderId>,
    name: &str,
) -> atlas_domain::entities::boards_tasks::Board {
    let repo = PgBoardRepo::new(db.conn().clone());
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    repo.create_board(
        &ctx,
        NewBoard {
            project_id,
            folder_id,
            name: name.to_string(),
        },
    )
    .await
    .expect("create board")
}

#[tokio::test]
async fn board_res_chain_includes_folder_segment_for_board_in_folder() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "boardres-folder-chain").await;

    let project_id = create_project(&db, &ws, &user, "boardres-folder-chain").await;
    let folder = create_folder(&db, &ws, &user).await;
    let board =
        create_board_in_folder(&db, &ws, &user, project_id, Some(folder.id), "Folder Board").await;

    let (_, chain) = BoardRes::resolve(db.conn(), &ws, board_params(board.id))
        .await
        .expect("resolve must succeed");

    let has_folder = chain
        .segments
        .iter()
        .any(|s| matches!(&s.resource, ResourceRef::Folder(fid) if fid.0 == folder.id.0));
    assert!(
        has_folder,
        "chain must include a Folder segment for a board inside a folder"
    );

    db.teardown().await;
}

#[tokio::test]
async fn folder_scope_grant_yields_effective_access_on_board_in_that_folder() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, owner) = support::seed_workspace(&db, "boardres-fg-eff").await;

    let user_repo = db.user_repo();
    let viewer = user_repo
        .create(atlas_server::persistence::repos::NewUser {
            username: "boardres-fg-viewer".to_string(),
            display_name: "Viewer".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create viewer");

    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(&ws, &owner);
    membership_repo
        .add(&ctx, viewer.id, MemberRole::Member)
        .await
        .expect("add viewer membership");

    let project_id = create_project(&db, &ws, &owner, "boardres-fg-eff").await;
    let folder = create_folder(&db, &ws, &owner).await;
    let board = create_board_in_folder(
        &db,
        &ws,
        &owner,
        project_id,
        Some(folder.id),
        "Folder Effective Board",
    )
    .await;

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(viewer.id),
            api_key_id: None,
            group_id: None,
            project_id: None,
            folder_id: Some(folder.id),
            document_id: None,
            board_id: None,
            role: ResourceRole::Viewer,
            created_by_user_id: Some(owner.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert folder grant");

    let (_, chain) = BoardRes::resolve(db.conn(), &ws, board_params(board.id))
        .await
        .expect("resolve must succeed");

    let chain_folder_ids: Vec<uuid::Uuid> = chain
        .segments
        .iter()
        .filter_map(|s| {
            if let ResourceRef::Folder(fid) = &s.resource {
                Some(fid.0)
            } else {
                None
            }
        })
        .collect();

    let grants = grant_repo
        .load_grants_for_resolution(ResolutionQuery {
            workspace_id: ws.id,
            user_id: Some(viewer.id.0),
            api_key_id: None,
            group_ids: vec![],
            chain_projects: vec![],
            chain_folders: chain_folder_ids,
            doc_id: None,
            board_id: None,
        })
        .await
        .expect("load_grants_for_resolution");

    let principal = Principal::User(viewer.id);
    let input = ResolutionInput {
        principal: &principal,
        membership: Some(MemberRole::Member),
        chain: &chain,
        grants: &grants,
    };

    let effective = atlas_domain::permissions::resolve(&input);

    assert!(
        effective.is_some(),
        "folder-scope grant must yield effective access on a board in that folder, got None"
    );
    assert!(
        effective.unwrap() >= ResourceRole::Viewer,
        "effective role must be at least Viewer, got: {:?}",
        effective
    );

    db.teardown().await;
}
