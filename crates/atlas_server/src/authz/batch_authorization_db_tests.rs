use crate::authz::ResourceRole;
use crate::persistence::repos::{PermissionGrantRepo, PgPermissionGrantRepo};
use atlas_acta::entities::identity::MemberRole;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::permissions::ResourceRef;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::Principal;
use atlas_core::principal::UserId;
use atlas_custos::capability::Capability;
use atlas_custos::capability::CapabilityAction;
use atlas_custos::capability::CapabilityFamily;
use std::sync::{Arc, Mutex};

use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
use sea_orm_migration::prelude::MigratorTrait;
use uuid::Uuid;

use super::batch_authorization::{
    BatchAuthorizationService, BatchAuthorizationSource, PgBatchAuthorizationSource,
    PrincipalFacts, ProjectionAuthContext, ProjectionSubject, SubjectFamily,
};
use super::policy::ResolutionQuery;

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
            ResourceRef::Document(atlas_acta::ids::DocumentId(document_id)),
            ResourceRef::Folder(atlas_acta::ids::FolderId(folder_id)),
            ResourceRef::Project(atlas_acta::ids::ProjectId(project_id)),
            ResourceRef::Workspace,
        ],
    );
    assert_eq!(task.ordinal, 1);
    assert_eq!(task.family, SubjectFamily::Tasks);
    assert_chain(
        task,
        &[
            ResourceRef::Board(atlas_acta::ids::BoardId(board_id)),
            ResourceRef::Project(atlas_acta::ids::ProjectId(project_id)),
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
            "DELETE FROM acta.workspace_memberships WHERE workspace_id = '{workspace_id}' AND user_id = '{user_id}'"
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
            "UPDATE custos.users SET disabled_at = now() WHERE id = '{user_id}'"
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
            "INSERT INTO custos.api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, scopes) \
             VALUES ('{key_id}', NULL, '{creator_id}', 'key', 'hash', 'agent', now(), false, ARRAY['docs:read'])"
        ))
        .await
        .expect("insert key");
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO custos.groups (id, workspace_id, name, created_by, created_at, updated_at) \
             VALUES ('{group_id}', '{workspace_id}', 'group', '{creator_id}', now(), now()); \
             INSERT INTO custos.group_members (group_id, user_id, created_at) VALUES ('{group_id}', '{creator_id}', now()); \
             INSERT INTO custos.permission_grants (id, workspace_id, api_key_id, resource_ref, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{key_id}', 'acta::workspace::{workspace_id}', 'viewer', now(), now()); \
             INSERT INTO custos.permission_grants (id, workspace_id, group_id, resource_ref, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{group_id}', 'acta::workspace::{workspace_id}', 'editor', now(), now())",
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
            "UPDATE custos.api_keys SET revoked_at = now(), expires_at = now() - interval '1 second', is_global = true, scopes = ARRAY['tasks:read'] WHERE id = '{key_id}'; \
             UPDATE custos.groups SET deleted_at = now() WHERE id = '{group_id}'; \
             UPDATE custos.users SET disabled_at = now() WHERE id = '{creator_id}'"
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
            "DELETE FROM custos.permission_grants WHERE api_key_id = '{key_id}'"
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
            "INSERT INTO custos.api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, scopes) \
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
        .execute_unprepared("DROP TABLE custos.permission_grants")
        .await
        .expect("remove query dependency");
    assert!(source.load_principal_facts(&context, &[]).await.is_err());

    db.teardown().await;
}

// PR3 characterization suite (design §S3b): the eight paths below pin current
// authorization outcomes before PR4 cuts the batch seam onto the
// `ResourceChainSource`/`PrincipalFactsSource` ports. Every test observes only
// allow/deny/error and which rows are consulted, so it must keep passing
// unmodified after the cut.

