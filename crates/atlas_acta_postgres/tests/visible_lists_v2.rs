#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Container-backed tests for the additive V2 list reads (D-E7S3-3,
//! ACTA-AUTHZ-3): every V2 list keeps its V1 liveness and filters, pages
//! exactly through its id cursor, agrees with the V1 list or search where
//! both see everything, and under a rule predicate returns exactly the live
//! rows `permits` allows.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::ids::{FolderId, ProjectId, WorkspaceId};
use atlas_acta::ports::documents::{DocumentRepo, FolderPresence};
use atlas_acta::ports::search::SearchRepo;
use atlas_acta::search::{SearchQuery, SearchSort, TypeSet};
use atlas_acta_postgres::repos::documents::PgDocumentRepo;
use atlas_acta_postgres::repos::identity::{NewWorkspace, PgWorkspaceRepo, WorkspaceRepo};
use atlas_acta_postgres::repos::search::PgSearchRepo;
use atlas_acta_postgres::repos::visible_lists::{Page, PgVisibleListRepo};
use atlas_core::ids::ResourcePath;
use atlas_core::principal::{Principal, UserId};
use atlas_core::visibility::ListVisibility;
use atlas_custos::entities::identity::NewUser;
use atlas_custos::eval::{GrantTarget, RuleEffect, VisibilityPredicate, VisibilityRule};
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

const LIMITS: [u64; 3] = [1, 2, 50];

/// A fixed tree in one workspace, plus a second workspace whose rows no
/// list of the first may return.
struct Tree {
    owner: Uuid,
    workspace: Uuid,
    p1: Uuid,
    p2: Uuid,
    fa: Uuid,
    fb: Uuid,
    fc: Uuid,
    d1: Uuid,
    d2: Uuid,
    d3: Uuid,
    d4: Uuid,
    b1: Uuid,
    b2: Uuid,
    t1: Uuid,
    t2: Uuid,
}

async fn exec(db: &TestDb, sql: String) {
    db.conn()
        .execute_unprepared(&sql)
        .await
        .unwrap_or_else(|error| panic!("seed statement failed: {error}\n{sql}"));
}

async fn seed_workspace(db: &TestDb, slug: &str) -> Uuid {
    let workspace = Uuid::now_v7();
    PgWorkspaceRepo {
        conn: db.conn().clone(),
    }
    .create(NewWorkspace {
        id: WorkspaceId(workspace),
        name: slug.to_string(),
        slug: slug.to_string(),
    })
    .await
    .expect("seed workspace");

    workspace
}

