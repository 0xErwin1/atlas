#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Container-backed characterization tests for Acta's resource provider over
//! `PgActaResourceStore` (ACTA-AUTHZ-1/5, D-E7-7): every Acta kind exists
//! with its spec §2 path, a trashed ancestor hides the whole subtree, share
//! links and alias ids are missing, a mixed batch keeps input order, and the
//! workspace `members` set lists every membership role.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.
//! `atlas_custos`/`atlas_custos_postgres` are dev-dependencies only, used to
//! seed the users and the API key the Acta rows reference.

use atlas_acta::ids::WorkspaceId;
use atlas_acta::provider::{ActaResourceProvider, MEMBERS_SET};
use atlas_acta_postgres::repos::identity::{NewWorkspace, PgWorkspaceRepo, WorkspaceRepo};
use atlas_acta_postgres::repos::resource_store::PgActaResourceStore;
use atlas_core::capabilities::{CapabilityError, ResourceExistence, ResourceProvider};
use atlas_core::ids::{ResourcePath, ResourceRef};
use atlas_core::registry::Authorization;
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey, NewUser};
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo, PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

async fn exec(db: &TestDb, sql: String) {
    db.conn()
        .execute_unprepared(&sql)
        .await
        .unwrap_or_else(|error| panic!("seed statement failed: {error}\n{sql}"));
}

async fn seed_user(db: &TestDb, username: &str) -> Uuid {
    PgUserRepo {
        conn: db.conn().clone(),
    }
    .create(NewUser {
        username: username.to_string(),
        display_name: username.to_string(),
        email: None,
        password_hash: None,
        is_root: false,
        is_system_admin: false,
    })
    .await
    .expect("seed user")
    .id
    .0
}

/// Every seeded Acta row, one per kind, in one workspace.
struct Tree {
    user: Uuid,
    workspace: Uuid,
    project: Uuid,
    outer: Uuid,
    inner: Uuid,
    document: Uuid,
    board: Uuid,
    column: Uuid,
    task: Uuid,
    subtask: Uuid,
    checklist_item: Uuid,
    task_comment: Uuid,
    document_comment: Uuid,
    comment_attachment: Uuid,
    document_attachment: Uuid,
    saved_search: Uuid,
    task_view: Uuid,
    tag: Uuid,
    property_definition: Uuid,
    status_template: Uuid,
    webhook: Uuid,
    automation_rule: Uuid,
    integration_config: Uuid,
}