/// Pins `repos/permissions.rs:376`'s raw group-membership predicate: a grant
/// held only through a group must stop resolving the moment the group is
/// soft-deleted. This path sits outside the two batch queries, but PR4's port
/// cut must not change it either.
#[tokio::test]
async fn group_grant_resolution_excludes_a_soft_deleted_group() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let group_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO custos.groups (id, workspace_id, name, created_by, created_at, updated_at) \
             VALUES ('{group_id}', '{workspace_id}', 'group', '{user_id}', now(), now()); \
             INSERT INTO custos.group_members (group_id, user_id, created_at) VALUES ('{group_id}', '{user_id}', now()); \
             INSERT INTO custos.permission_grants (id, workspace_id, group_id, resource_ref, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{group_id}', 'acta::workspace::{workspace_id}', 'editor', now(), now())",
            Uuid::now_v7(),
        ))
        .await
        .expect("seed live group grant");

    let repo = PgPermissionGrantRepo {
        conn: db.conn.clone(),
    };

    let grants = repo
        .load_grants_for_resolution(resolution_query(workspace_id, user_id, group_id))
        .await
        .expect("live group grant resolves");
    assert_eq!(
        grants,
        vec![(
            atlas_acta::permissions::resource_ref_codec::to_core(
                &ResourceRef::Workspace,
                WorkspaceId(workspace_id)
            ),
            ResourceRole::Editor
        )]
    );

    db.conn
        .execute_unprepared(&format!(
            "UPDATE custos.groups SET deleted_at = now() WHERE id = '{group_id}'"
        ))
        .await
        .expect("soft-delete group");

    let grants = repo
        .load_grants_for_resolution(resolution_query(workspace_id, user_id, group_id))
        .await
        .expect("soft-deleted group grant is excluded, not an error");
    assert!(grants.is_empty());

    db.teardown().await;
}

/// `key_facts` reuses `user_grants` for the creator's own facts. Beyond the
/// existing single happy-path test, this pins that losing workspace
/// membership mid-chain denies an otherwise-capable global api key, because
/// `resolve_user_role` requires either root/system-admin or a live
/// membership row — creator facts are read fresh on every call, not cached.
#[tokio::test]
async fn api_key_creator_membership_loss_denies_mid_chain() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let creator_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, creator_id, true).await;
    seed_workspace_scope_grant(
        &db.conn,
        workspace_id,
        GrantPrincipal::User(creator_id),
        "editor",
    )
    .await;
    seed_api_key(&db.conn, key_id, creator_id, true, &["docs:read"]).await;
    seed_minimal_document(&db.conn, workspace_id, creator_id, document_id).await;

    let service = BatchAuthorizationService::new(PgBatchAuthorizationSource::new(db.conn.clone()));
    let context = ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::ApiKey(ApiKeyId(key_id)),
    );

    let decisions = service
        .authorize(&context, &[ProjectionSubject::Document(document_id)])
        .await
        .expect("authorize while creator is still a member");
    assert_eq!(decisions, vec![true]);

    db.conn
        .execute_unprepared(&format!(
            "DELETE FROM acta.workspace_memberships WHERE workspace_id = '{workspace_id}' AND user_id = '{creator_id}'"
        ))
        .await
        .expect("simulate mid-chain membership loss");

    let decisions = service
        .authorize(&context, &[ProjectionSubject::Document(document_id)])
        .await
        .expect("authorize after the creator lost membership");
    assert_eq!(
        decisions,
        vec![false],
        "a global key's role tracks its creator's live membership, not a cached grant"
    );

    db.teardown().await;
}