async fn seed(db: &TestDb) -> Tree {
    let owner = PgUserRepo {
        conn: db.conn().clone(),
    }
    .create(NewUser {
        username: "visible-lists-owner".to_string(),
        display_name: "visible-lists-owner".to_string(),
        email: None,
        password_hash: None,
        is_root: false,
        is_system_admin: false,
    })
    .await
    .expect("seed user")
    .id
    .0;
    let workspace = seed_workspace(db, "visible-lists").await;
    let other = seed_workspace(db, "visible-lists-other").await;
    let id = || Uuid::now_v7();
    let tree = Tree {
        owner,
        workspace,
        p1: id(),
        p2: id(),
        fa: id(),
        fb: id(),
        fc: id(),
        d1: Uuid::nil(),
        d2: Uuid::nil(),
        d3: Uuid::nil(),
        d4: Uuid::nil(),
        b1: id(),
        b2: id(),
        t1: id(),
        t2: id(),
    };
    let Tree {
        p1,
        p2,
        fa,
        fb,
        fc,
        b1,
        b2,
        t1,
        t2,
        ..
    } = tree;
    let (trashed_folder, trashed_board) = (id(), id());
    let (c1, c2, c3, subtask, hidden_task) = (id(), id(), id(), id(), id());
    let other_project = id();
    let ws = workspace;
    let u = owner;

    exec(
        db,
        format!(
            "INSERT INTO acta.workspace_memberships (id, workspace_id, user_id, role, created_at, updated_at) \
             VALUES ('{}', '{ws}', '{u}', 'owner', now(), now()); \
             INSERT INTO acta.projects (id, workspace_id, name, slug, task_prefix, next_task_number, visibility, \
                 created_by_user_id, created_at, updated_at) \
             VALUES ('{p1}', '{ws}', 'P1', 'p1', 'PA', 1, 'workspace', '{u}', now(), now()), \
                    ('{p2}', '{ws}', 'P2', 'p2', 'PB', 1, 'private', '{u}', now(), now()), \
                    ('{other_project}', '{other}', 'P9', 'p9', 'PZ', 1, 'workspace', '{u}', now(), now()); \
             INSERT INTO acta.folders (id, workspace_id, project_id, parent_folder_id, name, created_by_user_id, \
                 created_at, updated_at) \
             VALUES ('{fa}', '{ws}', '{p1}', NULL, 'FA', '{u}', now(), now()), \
                    ('{fb}', '{ws}', '{p1}', '{fa}', 'FB', '{u}', now(), now()), \
                    ('{fc}', '{ws}', NULL, NULL, 'FC', '{u}', now(), now()); \
             INSERT INTO acta.folders (id, workspace_id, project_id, name, created_by_user_id, created_at, \
                 updated_at, deleted_at) \
             VALUES ('{trashed_folder}', '{ws}', '{p1}', 'FT', '{u}', now(), now(), now()); \
             INSERT INTO acta.boards (id, workspace_id, project_id, folder_id, name, created_by_user_id, \
                 created_at, updated_at) \
             VALUES ('{b1}', '{ws}', '{p1}', NULL, 'B1', '{u}', now(), now()), \
                    ('{b2}', '{ws}', '{p1}', '{fa}', 'B2', '{u}', now(), now()); \
             INSERT INTO acta.boards (id, workspace_id, project_id, name, created_by_user_id, created_at, \
                 updated_at, deleted_at) \
             VALUES ('{trashed_board}', '{ws}', '{p2}', 'B3', '{u}', now(), now(), now()); \
             INSERT INTO acta.board_columns (id, workspace_id, board_id, name, position_key, created_by_user_id, \
                 created_at, updated_at) \
             VALUES ('{c1}', '{ws}', '{b1}', 'Todo', 'a0', '{u}', now(), now()), \
                    ('{c2}', '{ws}', '{b2}', 'Todo', 'a0', '{u}', now(), now()), \
                    ('{c3}', '{ws}', '{trashed_board}', 'Todo', 'a0', '{u}', now(), now()); \
             INSERT INTO acta.tasks (id, workspace_id, project_id, board_id, column_id, parent_task_id, \
                 readable_id, title, position_key, created_by_user_id, created_at, updated_at) \
             VALUES ('{t1}', '{ws}', '{p1}', '{b1}', '{c1}', NULL, 'PA-1', 'T1', 'a0', '{u}', now(), now()), \
                    ('{t2}', '{ws}', '{p1}', '{b2}', '{c2}', NULL, 'PA-2', 'T2', 'a1', '{u}', now(), now()), \
                    ('{subtask}', '{ws}', '{p1}', '{b1}', '{c1}', '{t1}', 'PA-3', 'T3', 'a2', '{u}', now(), now()), \
                    ('{hidden_task}', '{ws}', '{p2}', '{trashed_board}', '{c3}', NULL, 'PB-1', 'T4', 'a0', \
                     '{u}', now(), now())",
            Uuid::now_v7(),
        ),
    )
    .await;

    let documents = [
        (ws, Some(p1), Some(fb), "D1"),
        (ws, Some(p1), None, "D2"),
        (ws, None, None, "D3"),
        (ws, None, Some(fc), "D4"),
        (ws, Some(p1), Some(trashed_folder), "D5"),
        (ws, Some(p2), None, "D6"),
        (other, Some(other_project), None, "D9"),
    ];
    let mut created: Vec<Uuid> = Vec::new();
    for (workspace, project, folder, title) in documents {
        created.push(create_document(db, owner, workspace, project, folder, title).await);
    }
    exec(
        db,
        format!(
            "UPDATE acta.documents SET deleted_at = now() WHERE id = '{}'",
            created[5]
        ),
    )
    .await;

    Tree {
        d1: created[0],
        d2: created[1],
        d3: created[2],
        d4: created[3],
        ..tree
    }
}

