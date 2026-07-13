use atlas_domain::{
    entities::identity::MemberRole,
    ids::{ApiKeyId, UserId, WorkspaceId},
    permissions::{
        Capability, CapabilityAction, CapabilityFamily, Principal, ResourceRef, ResourceRole,
    },
};
use std::sync::{Arc, Mutex};

use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use sea_orm_migration::prelude::MigratorTrait;
use uuid::Uuid;

use super::batch_authorization::{
    BatchAuthorizationService, BatchAuthorizationSource, PgBatchAuthorizationSource,
    PrincipalFacts, ProjectionAuthContext, ProjectionSubject, SubjectFamily,
};

#[tokio::test]
async fn query_a_resolves_live_document_task_attachment_and_comment_subject_chains() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let folder_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();
    let board_id = Uuid::now_v7();
    let column_id = Uuid::now_v7();
    let task_id = Uuid::now_v7();
    let document_attachment_id = Uuid::now_v7();
    let task_comment_id = Uuid::now_v7();
    let comment_attachment_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;
    seed_projection_subjects(
        &db.conn,
        ProjectionSubjects {
            workspace_id,
            user_id,
            project_id,
            folder_id,
            document_id,
            board_id,
            column_id,
            task_id,
            document_attachment_id,
            task_comment_id,
            comment_attachment_id,
        },
    )
    .await;

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let facts = source
        .load_subject_facts(
            &user_context(workspace_id, user_id),
            &[
                ProjectionSubject::Document(document_id),
                ProjectionSubject::Task(task_id),
                ProjectionSubject::Attachment(document_attachment_id),
                ProjectionSubject::SourceComment(task_comment_id),
                ProjectionSubject::Attachment(comment_attachment_id),
            ],
        )
        .await
        .expect("load subject facts");

    assert_eq!(facts.len(), 5);
    let [
        document,
        task,
        document_attachment,
        comment,
        comment_attachment,
    ] = facts.as_slice()
    else {
        panic!("expected one fact for each requested live subject");
    };
    assert_eq!(document.ordinal, 0);
    assert_eq!(document.family, SubjectFamily::Documents);
    assert_chain(
        document,
        &[
            ResourceRef::Document(atlas_domain::DocumentId(document_id)),
            ResourceRef::Folder(atlas_domain::FolderId(folder_id)),
            ResourceRef::Project(atlas_domain::ProjectId(project_id)),
            ResourceRef::Workspace,
        ],
    );
    assert_eq!(task.ordinal, 1);
    assert_eq!(task.family, SubjectFamily::Tasks);
    assert_chain(
        task,
        &[
            ResourceRef::Board(atlas_domain::BoardId(board_id)),
            ResourceRef::Project(atlas_domain::ProjectId(project_id)),
            ResourceRef::Workspace,
        ],
    );
    assert_eq!(document_attachment.family, SubjectFamily::Documents);
    assert_chain(
        document_attachment,
        &document
            .chain
            .segments
            .iter()
            .map(|segment| segment.resource.clone())
            .collect::<Vec<_>>(),
    );
    assert_eq!(comment.family, SubjectFamily::Tasks);
    assert_chain(
        comment,
        &task
            .chain
            .segments
            .iter()
            .map(|segment| segment.resource.clone())
            .collect::<Vec<_>>(),
    );
    assert_eq!(comment_attachment.family, SubjectFamily::Tasks);
    assert_chain(
        comment_attachment,
        &task
            .chain
            .segments
            .iter()
            .map(|segment| segment.resource.clone())
            .collect::<Vec<_>>(),
    );

    db.teardown().await;
}