/// `resolve()` unit tests exercise the enum directly; this asserts the same
/// visibility contribution once QUERY_A has assembled a real chain. Only the
/// project segment ever carries a visibility payload — document, folder,
/// board, and workspace segments are always stamped `visibility: NULL` — so a
/// member with no explicit grant is authorized exactly where a live project
/// segment exists, and denied where the chain has none.
#[tokio::test]
async fn visibility_contribution_flows_only_through_the_project_segment() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;

    let document_project_id = Uuid::now_v7();
    seed_project(
        &db.conn,
        workspace_id,
        document_project_id,
        user_id,
        "workspace",
        "viewer",
    )
    .await;
    let document_id = Uuid::now_v7();
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO documents (id, workspace_id, project_id, title, slug, content, frontmatter, current_revision_seq, created_by_user_id, created_at, updated_at) \
             VALUES ('{document_id}', '{workspace_id}', '{document_project_id}', 'Document', 'document-{document_id}', '', '{{}}', 1, '{user_id}', now(), now())"
        ))
        .await
        .expect("seed document under a visible project");

    let task_project_id = Uuid::now_v7();
    seed_project(
        &db.conn,
        workspace_id,
        task_project_id,
        user_id,
        "public",
        "editor",
    )
    .await;
    let board_id = Uuid::now_v7();
    let column_id = Uuid::now_v7();
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO boards (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{board_id}', '{workspace_id}', '{task_project_id}', 'Board', '{user_id}', now(), now()); \
             INSERT INTO board_columns (id, workspace_id, board_id, name, position_key, created_by_user_id, created_at, updated_at) \
             VALUES ('{column_id}', '{workspace_id}', '{board_id}', 'Todo', 'a0', '{user_id}', now(), now())"
        ))
        .await
        .expect("seed board under a visible project");
    let task_id = Uuid::now_v7();
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO tasks (id, workspace_id, project_id, board_id, column_id, readable_id, title, description, labels, position_key, created_by_user_id, created_at, updated_at) \
             VALUES ('{task_id}', '{workspace_id}', '{task_project_id}', '{board_id}', '{column_id}', 'AT-1', 'Task', '', ARRAY[]::text[], 'a0', '{user_id}', now(), now())"
        ))
        .await
        .expect("seed task under the visible board");

    let orphan_document_id = Uuid::now_v7();
    seed_minimal_document(&db.conn, workspace_id, user_id, orphan_document_id).await;

    let service = BatchAuthorizationService::new(PgBatchAuthorizationSource::new(db.conn.clone()));
    let context = user_context(workspace_id, user_id);

    let decisions = service
        .authorize(
            &context,
            &[
                ProjectionSubject::Document(document_id),
                ProjectionSubject::Task(task_id),
                ProjectionSubject::Document(orphan_document_id),
            ],
        )
        .await
        .expect("authorize a mixed-visibility batch");

    assert_eq!(
        decisions,
        vec![true, true, false],
        "a member with no explicit grant is allowed only where the assembled chain \
         carries a live project visibility segment"
    );

    db.teardown().await;
}

/// `folder_rows.inherited_project_id` is
/// `(array_agg(project_id ORDER BY depth DESC))[1]`: it always reads the
/// ancestor with the *largest* depth, i.e. the root-most folder — not the
/// folder nearest the document. This pins both directions: a nearer folder's
/// real project id is ignored when the root ancestor's is NULL, and a project
/// set only on the root ancestor is correctly inherited.
#[tokio::test]
async fn inherited_project_id_prefers_the_root_most_folder_ancestor() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;

    // Root ancestor has no project; the nearer folder does — the root's NULL
    // wins, so the resulting chain carries no project segment at all.
    let root_folder_id = Uuid::now_v7();
    let leaf_folder_id = Uuid::now_v7();
    let shadowed_project_id = Uuid::now_v7();
    seed_project(
        &db.conn,
        workspace_id,
        shadowed_project_id,
        user_id,
        "workspace",
        "editor",
    )
    .await;
    seed_folder(
        &db.conn,
        workspace_id,
        root_folder_id,
        None,
        None,
        user_id,
        "root",
    )
    .await;
    seed_folder(
        &db.conn,
        workspace_id,
        leaf_folder_id,
        Some(shadowed_project_id),
        Some(root_folder_id),
        user_id,
        "leaf",
    )
    .await;
    let shadowed_document_id = Uuid::now_v7();
    seed_document_in_folder(
        &db.conn,
        workspace_id,
        shadowed_document_id,
        leaf_folder_id,
        user_id,
    )
    .await;

    // Inverted shape: only the root ancestor carries a project — this is the
    // "expected" inheritance direction and must still resolve correctly.
    let root_with_project_id = Uuid::now_v7();
    let leaf_without_project_id = Uuid::now_v7();
    let inherited_project_id = Uuid::now_v7();
    seed_project(
        &db.conn,
        workspace_id,
        inherited_project_id,
        user_id,
        "workspace",
        "editor",
    )
    .await;
    seed_folder(
        &db.conn,
        workspace_id,
        root_with_project_id,
        Some(inherited_project_id),
        None,
        user_id,
        "inherited-root",
    )
    .await;
    seed_folder(
        &db.conn,
        workspace_id,
        leaf_without_project_id,
        None,
        Some(root_with_project_id),
        user_id,
        "inherited-leaf",
    )
    .await;
    let inheriting_document_id = Uuid::now_v7();
    seed_document_in_folder(
        &db.conn,
        workspace_id,
        inheriting_document_id,
        leaf_without_project_id,
        user_id,
    )
    .await;

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let facts = source
        .load_subject_facts(
            &user_context(workspace_id, user_id),
            &[
                ProjectionSubject::Document(shadowed_document_id),
                ProjectionSubject::Document(inheriting_document_id),
            ],
        )
        .await
        .expect("load both inheritance chains");
    let [shadowed, inheriting] = facts.as_slice() else {
        panic!("expected one fact per document");
    };

    assert_chain(
        shadowed,
        &[
            ResourceRef::Document(atlas_acta::ids::DocumentId(shadowed_document_id)),
            ResourceRef::Folder(atlas_acta::ids::FolderId(leaf_folder_id)),
            ResourceRef::Folder(atlas_acta::ids::FolderId(root_folder_id)),
            ResourceRef::Workspace,
        ],
    );
    assert_chain(
        inheriting,
        &[
            ResourceRef::Document(atlas_acta::ids::DocumentId(inheriting_document_id)),
            ResourceRef::Folder(atlas_acta::ids::FolderId(leaf_without_project_id)),
            ResourceRef::Folder(atlas_acta::ids::FolderId(root_with_project_id)),
            ResourceRef::Project(atlas_acta::ids::ProjectId(inherited_project_id)),
            ResourceRef::Workspace,
        ],
    );

    db.teardown().await;
}