/// A document created through the repository, so it carries the revision
/// the list mapping requires.
async fn create_document(
    db: &TestDb,
    owner: Uuid,
    workspace: Uuid,
    project: Option<Uuid>,
    folder: Option<Uuid>,
    title: &str,
) -> Uuid {
    let ctx = WorkspaceCtx::new(
        WorkspaceId(workspace),
        Actor::User(UserAttributionId(owner)),
    );

    PgDocumentRepo::new(db.conn().clone(), 50)
        .create(
            &ctx,
            NewDocument {
                title: title.to_string(),
                slug: None,
                content: String::new(),
                folder_id: folder.map(FolderId),
                project_id: project.map(ProjectId),
                frontmatter: None,
            },
        )
        .await
        .expect("seed document")
        .id
        .0
}

fn ctx(tree: &Tree) -> WorkspaceCtx {
    WorkspaceCtx::new(
        WorkspaceId(tree.workspace),
        Actor::User(UserAttributionId(tree.owner)),
    )
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

/// The live rows of each list with the paths Acta's provider reports.
fn live_rows(tree: &Tree) -> [(&'static str, Vec<(Uuid, String)>); 5] {
    let ws = format!("acta::workspace::{}", tree.workspace);
    let p1 = format!("{ws}/project::{}", tree.p1);
    let fa = format!("{p1}/folder::{}", tree.fa);
    let fb = format!("{fa}/folder::{}", tree.fb);
    let fc = format!("{ws}/folder::{}", tree.fc);
    let b1 = format!("{p1}/board::{}", tree.b1);
    let b2 = format!("{fa}/board::{}", tree.b2);

    [
        (
            "projects",
            vec![
                (tree.p1, p1.clone()),
                (tree.p2, format!("{ws}/project::{}", tree.p2)),
            ],
        ),
        (
            "folders",
            vec![
                (tree.fa, fa.clone()),
                (tree.fb, fb.clone()),
                (tree.fc, fc.clone()),
            ],
        ),
        (
            "documents",
            vec![
                (tree.d1, format!("{fb}/document::{}", tree.d1)),
                (tree.d2, format!("{p1}/document::{}", tree.d2)),
                (tree.d3, format!("{ws}/document::{}", tree.d3)),
                (tree.d4, format!("{fc}/document::{}", tree.d4)),
            ],
        ),
        ("boards", vec![(tree.b1, b1.clone()), (tree.b2, b2.clone())]),
        (
            "tasks",
            vec![
                (tree.t1, format!("{b1}/task::{}", tree.t1)),
                (tree.t2, format!("{b2}/task::{}", tree.t2)),
            ],
        ),
    ]
}

/// One page of the named V2 list.
async fn v2_page(
    db: &TestDb,
    tree: &Tree,
    list: &str,
    visibility: &ListVisibility,
    page: Page,
) -> Vec<Uuid> {
    let repo = PgVisibleListRepo {
        conn: db.conn().clone(),
    };
    let ctx = ctx(tree);

    match list {
        "projects" => repo
            .list_projects_v2(&ctx, visibility, page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        "folders" => repo
            .list_folders_v2(&ctx, visibility, None, page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        "documents" => PgDocumentRepo::new(db.conn().clone(), 50)
            .list_visible_v2_with_folder_presence(
                &ctx,
                visibility,
                None,
                FolderPresence::Any,
                page.after_id,
                page.limit,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        "boards" => repo
            .list_boards_v2(&ctx, visibility, None, page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        "tasks" => repo
            .list_tasks_v2(&ctx, visibility, None, page)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.id.0)
            .collect(),
        other => panic!("unknown list {other}"),
    }
}

/// Every page of the named V2 list at `limit`, concatenated.
async fn v2_all_pages(
    db: &TestDb,
    tree: &Tree,
    list: &str,
    visibility: &ListVisibility,
    limit: u64,
) -> Vec<Uuid> {
    let mut seen: Vec<Uuid> = Vec::new();

    loop {
        let page = Page {
            after_id: seen.last().copied(),
            limit,
        };
        let rows = v2_page(db, tree, list, visibility, page).await;
        let done = (rows.len() as u64) < limit;
        seen.extend(rows);

        if done {
            return seen;
        }
    }
}

#[tokio::test]
async fn every_v2_list_under_all_returns_its_live_rows_page_by_page() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed(&db).await;

    for (list, rows) in live_rows(&tree) {
        let expected = sorted(rows.iter().map(|(id, _)| *id).collect());

        for limit in LIMITS {
            assert_eq!(
                v2_all_pages(&db, &tree, list, &ListVisibility::All, limit).await,
                expected,
                "{list} at limit {limit}"
            );
        }
        assert!(
            v2_page(
                &db,
                &tree,
                list,
                &ListVisibility::Nothing,
                Page {
                    after_id: None,
                    limit: 50
                }
            )
            .await
            .is_empty(),
            "{list} under Nothing"
        );
    }

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn a_rule_predicate_selects_exactly_the_permitted_live_rows_of_every_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed(&db).await;
    let ws = format!("acta::workspace::{}", tree.workspace);
    let target = |raw: String| -> GrantTarget { GrantTarget::Ref(raw.parse().unwrap()) };
    let predicate = VisibilityPredicate::Rules {
        grants: vec![
            VisibilityRule {
                target: target(format!("acta::folder::{}", tree.fa)),
                effect: RuleEffect::Allow,
            },
            VisibilityRule {
                target: target(format!("acta::folder::{}", tree.fb)),
                effect: RuleEffect::Block,
            },
            VisibilityRule {
                target: GrantTarget::Path(
                    format!("{ws}/project::{}/board::{}", tree.p1, tree.b1)
                        .parse()
                        .unwrap(),
                ),
                effect: RuleEffect::Allow,
            },
            VisibilityRule {
                target: GrantTarget::Selector(
                    format!("{ws}/folder::{}/**", tree.fc).parse().unwrap(),
                ),
                effect: RuleEffect::Allow,
            },
        ],
        denies: vec![target(format!("acta::board::{}", tree.b2))],
    };
    let visibility = ListVisibility::from(&predicate);

    for (list, rows) in live_rows(&tree) {
        let expected = sorted(
            rows.iter()
                .filter(|(_, path)| predicate.permits(&path.parse::<ResourcePath>().unwrap()))
                .map(|(id, _)| *id)
                .collect(),
        );

        for limit in LIMITS {
            assert_eq!(
                v2_all_pages(&db, &tree, list, &visibility, limit).await,
                expected,
                "{list} at limit {limit}"
            );
        }
    }

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn the_v2_document_list_under_all_matches_the_v1_list_of_a_workspace_owner() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed(&db).await;
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let ctx = ctx(&tree);
    let owner = Principal::User(UserId(tree.owner));

    for project in [None, Some(ProjectId(tree.p1)), Some(ProjectId(tree.p2))] {
        for presence in [
            FolderPresence::Any,
            FolderPresence::Filed,
            FolderPresence::Unfiled,
        ] {
            for limit in LIMITS {
                let v1 = repo
                    .list_visible_with_folder_presence(&ctx, &owner, project, presence, None, limit)
                    .await
                    .unwrap();
                let v2 = repo
                    .list_visible_v2_with_folder_presence(
                        &ctx,
                        &ListVisibility::All,
                        project,
                        presence,
                        None,
                        limit,
                    )
                    .await
                    .unwrap();

                assert_eq!(
                    v2.iter().map(|row| row.id).collect::<Vec<_>>(),
                    v1.iter().map(|row| row.id).collect::<Vec<_>>(),
                    "project {project:?}, {presence:?}, limit {limit}"
                );
            }
        }
    }

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn v2_search_under_all_matches_the_v1_search_that_sees_everything() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed(&db).await;
    let repo = PgSearchRepo::new(db.conn().clone());
    let ctx = ctx(&tree);
    let query = SearchQuery {
        text: String::new(),
        filters: Vec::new(),
        sort: SearchSort::UpdatedDesc,
        type_filter: TypeSet::default(),
        warnings: Vec::new(),
        prefix: false,
    };

    let v1 = repo
        .search(
            &ctx,
            &Principal::User(UserId(tree.owner)),
            &query,
            100,
            None,
            true,
            true,
            true,
        )
        .await
        .unwrap();
    let v2 = repo
        .search_v2(
            &ctx,
            &query,
            100,
            None,
            &ListVisibility::All,
            &ListVisibility::All,
        )
        .await
        .unwrap();
    let nothing = repo
        .search_v2(
            &ctx,
            &query,
            100,
            None,
            &ListVisibility::Nothing,
            &ListVisibility::Nothing,
        )
        .await
        .unwrap();

    assert!(!v1.is_empty());
    assert_eq!(
        v2.iter().map(|hit| hit.id).collect::<Vec<_>>(),
        v1.iter().map(|hit| hit.id).collect::<Vec<_>>()
    );
    assert!(nothing.is_empty());

    let task_only = repo
        .search_v2(
            &ctx,
            &query,
            100,
            None,
            &ListVisibility::Nothing,
            &ListVisibility::All,
        )
        .await
        .unwrap();
    assert!(!task_only.is_empty());
    assert!(
        task_only
            .iter()
            .all(|hit| hit.kind == atlas_acta::search::SearchKind::Task),
        "only the task arm runs"
    );

    db.teardown().await.expect("teardown");
}

/// A folder whose chain never reaches a root folder, through a cycle or a
/// chain deeper than the walk follows, has no path: rule predicates hide it,
/// as Acta's provider reports it missing. `All` still lists it, since the
/// V1 liveness it shares does not require a root.
#[tokio::test]
async fn a_folder_without_a_reachable_root_is_hidden_under_rules() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed(&db).await;
    let (ws, p1, u) = (tree.workspace, tree.p1, tree.owner);
    let (cycle_a, cycle_b) = (Uuid::now_v7(), Uuid::now_v7());
    let chain: Vec<Uuid> = (0..66).map(|_| Uuid::now_v7()).collect();

    let mut statements = vec![format!(
        "INSERT INTO acta.folders (id, workspace_id, project_id, parent_folder_id, name, created_by_user_id, \
             created_at, updated_at) \
         VALUES ('{cycle_a}', '{ws}', '{p1}', NULL, 'Cycle A', '{u}', now(), now()), \
                ('{cycle_b}', '{ws}', '{p1}', '{cycle_a}', 'Cycle B', '{u}', now(), now()); \
         UPDATE acta.folders SET parent_folder_id = '{cycle_b}' WHERE id = '{cycle_a}'"
    )];
    for (depth, folder) in chain.iter().enumerate() {
        let parent = depth
            .checked_sub(1)
            .map_or_else(|| "NULL".to_string(), |above| format!("'{}'", chain[above]));
        statements.push(format!(
            "INSERT INTO acta.folders (id, workspace_id, project_id, parent_folder_id, name, \
                 created_by_user_id, created_at, updated_at) \
             VALUES ('{folder}', '{ws}', '{p1}', {parent}, 'Deep {depth}', '{u}', now(), now())"
        ));
    }
    exec(&db, statements.join("; ")).await;

    let workspace_grant = ListVisibility::Rules {
        grants: vec![atlas_core::visibility::VisibilityGrant {
            target: atlas_core::visibility::VisibilityTarget::Ref(
                format!("acta::workspace::{ws}").parse().unwrap(),
            ),
            allow: true,
        }],
        denies: Vec::new(),
    };
    let under_rules = v2_all_pages(&db, &tree, "folders", &workspace_grant, 200).await;
    let under_all = v2_all_pages(&db, &tree, "folders", &ListVisibility::All, 200).await;

    for hidden in [cycle_a, cycle_b, chain[64], chain[65]] {
        assert!(!under_rules.contains(&hidden), "{hidden} has no path");
        assert!(under_all.contains(&hidden), "{hidden} is live");
    }
    for shown in [chain[0], chain[63], tree.fa, tree.fb, tree.fc] {
        assert!(under_rules.contains(&shown), "{shown} reaches its root");
    }

    db.teardown().await.expect("teardown");
}