#[tokio::test]
async fn batch_authorization_executes_exactly_two_marked_statements_for_nonempty_batches() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;
    seed_minimal_document(&db.conn, workspace_id, user_id, document_id).await;

    let statements = Arc::new(Mutex::new(Vec::new()));
    let mut measured = db.conn.clone();
    let measured_statements = Arc::clone(&statements);
    measured.set_metric_callback(move |info| {
        if info.statement.sql.contains("atlas_batch_") {
            measured_statements
                .lock()
                .expect("statement metric lock")
                .push(info.statement.sql.clone());
        }
    });

    let service = BatchAuthorizationService::new(PgBatchAuthorizationSource::new(measured));
    let context = user_context(workspace_id, user_id);

    let decisions = service
        .authorize(
            &context,
            &[
                ProjectionSubject::Document(document_id),
                ProjectionSubject::Attachment(Uuid::now_v7()),
            ],
        )
        .await
        .expect("authorize mixed batch");
    assert_eq!(decisions, vec![false, false]);
    assert_marked_statement_pair(&statements);

    statements.lock().expect("statement metric lock").clear();
    let decisions = service
        .authorize(&context, &[ProjectionSubject::Document(Uuid::now_v7())])
        .await
        .expect("authorize all-missing batch");
    assert_eq!(decisions, vec![false]);
    assert_marked_statement_pair(&statements);

    statements.lock().expect("statement metric lock").clear();
    assert!(
        service
            .authorize(&context, &[])
            .await
            .expect("authorize empty batch")
            .is_empty()
    );
    assert!(statements.lock().expect("statement metric lock").is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn query_a_omits_dead_projects_and_unavailable_subjects() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let ids = ProjectionSubjects {
        workspace_id,
        user_id,
        project_id: Uuid::now_v7(),
        folder_id: Uuid::now_v7(),
        document_id: Uuid::now_v7(),
        board_id: Uuid::now_v7(),
        column_id: Uuid::now_v7(),
        task_id: Uuid::now_v7(),
        document_attachment_id: Uuid::now_v7(),
        task_comment_id: Uuid::now_v7(),
        comment_attachment_id: Uuid::now_v7(),
    };

    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;
    seed_projection_subjects(&db.conn, ids).await;

    db.conn
        .execute_unprepared(&format!(
            "UPDATE projects SET deleted_at = now() WHERE id = '{}'",
            ids.project_id
        ))
        .await
        .expect("soft-delete project");

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let context = user_context(workspace_id, user_id);
    let other_workspace_id = Uuid::now_v7();
    let other_user_id = Uuid::now_v7();
    let other_document_id = Uuid::now_v7();
    seed_workspace_user(&db.conn, other_workspace_id, other_user_id, true).await;
    seed_minimal_document(
        &db.conn,
        other_workspace_id,
        other_user_id,
        other_document_id,
    )
    .await;
    let facts = source
        .load_subject_facts(
            &context,
            &[
                ProjectionSubject::Document(ids.document_id),
                ProjectionSubject::Task(ids.task_id),
            ],
        )
        .await
        .expect("load chains without dead project");
    assert_eq!(facts.len(), 2);
    assert!(facts.iter().all(|fact| {
        fact.chain
            .segments
            .iter()
            .all(|segment| !matches!(segment.resource, ResourceRef::Project(_)))
    }));

    db.conn
        .execute_unprepared(&format!(
            "UPDATE documents SET deleted_at = now() WHERE id = '{}'; \
             UPDATE tasks SET deleted_at = now() WHERE id = '{}'",
            ids.document_id, ids.task_id
        ))
        .await
        .expect("soft-delete parents");

    let facts = source
        .load_subject_facts(
            &context,
            &[
                ProjectionSubject::Document(ids.document_id),
                ProjectionSubject::Task(ids.task_id),
                ProjectionSubject::Attachment(ids.document_attachment_id),
                ProjectionSubject::SourceComment(ids.task_comment_id),
                ProjectionSubject::Attachment(ids.comment_attachment_id),
                ProjectionSubject::Document(Uuid::now_v7()),
                ProjectionSubject::Document(other_document_id),
            ],
        )
        .await
        .expect("omit unavailable subjects");
    assert!(facts.is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn query_b_reloads_user_membership_and_active_state() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let context = user_context(workspace_id, user_id);

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("load active member facts");
    assert_user_facts(&facts, true, Some(MemberRole::Member));

    db.conn
        .execute_unprepared(&format!(
            "DELETE FROM workspace_memberships WHERE workspace_id = '{workspace_id}' AND user_id = '{user_id}'"
        ))
        .await
        .expect("remove membership");

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload removed membership");
    assert_user_facts(&facts, true, None);

    db.conn
        .execute_unprepared(&format!(
            "UPDATE users SET disabled_at = now() WHERE id = '{user_id}'"
        ))
        .await
        .expect("disable user");

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload disabled user");
    assert_user_facts(&facts, false, None);

    db.teardown().await;
}

#[tokio::test]
async fn query_b_reloads_key_creator_and_live_group_grant_facts() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let creator_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();
    let group_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, creator_id, true).await;
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, scopes) \
             VALUES ('{key_id}', NULL, '{creator_id}', 'key', 'hash', 'agent', now(), false, ARRAY['docs:read'])"
        ))
        .await
        .expect("insert key");
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO groups (id, workspace_id, name, created_by, created_at, updated_at) \
             VALUES ('{group_id}', '{workspace_id}', 'group', '{creator_id}', now(), now()); \
             INSERT INTO group_members (group_id, user_id, created_at) VALUES ('{group_id}', '{creator_id}', now()); \
             INSERT INTO permission_grants (id, workspace_id, api_key_id, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{key_id}', 'viewer', now(), now()); \
             INSERT INTO permission_grants (id, workspace_id, group_id, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{group_id}', 'editor', now(), now())",
            Uuid::now_v7(),
            Uuid::now_v7(),
        ))
        .await
        .expect("insert grants");

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let context = ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::ApiKey(ApiKeyId(key_id)),
    );

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("load key facts");
    assert_key_facts(&facts, key_id, creator_id, false, true, true);

    db.conn
        .execute_unprepared(&format!(
            "UPDATE api_keys SET revoked_at = now(), expires_at = now() - interval '1 second', is_global = true, scopes = ARRAY['tasks:read'] WHERE id = '{key_id}'; \
             UPDATE groups SET deleted_at = now() WHERE id = '{group_id}'; \
             UPDATE users SET disabled_at = now() WHERE id = '{creator_id}'"
        ))
        .await
        .expect("mutate current authority facts");

    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload changed key facts");
    let PrincipalFacts::ApiKey(facts) = facts else {
        panic!("expected API-key facts");
    };
    assert!(facts.is_revoked);
    assert!(facts.is_expired);
    assert!(facts.is_global);
    assert_eq!(
        facts.scopes,
        vec![Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        }]
    );
    assert!(!facts.creator.is_active);
    assert_eq!(
        facts.grants,
        vec![(ResourceRef::Workspace, ResourceRole::Viewer)]
    );
    assert!(facts.creator.grants.is_empty());

    db.conn
        .execute_unprepared(&format!(
            "DELETE FROM permission_grants WHERE api_key_id = '{key_id}'"
        ))
        .await
        .expect("remove direct key grant");
    let facts = source
        .load_principal_facts(&context, &[])
        .await
        .expect("reload removed direct key grant");
    let PrincipalFacts::ApiKey(facts) = facts else {
        panic!("expected API-key facts");
    };
    assert!(facts.grants.is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn query_b_rejects_unknown_scopes_and_propagates_sql_failures() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let creator_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, creator_id, true).await;
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, scopes) \
             VALUES ('{key_id}', NULL, '{creator_id}', 'key', 'hash', 'agent', now(), false, ARRAY['unknown:read'])"
        ))
        .await
        .expect("insert key with unknown scope");

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let context = ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::ApiKey(ApiKeyId(key_id)),
    );
    assert!(source.load_principal_facts(&context, &[]).await.is_err());

    db.conn
        .execute_unprepared("DROP TABLE permission_grants")
        .await
        .expect("remove query dependency");
    assert!(source.load_principal_facts(&context, &[]).await.is_err());

    db.teardown().await;
}

