#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Container-backed equivalence test for the V2 visibility translation
//! (D-E7S3-2a, ACTA-AUTHZ-3, EVAL-5): on randomly shaped Acta trees and
//! randomly drawn rule sets, the SQL condition selects a row exactly when
//! `VisibilityPredicate::permits` allows the row's path, for every listed
//! kind, with and without enforced denies. The path SQL itself is checked
//! against the path the tree was built with.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.
//! The generator is a fixed-seed xorshift, so a failure names its seed and
//! case and replays identically.

use std::collections::HashMap;

use atlas_acta::ids::WorkspaceId;
use atlas_acta_postgres::repos::identity::{NewWorkspace, PgWorkspaceRepo, WorkspaceRepo};
use atlas_acta_postgres::visibility_sql::{VisibleKind, path_sql, predicate_to_sql};
use atlas_core::ids::ResourcePath;
use atlas_core::visibility::ListVisibility;
use atlas_custos::entities::identity::NewUser;
use atlas_custos::eval::{GrantTarget, RuleEffect, VisibilityPredicate, VisibilityRule};
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};
use uuid::Uuid;

const SEEDS: [u64; 2] = [0x5eed_0001, 0x5eed_0002];
const CASES_PER_MODE: usize = 120;

const KINDS: [(VisibleKind, &str); 5] = [
    (VisibleKind::Project, "projects"),
    (VisibleKind::Folder, "folders"),
    (VisibleKind::Document, "documents"),
    (VisibleKind::Board, "boards"),
    (VisibleKind::Task, "tasks"),
];

/// A deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Every seeded row with the path Acta's provider reports for it, root
/// first, as `<kind>::<id>` segments.
#[derive(Default)]
struct Tree {
    paths: HashMap<Uuid, Vec<String>>,
    statements: Vec<String>,
    counter: usize,
}

impl Tree {
    fn next_name(&mut self) -> usize {
        self.counter += 1;
        self.counter
    }

    fn add(&mut self, id: Uuid, path: Vec<String>, statement: String) {
        self.paths.insert(id, path);
        self.statements.push(statement);
    }
}

fn segment(kind: &str, id: Uuid) -> String {
    format!("{kind}::{id}")
}

fn extended(path: &[String], kind: &str, id: Uuid) -> Vec<String> {
    let mut path = path.to_vec();
    path.push(segment(kind, id));
    path
}

/// A folder and the project its root-most folder hangs off, if any.
struct Folder {
    id: Uuid,
    project: Option<Uuid>,
    path: Vec<String>,
}

fn grow_workspace(tree: &mut Tree, rng: &mut Rng, user: Uuid, workspace: Uuid) {
    let root = vec![segment("workspace", workspace)];

    let projects: Vec<Uuid> = (0..2)
        .map(|_| {
            let id = Uuid::now_v7();
            let n = tree.next_name();
            tree.add(
                id,
                extended(&root, "project", id),
                format!(
                    "INSERT INTO acta.projects (id, workspace_id, name, slug, task_prefix, next_task_number, \
                         visibility, created_by_user_id, created_at, updated_at) \
                     VALUES ('{id}', '{workspace}', 'Project {n}', 'project-{n}', 'P{n}', 1, 'workspace', \
                         '{user}', now(), now())"
                ),
            );
            id
        })
        .collect();

    let folders = grow_folders(tree, rng, user, workspace, &root, &projects);
    grow_documents(tree, rng, user, workspace, &root, &projects, &folders);
    grow_boards(tree, rng, user, workspace, &root, &projects, &folders);
}

fn grow_folders(
    tree: &mut Tree,
    rng: &mut Rng,
    user: Uuid,
    workspace: Uuid,
    root: &[String],
    projects: &[Uuid],
) -> Vec<Folder> {
    let mut folders: Vec<Folder> = Vec::new();

    for _ in 0..3 {
        let project = if rng.chance(70) {
            Some(*rng.pick(projects))
        } else {
            None
        };
        let base = match project {
            Some(project) => extended(root, "project", project),
            None => root.to_vec(),
        };
        let id = Uuid::now_v7();
        insert_folder(tree, user, workspace, id, project, None, &base);
        folders.push(Folder {
            id,
            project,
            path: extended(&base, "folder", id),
        });
    }

    for _ in 0..5 {
        let parent = rng.below(folders.len());
        let (parent_id, project, base) = {
            let parent = &folders[parent];
            (parent.id, parent.project, parent.path.clone())
        };
        if base.len() > 5 {
            continue;
        }
        let id = Uuid::now_v7();
        insert_folder(tree, user, workspace, id, project, Some(parent_id), &base);
        folders.push(Folder {
            id,
            project,
            path: extended(&base, "folder", id),
        });
    }

    folders
}

