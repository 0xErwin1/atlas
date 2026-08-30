#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! O1 grant-target migration (design §S3c, spec "PR5 — O1 grant-target
//! migration replaces target columns with `resource_ref`").

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement};
use sea_orm_migration::prelude::MigratorTrait;
use uuid::Uuid;

/// Golden encoding pin (T5.2): the four backfilled `resource_ref` strings
/// must match `resource_ref_codec::to_core` byte-for-byte, so the migration's
/// raw SQL concatenation (`'acta::<kind>::' || id::text`) and the Rust codec
/// can never silently drift from each other.
#[test]
fn golden_encoding_pins_the_migration_sql_to_the_codec() {
    let workspace = atlas_acta::ids::WorkspaceId(Uuid::now_v7());
    let project = atlas_acta::ids::ProjectId(Uuid::now_v7());
    let folder = atlas_acta::ids::FolderId(Uuid::now_v7());
    let document = atlas_acta::ids::DocumentId(Uuid::now_v7());
    let board = atlas_acta::ids::BoardId(Uuid::now_v7());

    let cases = [
        (
            atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Workspace,
                workspace,
            )
            .to_string(),
            format!("acta::workspace::{}", workspace.0),
        ),
        (
            atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Project(project),
                workspace,
            )
            .to_string(),
            format!("acta::project::{}", project.0),
        ),
        (
            atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Folder(folder),
                workspace,
            )
            .to_string(),
            format!("acta::folder::{}", folder.0),
        ),
        (
            atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Document(document),
                workspace,
            )
            .to_string(),
            format!("acta::document::{}", document.0),
        ),
        (
            atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Board(board),
                workspace,
            )
            .to_string(),
            format!("acta::board::{}", board.0),
        ),
    ];

    for (codec_output, sql_shape) in cases {
        assert_eq!(codec_output, sql_shape);
    }
}