#[derive(Clone, Copy)]
struct ProjectionSubjects {
    workspace_id: Uuid,
    user_id: Uuid,
    project_id: Uuid,
    folder_id: Uuid,
    document_id: Uuid,
    board_id: Uuid,
    column_id: Uuid,
    task_id: Uuid,
    document_attachment_id: Uuid,
    task_comment_id: Uuid,
    comment_attachment_id: Uuid,
}

async fn seed_projection_subjects(conn: &DatabaseConnection, ids: ProjectionSubjects) {
    conn.execute_unprepared(&format!(
        "INSERT INTO projects (id, workspace_id, name, slug, task_prefix, next_task_number, visibility, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', 'Project', 'project-{}', 'AT', 1, 'workspace', '{}', now(), now()); \
         INSERT INTO folders (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', 'Folder', '{}', now(), now()); \
         INSERT INTO documents (id, workspace_id, project_id, folder_id, title, slug, content, frontmatter, current_revision_seq, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', '{}', 'Document', 'document-{}', '', '{{}}', 1, '{}', now(), now()); \
         INSERT INTO boards (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', 'Board', '{}', now(), now()); \
         INSERT INTO board_columns (id, workspace_id, board_id, name, position_key, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', 'Todo', 'a0', '{}', now(), now()); \
         INSERT INTO tasks (id, workspace_id, project_id, board_id, column_id, readable_id, title, description, labels, position_key, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', '{}', '{}', 'AT-1', 'Task', '', ARRAY[]::text[], 'a0', '{}', now(), now()); \
         INSERT INTO attachments (id, workspace_id, document_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', 'document.txt', 'text/plain', 1, 'document-digest', '{}', now(), now()); \
         INSERT INTO comments (id, workspace_id, task_id, body, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', 'comment', '{}', now(), now()); \
         INSERT INTO attachments (id, workspace_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
         VALUES ('{}', '{}', '{}', 'comment.txt', 'text/plain', 1, 'comment-digest', '{}', now(), now())",
        ids.project_id,
        ids.workspace_id,
        ids.project_id,
        ids.user_id,
        ids.folder_id,
        ids.workspace_id,
        ids.project_id,
        ids.user_id,
        ids.document_id,
        ids.workspace_id,
        ids.project_id,
        ids.folder_id,
        ids.document_id,
        ids.user_id,
        ids.board_id,
        ids.workspace_id,
        ids.project_id,
        ids.user_id,
        ids.column_id,
        ids.workspace_id,
        ids.board_id,
        ids.user_id,
        ids.task_id,
        ids.workspace_id,
        ids.project_id,
        ids.board_id,
        ids.column_id,
        ids.user_id,
        ids.document_attachment_id,
        ids.workspace_id,
        ids.document_id,
        ids.user_id,
        ids.task_comment_id,
        ids.workspace_id,
        ids.task_id,
        ids.user_id,
        ids.comment_attachment_id,
        ids.workspace_id,
        ids.task_comment_id,
        ids.user_id,
    ))
    .await
    .expect("seed projection subjects");
}

