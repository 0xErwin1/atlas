#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! S4 PR15 `SET SCHEMA acta` batch 5 migration (design §D1 batch 5, §D3) —
//! fifth and final of the five Acta SET SCHEMA batches, moving the
//! search/attachments/lifecycle-group tables. After this migration lands,
//! every table in the D1 36-table Acta inventory lives in `acta.*` (or
//! `platform.ui_state` for `user_ui_state`, PR9); none remain in `public.*`.

mod support;

use std::sync::Arc;

use async_trait::async_trait;
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::entities::lifecycle::{RestoreTarget, SecurityAuditRef, TrashKind};
use atlas_acta::entities::workspace_core::NewProject;
use atlas_acta::permissions::{Visibility, VisibilityRole};
use atlas_acta::semantic_search::{
    EmbeddingInput, EmbeddingProvider, ResourceKind, SemanticIndexChunk, SemanticSearchQuery,
    SemanticSearchRepo, SemanticSearchSource, SemanticSearchTypeFilter,
};
use atlas_acta_postgres::repos::documents::{DocumentRepo, PgDocumentRepo};
use atlas_core::error::DomainError;
use atlas_core::principal::{Principal, UserId};
use atlas_server::persistence::migrator::ComposedMigrator;
use atlas_server::persistence::repos::{
    NewPurgeOperation, PgProjectRepo, PgPurgeOperationRepo, PgSemanticIndexWriter,
    PgSemanticSearchRepo, ProjectRepo, UserRepo,
};
use sea_orm::{FromQueryResult, Statement};
use sea_orm_migration::prelude::MigratorTrait;

const ACTA_SEARCH_ATTACHMENTS_LIFECYCLE_TABLES: &[&str] = &[
    "search_embeddings",
    "search_index_queue",
    "purge_operations",
    "purge_operation_digests",
];

/// All four search/attachments/lifecycle-group tables must live in the
/// `acta` schema after the migration; every other batch's tables
/// (`workspaces`, `boards`, `comments`) also already live there, so the D1
/// 36-table inventory is complete post-move — the last "stays public" carve
/// out (`purge_operations`, tracked by every earlier batch's own test) is
/// retired by this migration.
#[tokio::test]
async fn search_attachments_lifecycle_group_tables_move_to_the_acta_schema() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        table_schema: String,
    }

    let all_names: Vec<String> = ACTA_SEARCH_ATTACHMENTS_LIFECYCLE_TABLES
        .iter()
        .chain(["workspaces", "boards", "comments"].iter())
        .map(|t| format!("'{t}'"))
        .collect();

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            r#"
            SELECT table_name, table_schema
            FROM information_schema.tables
            WHERE table_name IN ({})
            "#,
            all_names.join(",")
        ),
    ))
    .all(db.conn())
    .await
    .expect("query information_schema.tables");

    for table in ACTA_SEARCH_ATTACHMENTS_LIFECYCLE_TABLES
        .iter()
        .chain(["workspaces", "boards", "comments"].iter())
    {
        let schema = rows
            .iter()
            .find(|r| r.table_name == *table)
            .unwrap_or_else(|| panic!("table {table} not found"))
            .table_schema
            .clone();
        assert_eq!(schema, "acta", "expected {table} to live in acta");
    }

    db.teardown().await;
}

/// The final "no functional table remains in `public`" proof for this
/// slice's own tables (the epic-level version, covering every product's
/// tables, lands in PR16): every one of the 36 D1-inventoried Acta tables
/// resolves to `acta`, confirmed via a single query rather than 36 assertion
/// blocks scattered across five test files.
#[tokio::test]
async fn no_d1_acta_table_remains_in_public_after_the_final_batch() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    const D1_ACTA_TABLES: &[&str] = &[
        "workspaces",
        "workspace_memberships",
        "property_definitions",
        "projects",
        "folders",
        "documents",
        "document_revisions",
        "document_links",
        "attachments",
        "attachment_write_intents",
        "comment_attachment_drafts",
        "comment_attachment_draft_uploads",
        "boards",
        "board_columns",
        "tasks",
        "task_references",
        "task_assignees",
        "task_checklist_items",
        "task_activity",
        "workspace_status_templates",
        "platform_status_templates",
        "comments",
        "comment_links",
        "comment_link_events",
        "tags",
        "events_outbox",
        "webhook_subscriptions",
        "webhook_delivery_log",
        "automation_rules",
        "integration_configs",
        "saved_searches",
        "task_views",
        "search_embeddings",
        "search_index_queue",
        "purge_operations",
        "purge_operation_digests",
    ];
    assert_eq!(D1_ACTA_TABLES.len(), 36, "expected the full D1 inventory");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        table_schema: String,
    }

    let all_names: Vec<String> = D1_ACTA_TABLES.iter().map(|t| format!("'{t}'")).collect();

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            r#"
            SELECT table_name, table_schema
            FROM information_schema.tables
            WHERE table_name IN ({})
            "#,
            all_names.join(",")
        ),
    ))
    .all(db.conn())
    .await
    .expect("query information_schema.tables");

    for table in D1_ACTA_TABLES {
        let schema = rows
            .iter()
            .find(|r| r.table_name == *table)
            .unwrap_or_else(|| panic!("table {table} not found"))
            .table_schema
            .clone();
        assert_eq!(
            schema, "acta",
            "expected {table} to live in acta, not public"
        );
    }

    db.teardown().await;
}