/// The agent cap (`apply_agent_cap`) never substitutes for the docs/tasks
/// read-capability gate. This exercises that interaction against a real
/// workspace-scope `admin` grant end to end, not the cap unit test's
/// synthetic `ResolutionInput`.
#[tokio::test]
async fn agent_cap_does_not_bypass_the_read_capability_gate() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let creator_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, creator_id, true).await;
    db.conn
        .execute_unprepared(&format!(
            "UPDATE acta.workspace_memberships SET role = 'owner' WHERE workspace_id = '{workspace_id}' AND user_id = '{creator_id}'"
        ))
        .await
        .expect("promote creator to owner");
    seed_api_key(&db.conn, key_id, creator_id, false, &["docs:read"]).await;
    seed_workspace_scope_grant(
        &db.conn,
        workspace_id,
        GrantPrincipal::ApiKey(key_id),
        "admin",
    )
    .await;
    seed_minimal_document(&db.conn, workspace_id, creator_id, document_id).await;

    let service = BatchAuthorizationService::new(PgBatchAuthorizationSource::new(db.conn.clone()));
    let context = ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::ApiKey(ApiKeyId(key_id)),
    );

    let decisions = service
        .authorize(&context, &[ProjectionSubject::Document(document_id)])
        .await
        .expect("authorize with the docs:read scope present");
    assert_eq!(decisions, vec![true]);

    db.conn
        .execute_unprepared(&format!(
            "UPDATE custos.api_keys SET scopes = ARRAY[]::text[] WHERE id = '{key_id}'"
        ))
        .await
        .expect("strip the capability scope");

    let decisions = service
        .authorize(&context, &[ProjectionSubject::Document(document_id)])
        .await
        .expect("authorize with no capability scope left");
    assert_eq!(
        decisions,
        vec![false],
        "an admin-strength workspace grant must not bypass a missing docs:read scope"
    );

    db.teardown().await;
}