fn insert_folder(
    tree: &mut Tree,
    user: Uuid,
    workspace: Uuid,
    id: Uuid,
    project: Option<Uuid>,
    parent: Option<Uuid>,
    base: &[String],
) {
    let n = tree.next_name();
    let project = nullable(project);
    let parent = nullable(parent);

    tree.add(
        id,
        extended(base, "folder", id),
        format!(
            "INSERT INTO acta.folders (id, workspace_id, project_id, parent_folder_id, name, \
                 created_by_user_id, created_at, updated_at) \
             VALUES ('{id}', '{workspace}', {project}, {parent}, 'Folder {n}', '{user}', now(), now())"
        ),
    );
}

fn nullable(id: Option<Uuid>) -> String {
    id.map_or_else(|| "NULL".to_string(), |id| format!("'{id}'"))
}

fn grow_documents(
    tree: &mut Tree,
    rng: &mut Rng,
    user: Uuid,
    workspace: Uuid,
    root: &[String],
    projects: &[Uuid],
    folders: &[Folder],
) {
    for _ in 0..8 {
        let id = Uuid::now_v7();
        let (project, folder, base) = match rng.below(3) {
            0 => {
                let folder = rng.pick(folders);
                (folder.project, Some(folder.id), folder.path.clone())
            }
            1 => {
                let project = *rng.pick(projects);
                (Some(project), None, extended(root, "project", project))
            }
            _ => (None, None, root.to_vec()),
        };
        let (project, folder) = (nullable(project), nullable(folder));

        tree.add(
            id,
            extended(&base, "document", id),
            format!(
                "INSERT INTO acta.documents (id, workspace_id, project_id, folder_id, title, created_by_user_id) \
                 VALUES ('{id}', '{workspace}', {project}, {folder}, 'Doc', '{user}')"
            ),
        );
    }
}

fn grow_boards(
    tree: &mut Tree,
    rng: &mut Rng,
    user: Uuid,
    workspace: Uuid,
    root: &[String],
    projects: &[Uuid],
    folders: &[Folder],
) {
    let project_folders: Vec<&Folder> = folders
        .iter()
        .filter(|folder| folder.project.is_some())
        .collect();

    for _ in 0..3 {
        let id = Uuid::now_v7();
        let (project, folder, base) = if !project_folders.is_empty() && rng.chance(50) {
            let folder = *rng.pick(&project_folders);
            (
                folder.project.unwrap(),
                Some(folder.id),
                folder.path.clone(),
            )
        } else {
            let project = *rng.pick(projects);
            (project, None, extended(root, "project", project))
        };
        let board_path = extended(&base, "board", id);
        let column = Uuid::now_v7();
        let folder_value = nullable(folder);

        tree.add(
            id,
            board_path.clone(),
            format!(
                "INSERT INTO acta.boards (id, workspace_id, project_id, folder_id, name, created_by_user_id, \
                     created_at, updated_at) \
                 VALUES ('{id}', '{workspace}', '{project}', {folder_value}, 'Board', '{user}', now(), now()); \
                 INSERT INTO acta.board_columns (id, workspace_id, board_id, name, position_key, \
                     created_by_user_id, created_at, updated_at) \
                 VALUES ('{column}', '{workspace}', '{id}', 'Todo', 'a0', '{user}', now(), now())"
            ),
        );

        let board = Board {
            id,
            project,
            column,
            path: board_path,
        };
        grow_tasks(tree, rng, user, workspace, &board);
    }
}

/// A seeded board, the project its tasks carry, and its one column.
struct Board {
    id: Uuid,
    project: Uuid,
    column: Uuid,
    path: Vec<String>,
}

fn grow_tasks(tree: &mut Tree, rng: &mut Rng, user: Uuid, workspace: Uuid, board: &Board) {
    for _ in 0..2 {
        let task = Uuid::now_v7();
        insert_task(tree, user, workspace, board, task, None);

        if rng.chance(60) {
            insert_task(tree, user, workspace, board, Uuid::now_v7(), Some(task));
        }
    }
}

fn insert_task(
    tree: &mut Tree,
    user: Uuid,
    workspace: Uuid,
    board: &Board,
    id: Uuid,
    parent: Option<Uuid>,
) {
    let n = tree.next_name();
    let parent = nullable(parent);
    let Board {
        id: board_id,
        project,
        column,
        ..
    } = board;

    tree.add(
        id,
        extended(&board.path, "task", id),
        format!(
            "INSERT INTO acta.tasks (id, workspace_id, project_id, board_id, column_id, parent_task_id, \
                 readable_id, title, position_key, created_by_user_id, created_at, updated_at) \
             VALUES ('{id}', '{workspace}', '{project}', '{board_id}', '{column}', {parent}, \
                 'T{n}-{n}', 'Task', 'a{n}', '{user}', now(), now())"
        ),
    );
}