/// Backfill test (T5.3–T5.5): seeds `permission_grants` rows in the pre-migration
/// shape (one per target kind, plus a workspace-scope row) against a database
/// paused right before the O1 migration, runs the migration, then asserts every
/// row's `resource_ref` matches the codec's encoding exactly.
#[tokio::test]
async fn backfill_encodes_every_target_kind_exactly_like_the_codec() {
    let historical_count = migration::Migrator::migrations().len() as u32;
    let db = support::TestDb::create_with_migration_steps(Some(historical_count))
        .await
        .expect("V1-migrated TestDb");

    let workspace_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let folder_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();
    let board_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();

    // custos-schema-gate:off — frozen at `historical_count` steps, before
    // `custos_new()` (and the SET SCHEMA migration) has run; `users` and
    // `permission_grants` still physically live in `public` here.
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO users (id, username, display_name, is_root, is_system_admin, created_at, updated_at) \
             VALUES ('{user_id}', 'user-{user_id}', 'User', false, false, now(), now()); \
             INSERT INTO workspaces (id, name, slug, created_at, updated_at) \
             VALUES ('{workspace_id}', 'Workspace', 'workspace-{workspace_id}', now(), now()); \
             INSERT INTO projects (id, workspace_id, name, slug, task_prefix, next_task_number, visibility, created_by_user_id, created_at, updated_at) \
             VALUES ('{project_id}', '{workspace_id}', 'Project', 'project-{project_id}', 'PRJ', 1, 'workspace', '{user_id}', now(), now()); \
             INSERT INTO folders (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{folder_id}', '{workspace_id}', '{project_id}', 'Folder', '{user_id}', now(), now()); \
             INSERT INTO documents (id, workspace_id, folder_id, title, slug, content, frontmatter, current_revision_seq, created_by_user_id, created_at, updated_at) \
             VALUES ('{document_id}', '{workspace_id}', '{folder_id}', 'Document', 'document-{document_id}', '', '{{}}', 1, '{user_id}', now(), now()); \
             INSERT INTO boards (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) \
             VALUES ('{board_id}', '{workspace_id}', '{project_id}', 'Board', '{user_id}', now(), now())"
        ))
        .await
        .expect("seed user, workspace, and the four target resources");

    let workspace_grant_id = Uuid::now_v7();
    let project_grant_id = Uuid::now_v7();
    let folder_grant_id = Uuid::now_v7();
    let document_grant_id = Uuid::now_v7();
    let board_grant_id = Uuid::now_v7();

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO permission_grants (id, workspace_id, user_id, role, created_at, updated_at) \
             VALUES ('{workspace_grant_id}', '{workspace_id}', '{user_id}', 'viewer', now(), now()); \
             INSERT INTO permission_grants (id, workspace_id, user_id, project_id, role, created_at, updated_at) \
             VALUES ('{project_grant_id}', '{workspace_id}', '{user_id}', '{project_id}', 'viewer', now(), now()); \
             INSERT INTO permission_grants (id, workspace_id, user_id, folder_id, role, created_at, updated_at) \
             VALUES ('{folder_grant_id}', '{workspace_id}', '{user_id}', '{folder_id}', 'viewer', now(), now()); \
             INSERT INTO permission_grants (id, workspace_id, user_id, document_id, role, created_at, updated_at) \
             VALUES ('{document_grant_id}', '{workspace_id}', '{user_id}', '{document_id}', 'viewer', now(), now()); \
             INSERT INTO permission_grants (id, workspace_id, user_id, board_id, role, created_at, updated_at) \
             VALUES ('{board_grant_id}', '{workspace_id}', '{user_id}', '{board_id}', 'viewer', now(), now())"
        ))
        .await
        .expect("seed pre-migration grant rows");
    // custos-schema-gate:on

    db.run_remaining_migrations()
        .await
        .expect("run the O1 migration");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        id: Uuid,
        resource_ref: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT id, resource_ref FROM custos.permission_grants WHERE workspace_id = '{workspace_id}' ORDER BY resource_ref"
        ),
    ))
    .all(db.conn())
    .await
    .expect("query backfilled resource_ref values");

    let expected = [
        (
            workspace_grant_id,
            format!("acta::workspace::{workspace_id}"),
        ),
        (project_grant_id, format!("acta::project::{project_id}")),
        (folder_grant_id, format!("acta::folder::{folder_id}")),
        (document_grant_id, format!("acta::document::{document_id}")),
        (board_grant_id, format!("acta::board::{board_id}")),
    ];

    for (grant_id, expected_ref) in expected {
        let row = rows
            .iter()
            .find(|row| row.id == grant_id)
            .unwrap_or_else(|| panic!("grant {grant_id} missing after migration"));
        assert_eq!(row.resource_ref, expected_ref);
    }

    db.teardown().await;
}

/// Zero-outbound-FK test (T5.10): after the O1 migration, none of the eight
/// Custos-owned tables (§D1/§S3d list) may hold a foreign key pointing at a
/// table outside that set — the nine D1 constraints (five on
/// `permission_grants`, plus `groups`, `api_keys`, `security_audit_log`, and
/// `purge_operations.commit_audit_id`) are exactly what step 7 of the O1
/// migration drops.
#[tokio::test]
async fn no_custos_table_has_an_outbound_foreign_key_after_the_migration() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    const CUSTOS_TABLES: &str = "'users','sessions','user_activation_tokens','api_keys',\
        'groups','group_members','permission_grants','security_audit_log'";

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        constraint_name: String,
        foreign_table: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            r#"
            SELECT tc.table_name, tc.constraint_name, ccu.table_name AS foreign_table
            FROM information_schema.table_constraints tc
            JOIN information_schema.constraint_column_usage ccu
                ON tc.constraint_name = ccu.constraint_name
                AND tc.constraint_schema = ccu.constraint_schema
            WHERE tc.constraint_type = 'FOREIGN KEY'
              AND tc.table_name IN ({CUSTOS_TABLES})
              AND ccu.table_name NOT IN ({CUSTOS_TABLES})
            "#
        ),
    ))
    .all(db.conn())
    .await
    .expect("query information_schema for outbound FKs");

    assert!(
        rows.is_empty(),
        "expected zero outbound FKs from the eight Custos tables, found: {:?}",
        rows.iter()
            .map(|row| format!(
                "{}.{} -> {}",
                row.table_name, row.constraint_name, row.foreign_table
            ))
            .collect::<Vec<_>>()
    );

    db.teardown().await;
}