/// `ALTER TABLE ... SET SCHEMA` moves a table by OID without dropping or
/// recreating it, so every inbound/outbound foreign key must survive
/// unchanged and still resolve to the moved table.
///
/// **Correction against a live query (mirrors PR11's T11.7 deviation)**:
/// design §D3's cross-schema FK table names
/// `purge_operations.commit_audit_id → security_audit_log` as a surviving
/// FK. A live query against the pre-PR15 database shows no such constraint
/// exists — `m20260830_000050_grant_resource_ref` (S3's O1 migration)
/// already dropped `purge_operations_commit_audit_id_fkey` in its `up()`.
/// The FKs enumerated below are the ones a live `pg_constraint` query
/// actually returns: `purge_operations_original_actor_user_id_fkey` (the
/// cross-schema Acta→Custos edge that does survive, per spec's carve-out),
/// `purge_operations_workspace_id_fkey` (internal, into PR11's
/// already-moved `acta.workspaces`), and
/// `purge_operation_digests_operation_id_fkey` (internal to this batch,
/// both tables move together).
#[tokio::test]
async fn foreign_keys_survive_the_schema_move() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        conname: String,
        table_schema: String,
        table_name: String,
        ref_schema: String,
        ref_table: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT conname,
               nt.nspname AS table_schema,
               tc.relname AS table_name,
               nc.nspname AS ref_schema,
               rc.relname AS ref_table
        FROM pg_constraint
        JOIN pg_class tc ON tc.oid = conrelid
        JOIN pg_namespace nt ON nt.oid = tc.relnamespace
        JOIN pg_class rc ON rc.oid = confrelid
        JOIN pg_namespace nc ON nc.oid = rc.relnamespace
        WHERE contype = 'f'
          AND conname IN (
              'purge_operations_original_actor_user_id_fkey',
              'purge_operations_workspace_id_fkey',
              'purge_operation_digests_operation_id_fkey'
          )
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_constraint for representative foreign keys");

    let expectations = [
        (
            "purge_operations_original_actor_user_id_fkey",
            "acta",
            "purge_operations",
            "custos",
            "users",
        ),
        (
            "purge_operations_workspace_id_fkey",
            "acta",
            "purge_operations",
            "acta",
            "workspaces",
        ),
        (
            "purge_operation_digests_operation_id_fkey",
            "acta",
            "purge_operation_digests",
            "acta",
            "purge_operations",
        ),
    ];

    for (conname, table_schema, table_name, ref_schema, ref_table) in expectations {
        let row = rows
            .iter()
            .find(|r| r.conname == conname)
            .unwrap_or_else(|| panic!("constraint {conname} not found after the schema move"));
        assert_eq!(row.table_schema, table_schema, "table schema for {conname}");
        assert_eq!(row.table_name, table_name, "table name for {conname}");
        assert_eq!(row.ref_schema, ref_schema, "ref schema for {conname}");
        assert_eq!(row.ref_table, ref_table, "ref table for {conname}");
    }

    db.teardown().await;
}

/// `purge_operations.commit_audit_id → security_audit_log` genuinely has no
/// live FK constraint today (see the deviation documented on
/// `foreign_keys_survive_the_schema_move` and in the migration's own module
/// doc comment) — this is a boundary proof that the absence is real and not
/// an artifact of a typo'd constraint name in the positive test above.
#[tokio::test]
async fn purge_operations_commit_audit_id_carries_no_live_fk_constraint() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        conname: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT tc.constraint_name AS conname
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
            .map(|row| row.conname.clone())
            .collect::<Vec<_>>()
    );

    db.teardown().await;
}