async fn seed(db: &TestDb, rng: &mut Rng) -> Tree {
    let user = PgUserRepo {
        conn: db.conn().clone(),
    }
    .create(NewUser {
        username: "visibility-equivalence".to_string(),
        display_name: "visibility-equivalence".to_string(),
        email: None,
        password_hash: None,
        is_root: false,
        is_system_admin: false,
    })
    .await
    .expect("seed user")
    .id
    .0;

    let mut tree = Tree::default();
    for n in 0..2 {
        let workspace = Uuid::now_v7();
        PgWorkspaceRepo {
            conn: db.conn().clone(),
        }
        .create(NewWorkspace {
            id: WorkspaceId(workspace),
            name: format!("Workspace {n}"),
            slug: format!("equivalence-{n}"),
        })
        .await
        .expect("seed workspace");
        tree.paths
            .insert(workspace, vec![segment("workspace", workspace)]);

        grow_workspace(&mut tree, rng, user, workspace);
    }

    for statement in &tree.statements {
        db.conn()
            .execute_unprepared(statement)
            .await
            .unwrap_or_else(|error| panic!("seed statement failed: {error}\n{statement}"));
    }

    tree
}

/// A target drawn over the tree's real paths: a ref to one of their
/// elements, one of their prefixes as an exact path, a selector shaped
/// from a prefix with wildcards and an optional `**`, or a target of
/// another product.
fn random_target(rng: &mut Rng, paths: &[&Vec<String>]) -> GrantTarget {
    let path = *rng.pick(paths);
    let prefix = &path[..=rng.below(path.len())];

    match rng.below(10) {
        0 => GrantTarget::Ref(
            format!("custos::group::{}", Uuid::now_v7())
                .parse()
                .unwrap(),
        ),
        1..=3 => GrantTarget::Ref(format!("acta::{}", rng.pick(path)).parse().unwrap()),
        4..=5 => GrantTarget::Path(format!("acta::{}", prefix.join("/")).parse().unwrap()),
        6 => GrantTarget::Selector("acta::**".parse().unwrap()),
        _ => {
            let elements: Vec<String> = prefix
                .iter()
                .map(|element| {
                    if rng.chance(30) {
                        "*".to_string()
                    } else {
                        element.clone()
                    }
                })
                .collect();
            let open = if rng.chance(50) { "/**" } else { "" };
            GrantTarget::Selector(
                format!("acta::{}{open}", elements.join("/"))
                    .parse()
                    .unwrap(),
            )
        }
    }
}

fn random_predicate(rng: &mut Rng, paths: &[&Vec<String>], enforced: bool) -> VisibilityPredicate {
    match rng.below(40) {
        0 => return VisibilityPredicate::All,
        1 => return VisibilityPredicate::Nothing,
        _ => {}
    }

    let grants = (0..1 + rng.below(5))
        .map(|_| VisibilityRule {
            target: random_target(rng, paths),
            effect: if rng.chance(70) {
                RuleEffect::Allow
            } else {
                RuleEffect::Block
            },
        })
        .collect();
    let denies = if enforced {
        (0..rng.below(3))
            .map(|_| random_target(rng, paths))
            .collect()
    } else {
        Vec::new()
    };

    VisibilityPredicate::Rules { grants, denies }
}

#[derive(Debug, FromQueryResult)]
struct Selected {
    id: Uuid,
    path: String,
    visible: bool,
}

async fn select_with(
    db: &TestDb,
    kind: VisibleKind,
    table: &str,
    visibility: &ListVisibility,
) -> Vec<Selected> {
    let fragment = predicate_to_sql(visibility, kind, "listed", 1);
    let sql = format!(
        "SELECT listed.id AS id, array_to_string({path}, '/') AS path, ({condition}) AS visible \
         FROM acta.{table} listed ORDER BY listed.id",
        path = path_sql(kind, "listed"),
        condition = fragment.where_clause,
    );

    Selected::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        sql,
        fragment.binds,
    ))
    .all(db.conn())
    .await
    .unwrap_or_else(|error| panic!("{table}: visibility query failed: {error}"))
}

#[tokio::test]
async fn the_sql_condition_selects_exactly_the_rows_permits_allows() {
    let mut compared = 0usize;

    for seed_value in SEEDS {
        let db = TestDb::create().await.expect("TestDb::create");
        let mut rng = Rng(seed_value);
        let tree = seed(&db, &mut rng).await;
        let paths: Vec<&Vec<String>> = tree.paths.values().collect();

        for enforced in [false, true] {
            for case in 0..CASES_PER_MODE {
                let predicate = random_predicate(&mut rng, &paths, enforced);
                let visibility = ListVisibility::from(&predicate);

                for (kind, table) in KINDS {
                    for row in select_with(&db, kind, table, &visibility).await {
                        let expected = tree
                            .paths
                            .get(&row.id)
                            .unwrap_or_else(|| panic!("{table} row {} was not seeded", row.id));
                        assert_eq!(row.path, expected.join("/"), "{table} row {}: path", row.id);

                        let path: ResourcePath = format!("acta::{}", row.path).parse().unwrap();
                        assert_eq!(
                            row.visible,
                            predicate.permits(&path),
                            "seed {seed_value:#x}, enforced {enforced}, case {case}, {table} row \
                             {}: {predicate:?}",
                            row.id
                        );
                        compared += 1;
                    }
                }
            }
        }

        db.teardown().await.expect("teardown");
    }

    assert!(compared > 10_000, "only {compared} row decisions compared");
}