async fn seed_minimal_document(
    conn: &DatabaseConnection,
    workspace_id: Uuid,
    user_id: Uuid,
    document_id: Uuid,
) {
    conn.execute_unprepared(&format!(
        "INSERT INTO documents (id, workspace_id, title, slug, content, frontmatter, current_revision_seq, created_by_user_id, created_at, updated_at) \
         VALUES ('{document_id}', '{workspace_id}', 'Document', 'document-{document_id}', '', '{{}}', 1, '{user_id}', now(), now())"
    ))
    .await
    .expect("seed minimal document");
}

fn assert_marked_statement_pair(statements: &Arc<Mutex<Vec<String>>>) {
    let statements = statements.lock().expect("statement metric lock");
    assert_eq!(statements.len(), 2);
    let [subject_statement, principal_statement] = statements.as_slice() else {
        panic!("expected exactly the marked subject and principal statements");
    };
    assert!(subject_statement.contains("atlas_batch_subject_facts"));
    assert!(principal_statement.contains("atlas_batch_principal_facts"));
}

fn assert_chain(fact: &super::batch_authorization::SubjectFact, expected: &[ResourceRef]) {
    assert_eq!(
        fact.chain
            .segments
            .iter()
            .map(|segment| segment.resource.clone())
            .collect::<Vec<_>>(),
        expected
    );
}