/// Special-attention item from the brief: `search_embeddings_ann_idx` is an
/// IVFFLAT index (`USING ivfflat (embedding vector_cosine_ops) WITH (lists =
/// 100)`), not HNSW. `SET SCHEMA` moves a table (and its indexes) by OID
/// without dropping or recreating either, so the index survives the move
/// unchanged — this proves it live via `pg_indexes` rather than trusting the
/// migration's own doc comment.
#[tokio::test]
async fn the_vector_ann_index_survives_the_schema_move() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct IndexRow {
        schemaname: String,
        indexname: String,
        indexdef: String,
    }

    let rows = IndexRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT schemaname, indexname, indexdef
        FROM pg_indexes
        WHERE tablename = 'search_embeddings'
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_indexes for search_embeddings");

    let ann_index = rows
        .iter()
        .find(|row| row.indexname == "search_embeddings_ann_idx")
        .expect("search_embeddings_ann_idx must survive the schema move");

    assert_eq!(ann_index.schemaname, "acta");
    assert!(
        ann_index.indexdef.contains("ivfflat"),
        "expected the ANN index to stay ivfflat, got: {}",
        ann_index.indexdef
    );
    assert!(
        ann_index.indexdef.contains("vector_cosine_ops"),
        "expected the ANN index to keep vector_cosine_ops, got: {}",
        ann_index.indexdef
    );
    assert!(
        ann_index.indexdef.contains("acta.search_embeddings"),
        "expected the index definition to reference acta.search_embeddings, got: {}",
        ann_index.indexdef
    );

    let other_indexes = [
        "search_embeddings_pkey",
        "search_embeddings_workspace_resource_idx",
        "search_embeddings_model_dimensions_stale_idx",
        "search_embeddings_workspace_id_resource_kind_resource_id_so_key",
    ];
    for name in other_indexes {
        let row = rows
            .iter()
            .find(|row| row.indexname == name)
            .unwrap_or_else(|| panic!("{name} must survive the schema move"));
        assert_eq!(row.schemaname, "acta");
    }

    db.teardown().await;
}

#[derive(Debug)]
struct DeterministicProvider;

#[async_trait]
impl EmbeddingProvider for DeterministicProvider {
    async fn embed(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError> {
        Ok(inputs
            .iter()
            .map(|input| {
                let mut vector = vec![0.0; Self::DIMENSIONS];
                if input.text.contains("runbook") {
                    if let Some(value) = vector.get_mut(0) {
                        *value = 1.0;
                    }
                } else if let Some(value) = vector.get_mut(1) {
                    *value = 1.0;
                }
                vector
            })
            .collect())
    }

    fn model(&self) -> &str {
        "batch5-set-schema-probe"
    }

    fn dimensions(&self) -> usize {
        Self::DIMENSIONS
    }
}

impl DeterministicProvider {
    /// The `search_embeddings.embedding` column is declared `vector(1536)`
    /// (`crates/migration/src/m20260708_000039_search_embeddings.rs`); the
    /// provider must match that fixed width regardless of how little of it a
    /// deterministic test vector actually uses.
    const DIMENSIONS: usize = 1536;
}

/// Special-attention item from the brief: a live similarity query through
/// the semantic-search repo (`PgSemanticSearchRepo::search`, the ANN-driven
/// arm proven structurally above) must still return correct results reading
/// from `acta.search_embeddings` post-move — not just the index metadata,
/// the actual query path.
#[tokio::test]
async fn semantic_search_repo_still_returns_hits_reading_the_moved_table() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, owner) = support::seed_workspace(&db, "batch5-semantic").await;
    let owner_ctx = support::ctx(&workspace, &owner);

    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &owner_ctx,
        NewProject {
            name: "Batch 5 Project".to_owned(),
            slug: "batch5-project".to_owned(),
            task_prefix: "B5".to_owned(),
            visibility: Visibility::Public(VisibilityRole::Viewer),
        },
    )
    .await
    .expect("seed project");

    let doc = PgDocumentRepo::new(db.conn().clone(), 50)
        .create(
            &owner_ctx,
            NewDocument {
                title: "Postgres runbook".to_owned(),
                slug: None,
                content: "runbook content".to_owned(),
                folder_id: None,
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");

    let provider = Arc::new(DeterministicProvider);
    let writer = PgSemanticIndexWriter::new(db.conn().clone(), provider.clone());
    writer
        .index_chunks(&[SemanticIndexChunk {
            workspace_id: workspace.id,
            kind: ResourceKind::Document,
            resource_id: doc.id.0,
            source: SemanticSearchSource::Aggregate,
            chunk_ordinal: 0,
            content_hash: "runbook content".to_owned(),
            text: "runbook content".to_owned(),
            excerpt: "runbook content".to_owned(),
        }])
        .await
        .expect("index chunk into acta.search_embeddings");

    let repo = PgSemanticSearchRepo::new(db.conn().clone(), provider);
    let hits = repo
        .search(&SemanticSearchQuery::new(
            workspace.id,
            Principal::User(owner.id),
            "runbook".to_owned(),
            SemanticSearchTypeFilter::all(),
            10,
            None,
            true,
            true,
            true,
        ))
        .await
        .expect("semantic search must succeed reading acta.search_embeddings");

    assert_eq!(hits.len(), 1, "expected exactly one hit: {hits:?}");
    let hit = hits.first().expect("length asserted above");
    assert_eq!(hit.id, doc.id.0);
    assert!(hit.similarity > 0.9);

    db.teardown().await;
}

/// The composed migrator (`historical() ++ custos_new() ++ acta_new()`)
/// reports zero pending migrations against a from-empty database, and
/// includes this PR's SET SCHEMA migration by name (D5 regression guard,
/// extended for this PR's addition — the last of the five).
#[tokio::test]
async fn composed_migrator_has_zero_pending_including_the_set_schema_migration() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    assert!(
        ComposedMigrator::get_pending_migrations(db.conn())
            .await
            .expect("get_pending_migrations")
            .is_empty(),
        "expected zero pending migrations from a from-empty database"
    );

    #[derive(Debug, FromQueryResult)]
    struct MigrationRow {
        version: String,
    }

    let applied: Vec<String> = MigrationRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT version FROM seaql_migrations".to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query seaql_migrations")
    .into_iter()
    .map(|r| r.version)
    .collect();
    assert!(
        applied
            .contains(&"m20260905_000057_acta_search_attachments_lifecycle_set_schema".to_string()),
        "expected the SET SCHEMA migration to be part of the applied set"
    );

    db.teardown().await;
}