/// The `purge_operations.commit_audit_id -> security_audit_log` FK (the ninth
/// D1 constraint, inbound to Acta rather than outbound from Custos) must also
/// be gone: `security_audit_log` is Custos-owned, so an Acta table holding a
/// hard FK into it violates the same "no cross-schema FK" gate from the other
/// direction.
#[tokio::test]
async fn purge_operations_no_longer_fk_references_security_audit_log() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        constraint_name: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT tc.constraint_name
        FROM information_schema.table_constraints tc
        JOIN information_schema.constraint_column_usage ccu
            ON tc.constraint_name = ccu.constraint_name
            AND tc.constraint_schema = ccu.constraint_schema
        WHERE tc.constraint_type = 'FOREIGN KEY'
          AND tc.table_name = 'purge_operations'
          AND ccu.table_name = 'security_audit_log'
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query information_schema for the purge_operations FK");

    assert!(
        rows.is_empty(),
        "expected purge_operations to hold no FK into security_audit_log, found: {:?}",
        rows.iter()
            .map(|row| row.constraint_name.clone())
            .collect::<Vec<_>>()
    );

    db.teardown().await;
}

/// Down-path roundtrip (T5.13): asserts down() restores the pre-migration
/// shape (columns, unique index, nine FKs), drops an orphaned grant instead
/// of aborting the FK re-add, and re-applying up() leaves zero pending.
#[tokio::test]
async fn down_restores_target_columns_and_survives_a_forward_only_orphan() {
    let db = support::TestDb::create()
        .await
        .expect("fully migrated TestDb");

    let workspace_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let folder_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();
    let board_id = Uuid::now_v7();
    let orphan_document_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.users (id, username, display_name, is_root, is_system_admin, created_at, updated_at) VALUES ('{user_id}', 'user-{user_id}', 'User', false, false, now(), now()); \
             INSERT INTO workspaces (id, name, slug, created_at, updated_at) VALUES ('{workspace_id}', 'Workspace', 'workspace-{workspace_id}', now(), now()); \
             INSERT INTO projects (id, workspace_id, name, slug, task_prefix, next_task_number, visibility, created_by_user_id, created_at, updated_at) VALUES ('{project_id}', '{workspace_id}', 'Project', 'project-{project_id}', 'PRJ', 1, 'workspace', '{user_id}', now(), now()); \
             INSERT INTO folders (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) VALUES ('{folder_id}', '{workspace_id}', '{project_id}', 'Folder', '{user_id}', now(), now()); \
             INSERT INTO documents (id, workspace_id, folder_id, title, slug, content, frontmatter, current_revision_seq, created_by_user_id, created_at, updated_at) VALUES ('{document_id}', '{workspace_id}', '{folder_id}', 'Document', 'document-{document_id}', '', '{{}}', 1, '{user_id}', now(), now()); \
             INSERT INTO boards (id, workspace_id, project_id, name, created_by_user_id, created_at, updated_at) VALUES ('{board_id}', '{workspace_id}', '{project_id}', 'Board', '{user_id}', now(), now())"
        ))
        .await
        .expect("seed user, workspace, and the four target resources");

    let workspace_grant_id = Uuid::now_v7();
    let project_grant_id = Uuid::now_v7();
    let folder_grant_id = Uuid::now_v7();
    let document_grant_id = Uuid::now_v7();
    let board_grant_id = Uuid::now_v7();
    let orphan_grant_id = Uuid::now_v7();

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.permission_grants (id, workspace_id, user_id, resource_ref, role, created_at, updated_at) VALUES \
             ('{workspace_grant_id}', '{workspace_id}', '{user_id}', 'acta::workspace::{workspace_id}', 'viewer', now(), now()), ('{project_grant_id}', '{workspace_id}', '{user_id}', 'acta::project::{project_id}', 'viewer', now(), now()), \
             ('{folder_grant_id}', '{workspace_id}', '{user_id}', 'acta::folder::{folder_id}', 'viewer', now(), now()), ('{document_grant_id}', '{workspace_id}', '{user_id}', 'acta::document::{document_id}', 'viewer', now(), now()), \
             ('{board_grant_id}', '{workspace_id}', '{user_id}', 'acta::board::{board_id}', 'viewer', now(), now()), ('{orphan_grant_id}', '{workspace_id}', '{user_id}', 'acta::document::{orphan_document_id}', 'viewer', now(), now())"
        ))
        .await
        .expect("seed post-migration grant rows, including one orphaned by a never-live target");

    // Reverts two steps, not one: S3d appended `m20260830_000051_custos_set_schema`
    // after this migration in `custos_new()`, so "the last applied migration" is
    // now the schema move rather than the O1 migration under test here. Reverting
    // two steps undoes the schema move first (moving the eight tables back to
    // `public`) and then O1's own down(), landing on the same pre-O1,
    // unqualified-table-name state this test asserted before S3d existed.
    ComposedMigrator::down(db.conn(), Some(2))
        .await
        .expect("down survives an orphaned grant");

    #[derive(Debug, FromQueryResult)]
    struct TargetRow {
        id: Uuid,
        project_id: Option<Uuid>,
        folder_id: Option<Uuid>,
        document_id: Option<Uuid>,
        board_id: Option<Uuid>,
    }

    // custos-schema-gate:off — after the two-step down() above, SET SCHEMA has
    // been reverted, so `permission_grants` is back in `public` at this point.
    let rows = TargetRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!("SELECT id, project_id, folder_id, document_id, board_id FROM permission_grants WHERE workspace_id = '{workspace_id}'"),
    ))
    .all(db.conn())
    .await
    .expect("query restored target columns");
    // custos-schema-gate:on

    assert!(
        !rows.iter().any(|row| row.id == orphan_grant_id),
        "orphan grant not dropped by down()"
    );

    for (grant_id, expected) in [
        (workspace_grant_id, (None, None, None, None)),
        (project_grant_id, (Some(project_id), None, None, None)),
        (folder_grant_id, (None, Some(folder_id), None, None)),
        (document_grant_id, (None, None, Some(document_id), None)),
        (board_grant_id, (None, None, None, Some(board_id))),
    ] {
        let row = rows
            .iter()
            .find(|row| row.id == grant_id)
            .unwrap_or_else(|| panic!("grant {grant_id} missing after down()"));
        assert_eq!(
            (row.project_id, row.folder_id, row.document_id, row.board_id),
            expected
        );
    }

    async fn count(conn: &sea_orm::DatabaseConnection, sql: &str) -> i64 {
        #[derive(Debug, FromQueryResult)]
        struct CountRow {
            count: i64,
        }
        CountRow::find_by_statement(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql.to_string(),
        ))
        .one(conn)
        .await
        .expect("count query")
        .expect("count query always returns one row")
        .count
    }

    assert_eq!(
        count(db.conn(), "SELECT count(*) AS count FROM pg_indexes WHERE indexname = 'permission_grants_uq' AND indexdef LIKE '%project_id%' AND indexdef LIKE '%folder_id%' AND indexdef LIKE '%document_id%' AND indexdef LIKE '%board_id%'").await,
        1,
        "expected the restored 8-column permission_grants_uq index"
    );
    assert_eq!(
        count(db.conn(), "SELECT count(*) AS count FROM pg_constraint WHERE contype = 'f' AND conname IN ('permission_grants_workspace_id_fkey', 'permission_grants_project_id_fkey', 'permission_grants_folder_id_fkey', 'permission_grants_document_id_fkey', 'permission_grants_board_id_fkey', 'groups_workspace_id_fkey', 'api_keys_workspace_id_fkey', 'security_audit_log_workspace_id_fkey', 'purge_operations_commit_audit_id_fkey')").await,
        9,
        "expected all nine FKs restored"
    );

    ComposedMigrator::up(db.conn(), None)
        .await
        .expect("re-applying the migration after down()");
    assert!(
        ComposedMigrator::get_pending_migrations(db.conn())
            .await
            .expect("get_pending_migrations")
            .is_empty(),
        "expected zero pending migrations after re-applying up()"
    );

    db.teardown().await;
}