fn user_context(workspace_id: Uuid, user_id: Uuid) -> ProjectionAuthContext {
    ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::User(UserId(user_id)),
    )
}

fn assert_user_facts(facts: &PrincipalFacts, is_active: bool, membership: Option<MemberRole>) {
    let PrincipalFacts::User(facts) = facts else {
        panic!("expected user facts");
    };
    assert_eq!(facts.is_active, is_active);
    assert_eq!(facts.effective_membership, membership);
}

fn assert_key_facts(
    facts: &PrincipalFacts,
    key_id: Uuid,
    creator_id: Uuid,
    is_global: bool,
    creator_active: bool,
    creator_group_grant_present: bool,
) {
    let PrincipalFacts::ApiKey(facts) = facts else {
        panic!("expected API-key facts");
    };
    assert_eq!(facts.key_id, ApiKeyId(key_id));
    assert_eq!(facts.creator.user_id, UserId(creator_id));
    assert_eq!(facts.is_global, is_global);
    assert_eq!(facts.creator.is_active, creator_active);
    assert_eq!(
        facts
            .creator
            .grants
            .contains(&(ResourceRef::Workspace, ResourceRole::Editor)),
        creator_group_grant_present,
    );
}

async fn seed_workspace_user(
    conn: &DatabaseConnection,
    workspace_id: Uuid,
    user_id: Uuid,
    member: bool,
) {
    conn.execute_unprepared(&format!(
        "INSERT INTO users (id, username, display_name, is_root, is_system_admin, created_at, updated_at) \
         VALUES ('{user_id}', 'user-{user_id}', 'User', false, false, now(), now()); \
         INSERT INTO workspaces (id, name, slug, created_at, updated_at) \
         VALUES ('{workspace_id}', 'Workspace', 'workspace-{workspace_id}', now(), now())"
    ))
    .await
    .expect("seed user and workspace");

    if member {
        conn.execute_unprepared(&format!(
            "INSERT INTO workspace_memberships (id, workspace_id, user_id, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{user_id}', 'member', now(), now())",
            Uuid::now_v7(),
        ))
        .await
        .expect("seed membership");
    }
}

struct BatchAuthorizationDb {
    conn: DatabaseConnection,
    name: String,
    admin_url: String,
}

impl BatchAuthorizationDb {
    async fn create() -> Self {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://atlas:atlas@localhost:5432/atlas_dev".to_string());
        let admin_url = replace_database_name(&database_url, "postgres");
        let name = format!("atlas_batch_authz_{}", Uuid::now_v7().as_simple());
        let admin = Database::connect(&admin_url)
            .await
            .expect("connect admin database");
        admin
            .execute_unprepared(&format!("CREATE DATABASE \"{name}\""))
            .await
            .expect("create test database");
        drop(admin);

        let conn = Database::connect(replace_database_name(&database_url, &name))
            .await
            .expect("connect test database");
        migration::Migrator::up(&conn, None)
            .await
            .expect("migrate test database");

        Self {
            conn,
            name,
            admin_url,
        }
    }

    async fn teardown(self) {
        drop(self.conn);
        let admin = Database::connect(&self.admin_url)
            .await
            .expect("connect admin database");
        admin
            .execute_unprepared(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.name
            ))
            .await
            .expect("drop test database");
    }
}

fn replace_database_name(url: &str, name: &str) -> String {
    let Some(index) = url.rfind('/') else {
        panic!("database URL has no database name");
    };
    let (prefix, suffix) = url.split_at(index + 1);
    let query_start = suffix.find('?').unwrap_or(suffix.len());
    format!("{prefix}{name}{}", &suffix[query_start..])
}
