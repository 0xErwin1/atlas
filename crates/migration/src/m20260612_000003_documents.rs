use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260612_000003_documents"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE documents (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                project_id UUID REFERENCES projects(id),
                folder_id UUID REFERENCES folders(id),
                title TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                frontmatter JSONB NOT NULL DEFAULT '{}',
                current_revision_id UUID,
                current_revision_seq BIGINT NOT NULL DEFAULT 0,
                search_vector TSVECTOR GENERATED ALWAYS AS (
                    setweight(to_tsvector('simple', coalesce(title, '')), 'A') ||
                    setweight(to_tsvector('simple', coalesce(content, '')), 'B')
                ) STORED,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT documents_num_actors_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX documents_workspace_folder_idx ON documents (workspace_id, folder_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX documents_workspace_project_idx ON documents (workspace_id, project_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX documents_frontmatter_gin ON documents USING gin (frontmatter jsonb_path_ops)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX documents_search_vector_gin ON documents USING gin (search_vector)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE document_revisions (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
                seq BIGINT NOT NULL,
                patch TEXT,
                snapshot TEXT,
                is_anchor BOOLEAN NOT NULL DEFAULT false,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                UNIQUE (document_id, seq),
                CONSTRAINT document_revisions_seq_check CHECK (seq >= 1),
                CONSTRAINT document_revisions_anchor_check
                    CHECK (
                        (is_anchor AND snapshot IS NOT NULL)
                        OR (NOT is_anchor AND patch IS NOT NULL AND snapshot IS NULL)
                    ),
                CONSTRAINT document_revisions_num_actors_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX document_revisions_anchor_idx
               ON document_revisions (document_id, seq DESC)
               WHERE is_anchor"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE document_links (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                source_document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
                target_document_id UUID REFERENCES documents(id) ON DELETE SET NULL,
                target_title TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                UNIQUE (source_document_id, target_title)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX document_links_source_idx ON document_links (source_document_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX document_links_target_idx ON document_links (target_document_id)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE attachments (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                document_id UUID REFERENCES documents(id),
                task_id UUID,
                file_name TEXT NOT NULL,
                content_type TEXT NOT NULL,
                size_bytes BIGINT NOT NULL,
                sha256 TEXT NOT NULL,
                created_by_user_id UUID REFERENCES users(id),
                created_by_api_key_id UUID REFERENCES api_keys(id),
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ,
                CONSTRAINT attachments_owner_check
                    CHECK (num_nonnulls(document_id, task_id) = 1),
                CONSTRAINT attachments_num_actors_check
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX attachments_sha256_idx ON attachments (workspace_id, sha256)"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE documents
               ADD CONSTRAINT documents_current_revision_fk
               FOREIGN KEY (current_revision_id) REFERENCES document_revisions(id)"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "ALTER TABLE documents DROP CONSTRAINT IF EXISTS documents_current_revision_fk",
        )
        .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS attachments CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS document_links CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS document_revisions CASCADE")
            .await?;
        conn.execute_unprepared("DROP TABLE IF EXISTS documents CASCADE")
            .await?;

        Ok(())
    }
}