/// The root/system-admin short circuit bypasses the membership requirement
/// entirely, for both principal kinds: a bare `is_root`/`is_system_admin`
/// flag stands in for `MemberRole::Admin` even with zero membership rows and
/// zero grants.
#[tokio::test]
async fn root_admin_short_circuits_membership_for_both_principal_kinds() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    seed_workspace_only(&db.conn, workspace_id).await;

    let root_user_id = Uuid::now_v7();
    seed_user(&db.conn, root_user_id, true, false).await;
    let root_document_id = Uuid::now_v7();
    seed_minimal_document(&db.conn, workspace_id, root_user_id, root_document_id).await;

    let admin_creator_id = Uuid::now_v7();
    seed_user(&db.conn, admin_creator_id, false, true).await;
    let key_id = Uuid::now_v7();
    seed_api_key(&db.conn, key_id, admin_creator_id, true, &["docs:read"]).await;
    let key_document_id = Uuid::now_v7();
    seed_minimal_document(&db.conn, workspace_id, admin_creator_id, key_document_id).await;

    let user_decisions =
        BatchAuthorizationService::new(PgBatchAuthorizationSource::new(db.conn.clone()))
            .authorize(
                &user_context(workspace_id, root_user_id),
                &[ProjectionSubject::Document(root_document_id)],
            )
            .await
            .expect("root user authorizes without a membership row");
    assert_eq!(user_decisions, vec![true]);

    let key_context = ProjectionAuthContext::from_validated(
        WorkspaceId(workspace_id),
        Principal::ApiKey(ApiKeyId(key_id)),
    );
    let key_decisions =
        BatchAuthorizationService::new(PgBatchAuthorizationSource::new(db.conn.clone()))
            .authorize(
                &key_context,
                &[ProjectionSubject::Document(key_document_id)],
            )
            .await
            .expect("api key authorizes via its system-admin creator, also without membership");
    assert_eq!(key_decisions, vec![true]);

    db.teardown().await;
}

/// The recursive folder walk only continues while `depth < 31`, so it ever
/// includes depths `0..=31` (32 folders). A project set on an ancestor beyond
/// that ceiling must never surface in the resolved chain.
#[tokio::test]
async fn folder_ancestry_truncates_at_the_chain_depth_ceiling() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;

    const CHAIN_LENGTH: usize = 35;
    let folder_ids: Vec<Uuid> = (0..CHAIN_LENGTH).map(|_| Uuid::now_v7()).collect();
    let beyond_ceiling_project_id = Uuid::now_v7();
    seed_project(
        &db.conn,
        workspace_id,
        beyond_ceiling_project_id,
        user_id,
        "workspace",
        "editor",
    )
    .await;

    // folder_ids[0] is the document's own folder (depth 0); folder_ids[last]
    // is the root, 34 hops away — past the depth-31 ceiling — and holds the
    // only project in the chain. Insert root-first so each `parent_folder_id`
    // FK target already exists.
    for index in (0..CHAIN_LENGTH).rev() {
        let folder_id = *folder_ids.get(index).expect("index within folder_ids");
        let parent = folder_ids.get(index + 1).copied();
        let project = if index + 1 == CHAIN_LENGTH {
            Some(beyond_ceiling_project_id)
        } else {
            None
        };
        seed_folder(
            &db.conn,
            workspace_id,
            folder_id,
            project,
            parent,
            user_id,
            &format!("f{index}"),
        )
        .await;
    }
    let document_id = Uuid::now_v7();
    let own_folder_id = *folder_ids.first().expect("chain has at least one folder");
    seed_document_in_folder(&db.conn, workspace_id, document_id, own_folder_id, user_id).await;

    let source = PgBatchAuthorizationSource::new(db.conn.clone());
    let facts = source
        .load_subject_facts(
            &user_context(workspace_id, user_id),
            &[ProjectionSubject::Document(document_id)],
        )
        .await
        .expect("load the truncated ancestry chain");
    let [fact] = facts.as_slice() else {
        panic!("expected exactly one fact");
    };

    const INCLUDED_DEPTHS: usize = 32;
    let mut expected = vec![ResourceRef::Document(atlas_acta::ids::DocumentId(
        document_id,
    ))];
    expected.extend(
        folder_ids
            .iter()
            .take(INCLUDED_DEPTHS)
            .map(|id| ResourceRef::Folder(atlas_acta::ids::FolderId(*id))),
    );
    expected.push(ResourceRef::Workspace);

    assert_chain(fact, &expected);

    db.teardown().await;
}