async fn seed_tree(db: &TestDb, name: &str) -> Tree {
    let user = seed_user(db, name).await;
    let workspace = Uuid::now_v7();
    PgWorkspaceRepo {
        conn: db.conn().clone(),
    }
    .create(NewWorkspace {
        id: WorkspaceId(workspace),
        name: name.to_string(),
        slug: name.to_string(),
    })
    .await
    .expect("seed workspace");

    let tree = Tree {
        user,
        workspace,
        project: Uuid::now_v7(),
        outer: Uuid::now_v7(),
        inner: Uuid::now_v7(),
        document: Uuid::now_v7(),
        board: Uuid::now_v7(),
        column: Uuid::now_v7(),
        task: Uuid::now_v7(),
        subtask: Uuid::now_v7(),
        checklist_item: Uuid::now_v7(),
        task_comment: Uuid::now_v7(),
        document_comment: Uuid::now_v7(),
        comment_attachment: Uuid::now_v7(),
        document_attachment: Uuid::now_v7(),
        saved_search: Uuid::now_v7(),
        task_view: Uuid::now_v7(),
        tag: Uuid::now_v7(),
        property_definition: Uuid::now_v7(),
        status_template: Uuid::now_v7(),
        webhook: Uuid::now_v7(),
        automation_rule: Uuid::now_v7(),
        integration_config: Uuid::now_v7(),
    };
    let Tree {
        user,
        workspace: ws,
        project,
        outer,
        inner,
        document,
        board,
        column,
        task,
        subtask,
        checklist_item,
        task_comment,
        document_comment,
        comment_attachment,
        document_attachment,
        ..
    } = tree;

    exec(
        db,
        format!(
            "INSERT INTO acta.projects (id, workspace_id, name, slug, task_prefix, next_task_number, \
                 visibility, created_by_user_id, created_at, updated_at) \
             VALUES ('{project}', '{ws}', '{name}', '{name}', 'TSK', 1, 'workspace', '{user}', now(), now()); \
             INSERT INTO acta.folders (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{outer}', '{ws}', '{project}', 'Outer', '{user}', now(), now()); \
             INSERT INTO acta.folders (id, workspace_id, project_id, parent_folder_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{inner}', '{ws}', '{project}', '{outer}', 'Inner', '{user}', now(), now()); \
             INSERT INTO acta.documents (id, workspace_id, project_id, folder_id, title, created_by_user_id) \
             VALUES ('{document}', '{ws}', '{project}', '{inner}', 'Doc', '{user}'); \
             INSERT INTO acta.boards (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{board}', '{ws}', '{project}', 'Board', '{user}', now(), now()); \
             INSERT INTO acta.board_columns (id, workspace_id, board_id, name, position_key, created_by_user_id, created_at, updated_at) \
             VALUES ('{column}', '{ws}', '{board}', 'Todo', 'a0', '{user}', now(), now()); \
             INSERT INTO acta.tasks (id, workspace_id, project_id, board_id, column_id, readable_id, title, position_key, created_by_user_id, created_at, updated_at) \
             VALUES ('{task}', '{ws}', '{project}', '{board}', '{column}', 'TSK-1', 'Task', 'a0', '{user}', now(), now()); \
             INSERT INTO acta.tasks (id, workspace_id, project_id, board_id, column_id, parent_task_id, readable_id, title, position_key, created_by_user_id, created_at, updated_at) \
             VALUES ('{subtask}', '{ws}', '{project}', '{board}', '{column}', '{task}', 'TSK-2', 'Subtask', 'a1', '{user}', now(), now()); \
             INSERT INTO acta.task_checklist_items (id, task_id, workspace_id, title, position_key, created_by_user_id) \
             VALUES ('{checklist_item}', '{task}', '{ws}', 'Item', 'a0', '{user}'); \
             INSERT INTO acta.comments (id, workspace_id, task_id, body, created_by_user_id, created_at, updated_at) \
             VALUES ('{task_comment}', '{ws}', '{task}', 'On the task', '{user}', now(), now()); \
             INSERT INTO acta.comments (id, workspace_id, document_id, body, created_by_user_id, created_at, updated_at) \
             VALUES ('{document_comment}', '{ws}', '{document}', 'On the doc', '{user}', now(), now()); \
             INSERT INTO acta.attachments (id, workspace_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
             VALUES ('{comment_attachment}', '{ws}', '{task_comment}', 'c.txt', 'text/plain', 1, 'c-digest', '{user}', now(), now()); \
             INSERT INTO acta.attachments (id, workspace_id, document_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
             VALUES ('{document_attachment}', '{ws}', '{document}', 'd.txt', 'text/plain', 1, 'd-digest', '{user}', now(), now())"
        ),
    )
    .await;

    seed_workspace_children(db, &tree).await;

    tree
}

/// The eight workspace-level kinds, each under the tree's workspace.
async fn seed_workspace_children(db: &TestDb, tree: &Tree) {
    let Tree {
        user,
        workspace: ws,
        saved_search,
        task_view,
        tag,
        property_definition,
        status_template,
        webhook,
        automation_rule,
        integration_config,
        ..
    } = *tree;

    let key = PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create_for_user(
        atlas_core::principal::UserId(user),
        NewApiKey {
            name: format!("integration-{ws}"),
            token_hash: format!("hash-{ws}"),
            type_: ApiKeyType::Integration,
            expires_at: None,
            scopes: Vec::new(),
        },
    )
    .await
    .expect("seed integration key")
    .id
    .0;

    exec(
        db,
        format!(
            "INSERT INTO acta.saved_searches (id, workspace_id, name, query, owner_user_id) \
             VALUES ('{saved_search}', '{ws}', 'Search', 'q', '{user}'); \
             INSERT INTO acta.task_views (id, workspace_id, name, owner_user_id) \
             VALUES ('{task_view}', '{ws}', 'View', '{user}'); \
             INSERT INTO acta.tags (id, workspace_id, name, created_by_user_id) \
             VALUES ('{tag}', '{ws}', 'tag', '{user}'); \
             INSERT INTO acta.property_definitions (id, workspace_id, key, name, kind, created_by_user_id, created_at, updated_at) \
             VALUES ('{property_definition}', '{ws}', 'size', 'Size', 'text', '{user}', now(), now()); \
             INSERT INTO acta.workspace_status_templates (id, workspace_id, name, position_key) \
             VALUES ('{status_template}', '{ws}', 'Doing', 'a0'); \
             INSERT INTO acta.webhook_subscriptions (id, workspace_id, target_url, event_types, scope_type, \
                 encrypted_secret, secret_nonce, created_by_user_id) \
             VALUES ('{webhook}', '{ws}', 'https://example.invalid/hook', ARRAY['task.created'], 'workspace', \
                 '\\x00', '\\x00', '{user}'); \
             INSERT INTO acta.automation_rules (id, workspace_id, name, is_active, trigger_event_type, action_type, \
                 action_params, created_by_user_id, created_at, updated_at) \
             VALUES ('{automation_rule}', '{ws}', 'Rule', true, 'external.github.push', 'create_task', \
                 '{{}}'::jsonb, '{user}', now(), now()); \
             INSERT INTO acta.integration_configs (id, workspace_id, integration, encrypted_secret, secret_nonce, \
                 integration_api_key_id, created_by_user_id) \
             VALUES ('{integration_config}', '{ws}', 'github', '\\x00', '\\x00', '{key}', '{user}')"
        ),
    )
    .await;
}

fn provider(db: &TestDb) -> ActaResourceProvider<PgActaResourceStore> {
    ActaResourceProvider::new(
        PgActaResourceStore {
            conn: db.conn().clone(),
        },
        &Authorization {
            resource_kinds: vec!["document".to_string()],
            actions: Vec::new(),
            role_definitions: Vec::new(),
            role_definitions_v2: Vec::new(),
            principal_sets: vec![MEMBERS_SET.to_string()],
            provider: true,
        },
    )
}

fn acta(kind: &str, id: impl std::fmt::Display) -> ResourceRef {
    format!("acta::{kind}::{id}").parse().unwrap()
}

/// One `kind::id` path segment.
type Segment = (&'static str, Uuid);

fn path(segments: &[Segment]) -> ResourcePath {
    let joined: Vec<String> = segments
        .iter()
        .map(|(kind, id)| format!("{kind}::{id}"))
        .collect();

    format!("acta::{}", joined.join("/")).parse().unwrap()
}

/// Every seeded resource with its expected spec §2 path.
fn expected_paths(tree: &Tree) -> Vec<(ResourceRef, ResourcePath)> {
    let ws = ("workspace", tree.workspace);
    let project = ("project", tree.project);
    let folders = [ws, project, ("folder", tree.outer), ("folder", tree.inner)];
    let document = [folders.as_slice(), &[("document", tree.document)]].concat();
    let board = [ws, project, ("board", tree.board)];
    let task = [board.as_slice(), &[("task", tree.task)]].concat();
    let task_comment = [task.as_slice(), &[("comment", tree.task_comment)]].concat();

    let mut cases: Vec<(&str, Uuid, Vec<Segment>)> = vec![
        ("workspace", tree.workspace, vec![ws]),
        ("project", tree.project, vec![ws, project]),
        ("folder", tree.inner, folders.to_vec()),
        ("document", tree.document, document.clone()),
        ("board", tree.board, board.to_vec()),
        (
            "column",
            tree.column,
            [board.as_slice(), &[("column", tree.column)]].concat(),
        ),
        ("task", tree.task, task.clone()),
        (
            "task",
            tree.subtask,
            [board.as_slice(), &[("task", tree.subtask)]].concat(),
        ),
        (
            "checklist_item",
            tree.checklist_item,
            [task.as_slice(), &[("checklist_item", tree.checklist_item)]].concat(),
        ),
        ("comment", tree.task_comment, task_comment.clone()),
        (
            "comment",
            tree.document_comment,
            [document.as_slice(), &[("comment", tree.document_comment)]].concat(),
        ),
        (
            "attachment",
            tree.comment_attachment,
            [
                task_comment.as_slice(),
                &[("attachment", tree.comment_attachment)],
            ]
            .concat(),
        ),
        (
            "attachment",
            tree.document_attachment,
            [
                document.as_slice(),
                &[("attachment", tree.document_attachment)],
            ]
            .concat(),
        ),
    ];

    for (kind, id) in [
        ("saved_search", tree.saved_search),
        ("task_view", tree.task_view),
        ("tag", tree.tag),
        ("property_definition", tree.property_definition),
        ("status_template", tree.status_template),
        ("webhook", tree.webhook),
        ("automation_rule", tree.automation_rule),
        ("integration_config", tree.integration_config),
    ] {
        cases.push((kind, id, vec![ws, (kind, id)]));
    }

    cases
        .into_iter()
        .map(|(kind, id, segments)| (acta(kind, id), path(&segments)))
        .collect()
}

#[tokio::test]
async fn every_acta_kind_exists_with_its_spec_path_in_one_mixed_batch() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed_tree(&db, "provider-every-kind").await;
    let expected = expected_paths(&tree);
    let ghost = acta("document", Uuid::now_v7());
    let resources: Vec<ResourceRef> = expected
        .iter()
        .map(|(resource, _)| resource.clone())
        .chain([ghost.clone()])
        .collect();

    let facts = provider(&db).resource_facts(&resources).await.unwrap();

    assert_eq!(facts.len(), resources.len());
    for ((resource, path), fact) in expected.iter().zip(&facts) {
        assert_eq!(&fact.resource, resource);
        assert_eq!(fact.existence, ResourceExistence::Exists, "{resource}");
        assert_eq!(fact.path.as_ref(), Some(path), "{resource}");
    }
    let last = facts.last().unwrap();
    assert_eq!(last.resource, ghost);
    assert_eq!(last.existence, ResourceExistence::Missing);

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn a_trashed_ancestor_hides_its_whole_subtree() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed_tree(&db, "provider-trashed").await;
    exec(
        &db,
        format!(
            "UPDATE acta.folders SET deleted_at = now() WHERE id = '{}'",
            tree.outer
        ),
    )
    .await;
    let hidden = [
        acta("folder", tree.inner),
        acta("document", tree.document),
        acta("comment", tree.document_comment),
        acta("attachment", tree.document_attachment),
    ];
    let still_live = [acta("project", tree.project), acta("task", tree.task)];

    let facts = provider(&db)
        .resource_facts(&[hidden.as_slice(), still_live.as_slice()].concat())
        .await
        .unwrap();

    for fact in &facts[..hidden.len()] {
        assert_eq!(
            fact.existence,
            ResourceExistence::Missing,
            "{}",
            fact.resource
        );
        assert!(fact.path.is_none());
    }
    for fact in &facts[hidden.len()..] {
        assert_eq!(
            fact.existence,
            ResourceExistence::Exists,
            "{}",
            fact.resource
        );
    }

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn a_trashed_workspace_hides_its_workspace_level_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed_tree(&db, "provider-trashed-ws").await;
    exec(
        &db,
        format!(
            "UPDATE acta.workspaces SET deleted_at = now() WHERE id = '{}'",
            tree.workspace
        ),
    )
    .await;

    let facts = provider(&db)
        .resource_facts(&[acta("tag", tree.tag), acta("task", tree.task)])
        .await
        .unwrap();

    assert!(
        facts
            .iter()
            .all(|fact| fact.existence == ResourceExistence::Missing)
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn share_links_and_alias_ids_are_missing() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed_tree(&db, "provider-aliases").await;
    let resources = [
        acta("share_link", Uuid::now_v7()),
        acta(
            "document",
            tree.document.hyphenated().to_string().to_uppercase(),
        ),
        acta("document", tree.document.simple()),
        acta("document", tree.document.braced()),
        acta("document", tree.document),
    ];

    let facts = provider(&db).resource_facts(&resources).await.unwrap();

    assert_eq!(
        facts.iter().map(|fact| fact.existence).collect::<Vec<_>>(),
        vec![
            ResourceExistence::Missing,
            ResourceExistence::Missing,
            ResourceExistence::Missing,
            ResourceExistence::Missing,
            ResourceExistence::Exists,
        ]
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn the_members_set_lists_every_membership_role_and_nothing_for_an_unknown_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed_tree(&db, "provider-members").await;
    let mut expected = Vec::new();
    for role in ["owner", "admin", "member"] {
        let user = seed_user(&db, &format!("provider-member-{role}")).await;
        exec(
            &db,
            format!(
                "INSERT INTO acta.workspace_memberships (id, workspace_id, user_id, role, created_at, updated_at) \
                 VALUES ('{}', '{}', '{user}', '{role}', now(), now())",
                Uuid::now_v7(),
                tree.workspace
            ),
        )
        .await;
        expected.push(user.hyphenated().to_string());
    }
    expected.sort();
    let provider = provider(&db);

    let mut members: Vec<String> = provider
        .members_of(
            &format!("acta::workspace::{}::{MEMBERS_SET}", tree.workspace)
                .parse()
                .unwrap(),
        )
        .await
        .unwrap()
        .iter()
        .map(|member| member.as_str().to_string())
        .collect();
    members.sort();

    assert_eq!(members, expected);
    assert!(matches!(
        provider
            .members_of(
                &format!("acta::workspace::{}::{MEMBERS_SET}", Uuid::now_v7())
                    .parse()
                    .unwrap()
            )
            .await,
        Err(CapabilityError::NotFound { .. })
    ));

    db.teardown().await.expect("teardown");
}

/// The path-parent fallbacks the full tree does not reach: a board in a
/// folder, a folder and a document straight under the workspace, a document
/// straight under a project, and an attachment on a task. A draft-only
/// attachment has no path parent and is missing.
#[tokio::test]
async fn every_path_parent_fallback_resolves_and_a_draft_only_attachment_is_missing() {
    let db = TestDb::create().await.expect("TestDb::create");
    let tree = seed_tree(&db, "provider-fallbacks").await;
    let Tree {
        user,
        workspace: ws,
        project,
        outer,
        task,
        ..
    } = tree;
    let foldered_board = Uuid::now_v7();
    let root_folder = Uuid::now_v7();
    let root_document = Uuid::now_v7();
    let project_document = Uuid::now_v7();
    let task_attachment = Uuid::now_v7();
    let draft = Uuid::now_v7();
    let draft_attachment = Uuid::now_v7();

    exec(
        &db,
        format!(
            "INSERT INTO acta.boards (id, workspace_id, project_id, folder_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{foldered_board}', '{ws}', '{project}', '{outer}', 'Foldered', '{user}', now(), now()); \
             INSERT INTO acta.folders (id, workspace_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{root_folder}', '{ws}', 'Root', '{user}', now(), now()); \
             INSERT INTO acta.documents (id, workspace_id, title, created_by_user_id) \
             VALUES ('{root_document}', '{ws}', 'Root doc', '{user}'); \
             INSERT INTO acta.documents (id, workspace_id, project_id, title, created_by_user_id) \
             VALUES ('{project_document}', '{ws}', '{project}', 'Project doc', '{user}'); \
             INSERT INTO acta.attachments (id, workspace_id, task_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
             VALUES ('{task_attachment}', '{ws}', '{task}', 't.txt', 'text/plain', 1, 't-digest', '{user}', now(), now()); \
             INSERT INTO acta.comment_attachment_drafts (id, workspace_id, task_id, created_by_user_id, create_token, \
                 create_digest, state, expires_at) \
             VALUES ('{draft}', '{ws}', '{task}', '{user}', 'draft-token', \
                 decode(repeat('00', 32), 'hex'), 'active', now() + interval '1 hour'); \
             INSERT INTO acta.attachments (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
             VALUES ('{draft_attachment}', '{ws}', '{draft}', 'd.txt', 'text/plain', 1, 'draft-digest', '{user}', now(), now())"
        ),
    )
    .await;

    let workspace = ("workspace", ws);
    let task_path = [
        workspace,
        ("project", project),
        ("board", tree.board),
        ("task", task),
    ];
    let expected = [
        (
            acta("board", foldered_board),
            path(&[
                workspace,
                ("project", project),
                ("folder", outer),
                ("board", foldered_board),
            ]),
        ),
        (
            acta("folder", root_folder),
            path(&[workspace, ("folder", root_folder)]),
        ),
        (
            acta("document", root_document),
            path(&[workspace, ("document", root_document)]),
        ),
        (
            acta("document", project_document),
            path(&[
                workspace,
                ("project", project),
                ("document", project_document),
            ]),
        ),
        (
            acta("attachment", task_attachment),
            path(&[task_path.as_slice(), &[("attachment", task_attachment)]].concat()),
        ),
    ];
    let resources: Vec<ResourceRef> = expected
        .iter()
        .map(|(resource, _)| resource.clone())
        .chain([acta("attachment", draft_attachment)])
        .collect();

    let facts = provider(&db).resource_facts(&resources).await.unwrap();

    assert_eq!(facts.len(), resources.len());
    for ((resource, path), fact) in expected.iter().zip(&facts) {
        assert_eq!(&fact.resource, resource);
        assert_eq!(fact.existence, ResourceExistence::Exists, "{resource}");
        assert_eq!(fact.path.as_ref(), Some(path), "{resource}");
    }
    let draft_fact = facts.last().unwrap();
    assert_eq!(draft_fact.resource, acta("attachment", draft_attachment));
    assert_eq!(draft_fact.existence, ResourceExistence::Missing);
    assert!(draft_fact.path.is_none());

    db.teardown().await.expect("teardown");
}