/// The permanent regression form of this PR's PL/pgSQL function-body audit
/// (mirrors T12.1/PR14's equivalent): queries `pg_proc` on a live,
/// fully-migrated database and asserts no `plpgsql` routine's body contains
/// an unqualified reference to any of the four search/attachments/lifecycle
/// tables.
#[tokio::test]
async fn no_plpgsql_routine_references_a_search_attachments_lifecycle_table_unqualified() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct FuncRow {
        proname: String,
        prosrc: String,
    }

    let funcs = FuncRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT p.proname, p.prosrc
        FROM pg_proc p
        JOIN pg_namespace n ON n.oid = p.pronamespace
        JOIN pg_language l ON l.oid = p.prolang
        WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
          AND l.lanname = 'plpgsql'
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_proc for plpgsql routines");

    let mut violations = Vec::new();
    for func in &funcs {
        for table in ACTA_SEARCH_ATTACHMENTS_LIFECYCLE_TABLES {
            for keyword in ["FROM", "JOIN", "INTO", "UPDATE"] {
                let unqualified = format!("{keyword} {table}");
                if func.prosrc.contains(&unqualified) {
                    violations.push(format!("{}: unqualified `{unqualified}`", func.proname));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found a PL/pgSQL routine with an unqualified reference to a moved table:\n{}",
        violations.join("\n")
    );

    db.teardown().await;
}

/// Repo-level smoke test (T15.8 baseline, `PgPurgeOperationRepo`): the same
/// operation lifecycle `search_index_queue_lifecycle_repos_characterization.rs`
/// exercised pre-move must still round-trip after this batch's tables move.
#[tokio::test]
async fn purge_operation_repo_round_trips_after_the_schema_move() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, _owner) = support::seed_workspace(&db, "batch5-purge").await;

    let audit_user = db
        .user_repo()
        .create(atlas_server::persistence::repos::NewUser {
            username: "batch5-purge-actor".to_owned(),
            display_name: "batch5-purge-actor".to_owned(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed actor user");

    let repo = PgPurgeOperationRepo;
    let operation = repo
        .create_in(
            db.conn(),
            NewPurgeOperation {
                workspace_id: workspace.id,
                target: RestoreTarget {
                    kind: TrashKind::Document,
                    target_id: uuid::Uuid::now_v7(),
                },
                original_actor_user_id: UserId(audit_user.id.0),
                commit_audit_id: SecurityAuditRef(uuid::Uuid::now_v7()),
            },
        )
        .await
        .expect("create purge operation");

    let digest = repo
        .create_digest_in(db.conn(), operation.id, "digest-1".to_owned())
        .await
        .expect("create purge digest");
    assert_eq!(digest.operation_id, operation.id);

    let digests = repo
        .list_digests_in(db.conn(), operation.id)
        .await
        .expect("list digests");
    assert_eq!(digests.len(), 1);

    db.teardown().await;
}