/// S3b3 addition (disclosed gap from the S3b2 verify pass): T3.1 above pins
/// `repos/permissions.rs:376`'s single-resource soft-deleted-group predicate,
/// used by `load_grants_for_resolution`. That is a distinct raw-SQL predicate
/// from the one the batch path actually exercises — `batch_authorization.rs`'s
/// `user_grants` CTE has its own `groups.deleted_at IS NULL` exclusion inside
/// `QUERY_B_PRINCIPAL_FACTS`. This test pins that batch-path predicate
/// end-to-end through `BatchAuthorizationService::authorize()`, independent
/// of T3.1, so the S3b3 port cut cannot silently drop it.
#[tokio::test]
async fn batch_principal_facts_excludes_a_soft_deleted_group_grant() {
    let db = BatchAuthorizationDb::create().await;
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let group_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();

    seed_workspace_user(&db.conn, workspace_id, user_id, true).await;
    seed_minimal_document(&db.conn, workspace_id, user_id, document_id).await;
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO custos.groups (id, workspace_id, name, created_by, created_at, updated_at) \
             VALUES ('{group_id}', '{workspace_id}', 'group', '{user_id}', now(), now()); \
             INSERT INTO custos.group_members (group_id, user_id, created_at) VALUES ('{group_id}', '{user_id}', now()); \
             INSERT INTO custos.permission_grants (id, workspace_id, group_id, resource_ref, role, created_at, updated_at) \
             VALUES ('{}', '{workspace_id}', '{group_id}', 'acta::workspace::{workspace_id}', 'editor', now(), now())",
            Uuid::now_v7(),
        ))
        .await
        .expect("seed live group grant");

    let service = BatchAuthorizationService::new(PgBatchAuthorizationSource::new(db.conn.clone()));
    let context = user_context(workspace_id, user_id);

    let decisions = service
        .authorize(&context, &[ProjectionSubject::Document(document_id)])
        .await
        .expect("authorize while the group is live");
    assert_eq!(decisions, vec![true]);

    db.conn
        .execute_unprepared(&format!(
            "UPDATE custos.groups SET deleted_at = now() WHERE id = '{group_id}'"
        ))
        .await
        .expect("soft-delete group");

    let decisions = service
        .authorize(&context, &[ProjectionSubject::Document(document_id)])
        .await
        .expect("authorize after the group is soft-deleted");
    assert_eq!(
        decisions,
        vec![false],
        "QUERY_B's user_grants CTE must exclude a grant held only through a \
         soft-deleted group, independent of repos/permissions.rs's own predicate"
    );

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

fn resolution_query(workspace_id: Uuid, user_id: Uuid, group_id: Uuid) -> ResolutionQuery {
    ResolutionQuery {
        workspace_id: atlas_custos::WorkspaceScope(workspace_id),
        user_id: Some(user_id),
        api_key_id: None,
        group_ids: vec![group_id],
        resource_refs: vec![atlas_acta::permissions::resource_ref_codec::to_core(
            &ResourceRef::Workspace,
            WorkspaceId(workspace_id),
        )],
    }
}

async fn seed_project(
    conn: &DatabaseConnection,
    workspace_id: Uuid,
    project_id: Uuid,
    creator_id: Uuid,
    visibility: &str,
    visibility_role: &str,
) {
    // task_prefix is unique per (workspace_id, task_prefix); derive one from
    // the project id's random tail (UUIDv7's leading bytes are a timestamp
    // shared across ids minted in the same test, so a prefix slice would
    // collide) so multiple projects can share a workspace in one test.
    let hex = project_id.simple().to_string();
    let task_prefix = format!("P{}", hex[hex.len() - 5..].to_uppercase());
    conn.execute_unprepared(&format!(
        "INSERT INTO projects (id, workspace_id, name, slug, task_prefix, next_task_number, visibility, visibility_role, created_by_user_id, created_at, updated_at) \
         VALUES ('{project_id}', '{workspace_id}', 'Project', 'project-{project_id}', '{task_prefix}', 1, '{visibility}', '{visibility_role}', '{creator_id}', now(), now())"
    ))
    .await
    .expect("seed project");
}

async fn seed_folder(
    conn: &DatabaseConnection,
    workspace_id: Uuid,
    folder_id: Uuid,
    project_id: Option<Uuid>,
    parent_folder_id: Option<Uuid>,
    creator_id: Uuid,
    name: &str,
) {
    let project_value = project_id
        .map(|id| format!("'{id}'"))
        .unwrap_or_else(|| "NULL".to_string());
    let parent_value = parent_folder_id
        .map(|id| format!("'{id}'"))
        .unwrap_or_else(|| "NULL".to_string());
    conn.execute_unprepared(&format!(
        "INSERT INTO folders (id, workspace_id, project_id, parent_folder_id, name, created_by_user_id, created_at, updated_at) \
         VALUES ('{folder_id}', '{workspace_id}', {project_value}, {parent_value}, '{name}', '{creator_id}', now(), now())"
    ))
    .await
    .expect("seed folder");
}

async fn seed_document_in_folder(
    conn: &DatabaseConnection,
    workspace_id: Uuid,
    document_id: Uuid,
    folder_id: Uuid,
    creator_id: Uuid,
) {
    conn.execute_unprepared(&format!(
        "INSERT INTO documents (id, workspace_id, folder_id, title, slug, content, frontmatter, current_revision_seq, created_by_user_id, created_at, updated_at) \
         VALUES ('{document_id}', '{workspace_id}', '{folder_id}', 'Document', 'document-{document_id}', '', '{{}}', 1, '{creator_id}', now(), now())"
    ))
    .await
    .expect("seed document in folder");
}

enum GrantPrincipal {
    User(Uuid),
    ApiKey(Uuid),
}

async fn seed_workspace_scope_grant(
    conn: &DatabaseConnection,
    workspace_id: Uuid,
    principal: GrantPrincipal,
    role: &str,
) {
    let (column, id) = match principal {
        GrantPrincipal::User(id) => ("user_id", id),
        GrantPrincipal::ApiKey(id) => ("api_key_id", id),
    };
    conn.execute_unprepared(&format!(
        "INSERT INTO custos.permission_grants (id, workspace_id, {column}, resource_ref, role, created_at, updated_at) \
         VALUES ('{}', '{workspace_id}', '{id}', 'acta::workspace::{workspace_id}', '{role}', now(), now())",
        Uuid::now_v7(),
    ))
    .await
    .expect("seed workspace-scope grant");
}

async fn seed_api_key(
    conn: &DatabaseConnection,
    key_id: Uuid,
    creator_id: Uuid,
    is_global: bool,
    scopes: &[&str],
) {
    let scopes_sql = scopes
        .iter()
        .map(|scope| format!("'{scope}'"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_unprepared(&format!(
        "INSERT INTO custos.api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, scopes) \
         VALUES ('{key_id}', NULL, '{creator_id}', 'key', 'hash', 'agent', now(), {is_global}, ARRAY[{scopes_sql}])"
    ))
    .await
    .expect("seed api key");
}

async fn seed_user(conn: &DatabaseConnection, user_id: Uuid, is_root: bool, is_system_admin: bool) {
    conn.execute_unprepared(&format!(
        "INSERT INTO custos.users (id, username, display_name, is_root, is_system_admin, created_at, updated_at) \
         VALUES ('{user_id}', 'user-{user_id}', 'User', {is_root}, {is_system_admin}, now(), now())"
    ))
    .await
    .expect("seed user");
}

async fn seed_workspace_only(conn: &DatabaseConnection, workspace_id: Uuid) {
    conn.execute_unprepared(&format!(
        "INSERT INTO acta.workspaces (id, name, slug, created_at, updated_at) \
         VALUES ('{workspace_id}', 'Workspace', 'workspace-{workspace_id}', now(), now())"
    ))
    .await
    .expect("seed workspace");
}

/// The exactly-two-statements contract (design §S3b, spec scenario "Statement
/// count is unchanged after the cut"). PR4 re-runs this exact assertion,
/// unmodified, against `ResourceChainSource`/`PrincipalFactsSource` after the
/// port cut: any regression that grows the batch to more than one subject
/// statement and one principal statement must fail here.
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
        "INSERT INTO custos.users (id, username, display_name, is_root, is_system_admin, created_at, updated_at) \
         VALUES ('{user_id}', 'user-{user_id}', 'User', false, false, now(), now()); \
         INSERT INTO acta.workspaces (id, name, slug, created_at, updated_at) \
         VALUES ('{workspace_id}', 'Workspace', 'workspace-{workspace_id}', now(), now())"
    ))
    .await
    .expect("seed user and workspace");

    if member {
        conn.execute_unprepared(&format!(
            "INSERT INTO acta.workspace_memberships (id, workspace_id, user_id, role, created_at, updated_at) \
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
        let database_url = atlas_test_db::fixture_database_url();
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
        crate::persistence::migrator::ComposedMigrator::up(&conn, None)
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
