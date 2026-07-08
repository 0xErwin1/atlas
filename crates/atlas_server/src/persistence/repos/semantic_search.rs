use async_trait::async_trait;
use atlas_domain::{
    DomainError,
    permissions::Principal,
    semantic_search::{
        EmbeddingInput, EmbeddingProvider, ResourceKind, SemanticIndexChunk, SemanticSearchHit,
        SemanticSearchQuery, SemanticSearchRepo, SemanticSearchSource,
    },
};
use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement};
use std::sync::Arc;
use uuid::Uuid;

pub struct PgSemanticIndexWriter {
    conn: DatabaseConnection,
    provider: Arc<dyn EmbeddingProvider>,
}

impl PgSemanticIndexWriter {
    pub fn new(conn: DatabaseConnection, provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self { conn, provider }
    }

    pub async fn index_chunks(&self, chunks: &[SemanticIndexChunk]) -> Result<(), DomainError> {
        let mut pending = Vec::new();
        for chunk in chunks {
            if !self.is_unchanged(chunk).await? {
                pending.push(chunk.clone());
            }
        }
        if pending.is_empty() {
            return Ok(());
        }

        let inputs: Vec<EmbeddingInput> = pending
            .iter()
            .map(|chunk| EmbeddingInput {
                text: chunk.text.clone(),
            })
            .collect();
        let embeddings = self.provider.embed(&inputs).await?;
        if embeddings.len() != pending.len() {
            return Err(DomainError::Internal {
                message: format!(
                    "embedding provider returned {} vectors for {} chunks",
                    embeddings.len(),
                    pending.len()
                ),
            });
        }

        for (chunk, embedding) in pending.iter().zip(embeddings.iter()) {
            if embedding.len() != self.provider.dimensions() {
                return Err(DomainError::Internal {
                    message: format!(
                        "embedding dimension mismatch: expected {}, got {}",
                        self.provider.dimensions(),
                        embedding.len()
                    ),
                });
            }
            self.upsert_chunk(chunk, embedding).await?;
        }
        Ok(())
    }

    pub async fn mark_resource_stale(
        &self,
        workspace_id: atlas_domain::ids::WorkspaceId,
        kind: ResourceKind,
        resource_id: Uuid,
    ) -> Result<(), DomainError> {
        self.conn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                r#"UPDATE search_embeddings
                   SET stale_at = now(), updated_at = now()
                   WHERE workspace_id = $1
                     AND resource_kind = $2
                     AND resource_id = $3
                     AND model = $4
                     AND dimensions = $5
                     AND stale_at IS NULL"#,
                vec![
                    workspace_id.0.into(),
                    kind.db_str().into(),
                    resource_id.into(),
                    self.provider.model().to_owned().into(),
                    (self.provider.dimensions() as i32).into(),
                ],
            ))
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn is_unchanged(&self, chunk: &SemanticIndexChunk) -> Result<bool, DomainError> {
        #[derive(Debug, FromQueryResult)]
        struct HashRow {
            content_hash: String,
        }

        let row = HashRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            r#"SELECT content_hash
               FROM search_embeddings
               WHERE workspace_id = $1
                 AND resource_kind = $2
                 AND resource_id = $3
                 AND source_field = $4
                 AND chunk_ordinal = $5
                 AND model = $6
                 AND dimensions = $7
                 AND stale_at IS NULL"#,
            vec![
                chunk.workspace_id.0.into(),
                chunk.kind.db_str().into(),
                chunk.resource_id.into(),
                chunk.source.db_str().into(),
                chunk.chunk_ordinal.into(),
                self.provider.model().to_owned().into(),
                (self.provider.dimensions() as i32).into(),
            ],
        ))
        .one(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(row.is_some_and(|row| row.content_hash == chunk.content_hash))
    }

    async fn upsert_chunk(
        &self,
        chunk: &SemanticIndexChunk,
        embedding: &[f32],
    ) -> Result<(), DomainError> {
        self.conn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                r#"INSERT INTO search_embeddings (
                       id, workspace_id, resource_kind, resource_id, source_field, chunk_ordinal,
                       content_hash, model, dimensions, embedding, excerpt, token_count,
                       indexed_at, stale_at, created_at, updated_at
                   ) VALUES (
                       gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9::vector, $10, $11,
                       now(), NULL, now(), now()
                   )
                   ON CONFLICT (workspace_id, resource_kind, resource_id, source_field, chunk_ordinal, model, dimensions)
                   DO UPDATE SET
                       content_hash = EXCLUDED.content_hash,
                       embedding = EXCLUDED.embedding,
                       excerpt = EXCLUDED.excerpt,
                       token_count = EXCLUDED.token_count,
                       indexed_at = now(),
                       stale_at = NULL,
                       updated_at = now()"#,
                vec![
                    chunk.workspace_id.0.into(),
                    chunk.kind.db_str().into(),
                    chunk.resource_id.into(),
                    chunk.source.db_str().into(),
                    chunk.chunk_ordinal.into(),
                    chunk.content_hash.clone().into(),
                    self.provider.model().to_owned().into(),
                    (self.provider.dimensions() as i32).into(),
                    vector_literal(embedding).into(),
                    chunk.excerpt.clone().into(),
                    token_count(&chunk.text).into(),
                ],
            ))
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

pub struct PgSemanticSearchRepo {
    conn: DatabaseConnection,
    provider: Arc<dyn EmbeddingProvider>,
}

impl PgSemanticSearchRepo {
    pub fn new(conn: DatabaseConnection, provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self { conn, provider }
    }
}

#[derive(Debug, FromQueryResult)]
struct SemanticSearchRow {
    kind: String,
    id: Uuid,
    readable_id: Option<String>,
    title: String,
    project_slug: Option<String>,
    column_name: Option<String>,
    similarity: f32,
    source: String,
    excerpt: String,
}

#[async_trait]
impl SemanticSearchRepo for PgSemanticSearchRepo {
    async fn search(
        &self,
        query: &SemanticSearchQuery,
    ) -> Result<Vec<SemanticSearchHit>, DomainError> {
        if query.text.trim().is_empty() || query.limit == 0 {
            return Ok(Vec::new());
        }
        let emit_docs = query.type_filter.documents && query.may_read_documents;
        let emit_tasks = query.type_filter.tasks && query.may_read_tasks;
        if !emit_docs && !emit_tasks {
            return Ok(Vec::new());
        }

        let vectors = self
            .provider
            .embed(&[EmbeddingInput {
                text: query.text.clone(),
            }])
            .await?;
        let query_vector = vectors.first().ok_or_else(|| DomainError::Internal {
            message: "embedding provider returned no query vector".to_owned(),
        })?;
        if query_vector.len() != self.provider.dimensions() {
            return Err(DomainError::Internal {
                message: format!(
                    "embedding dimension mismatch: expected {}, got {}",
                    self.provider.dimensions(),
                    query_vector.len()
                ),
            });
        }

        let (principal_col, principal_id, owner_admin_clause, member_clause) =
            principal_sql(&query.principal, query.bypass);
        let doc_perm = build_doc_permission(&owner_admin_clause, &member_clause, principal_col);
        let task_perm = build_task_permission(&owner_admin_clause, &member_clause, principal_col);
        let kind_cond = match (emit_docs, emit_tasks) {
            (true, true) => String::new(),
            (true, false) => "AND ranked.resource_kind = 'document'".to_owned(),
            (false, true) => "AND ranked.resource_kind = 'task'".to_owned(),
            (false, false) => unreachable!(),
        };
        let cursor_cond = if let Some(after) = query.after {
            let kind_rank = after.resource_kind.sort_rank();
            format!(
                "AND (ranked.similarity < {similarity} OR \
                 (ranked.similarity = {similarity} AND ranked.kind_rank > {kind_rank}) OR \
                 (ranked.similarity = {similarity} AND ranked.kind_rank = {kind_rank} \
                  AND ranked.resource_id < '{resource_id}'))",
                similarity = after.similarity,
                resource_id = after.resource_id
            )
        } else {
            String::new()
        };

        let limit = query.limit as i64;
        let sql = format!(
            r#"WITH nearest AS (
                   SELECT se.resource_kind,
                          se.resource_id,
                          se.source_field,
                          se.excerpt,
                          1 - (se.embedding <=> $3::vector) AS similarity,
                          CASE se.resource_kind WHEN 'document' THEN 0 ELSE 1 END AS kind_rank,
                          se.chunk_ordinal
                   FROM search_embeddings se
                   WHERE se.workspace_id = $1
                     AND se.model = $4
                     AND se.dimensions = $5
                     AND se.stale_at IS NULL
               ), ranked AS (
                   SELECT *, row_number() OVER (
                       PARTITION BY resource_kind, resource_id
                       ORDER BY similarity DESC, chunk_ordinal ASC
                   ) AS resource_rank
                   FROM nearest
               )
               SELECT ranked.resource_kind AS kind,
                      ranked.resource_id AS id,
                      NULL::text AS readable_id,
                      d.title,
                      p.slug AS project_slug,
                      NULL::text AS column_name,
                      ranked.similarity::real AS similarity,
                      ranked.source_field AS source,
                      ranked.excerpt
               FROM ranked
               JOIN documents d ON d.id = ranked.resource_id
                   AND d.workspace_id = $1
                   AND d.deleted_at IS NULL
                   AND ranked.resource_kind = 'document'
               LEFT JOIN projects p ON p.id = d.project_id AND p.workspace_id = $1 AND p.deleted_at IS NULL
               WHERE ranked.resource_rank = 1
                 {kind_cond}
                 {cursor_cond}
                 AND ({doc_perm})
               UNION ALL
               SELECT ranked.resource_kind AS kind,
                      ranked.resource_id AS id,
                      t.readable_id,
                      t.title,
                      p.slug AS project_slug,
                      bc.name AS column_name,
                      ranked.similarity::real AS similarity,
                      ranked.source_field AS source,
                      ranked.excerpt
               FROM ranked
               JOIN tasks t ON t.id = ranked.resource_id
                   AND t.workspace_id = $1
                   AND t.deleted_at IS NULL
                   AND ranked.resource_kind = 'task'
               LEFT JOIN projects p ON p.id = t.project_id AND p.workspace_id = $1 AND p.deleted_at IS NULL
               LEFT JOIN board_columns bc ON bc.id = t.column_id AND bc.workspace_id = $1 AND bc.deleted_at IS NULL
               WHERE ranked.resource_rank = 1
                 {kind_cond}
                 {cursor_cond}
                 AND ({task_perm})
               ORDER BY similarity DESC, kind ASC, id DESC
               LIMIT $6"#,
        );

        let rows = SemanticSearchRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            vec![
                query.workspace_id.0.into(),
                principal_id.into(),
                vector_literal(query_vector).into(),
                self.provider.model().to_owned().into(),
                (self.provider.dimensions() as i32).into(),
                limit.into(),
            ],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        rows.into_iter().map(row_to_hit).collect()
    }
}

trait ResourceKindSql {
    fn db_str(self) -> &'static str;
    fn sort_rank(self) -> i32;
}

impl ResourceKindSql for ResourceKind {
    fn db_str(self) -> &'static str {
        match self {
            ResourceKind::Document => "document",
            ResourceKind::Task => "task",
        }
    }

    fn sort_rank(self) -> i32 {
        match self {
            ResourceKind::Document => 0,
            ResourceKind::Task => 1,
        }
    }
}

trait SemanticSearchSourceSql {
    fn db_str(self) -> &'static str;
}

impl SemanticSearchSourceSql for SemanticSearchSource {
    fn db_str(self) -> &'static str {
        match self {
            SemanticSearchSource::Title => "title",
            SemanticSearchSource::Content => "content",
            SemanticSearchSource::Comment => "comment",
            SemanticSearchSource::AttachmentName => "attachment_name",
            SemanticSearchSource::Checklist => "checklist",
            SemanticSearchSource::Subtask => "subtask",
            SemanticSearchSource::Aggregate => "aggregate",
        }
    }
}

fn row_to_hit(row: SemanticSearchRow) -> Result<SemanticSearchHit, DomainError> {
    Ok(SemanticSearchHit {
        kind: parse_kind(&row.kind)?,
        id: row.id,
        readable_id: row.readable_id,
        title: row.title,
        project_slug: row.project_slug,
        column_name: row.column_name,
        similarity: row.similarity,
        source: parse_source(&row.source)?,
        excerpt: row.excerpt,
    })
}

fn parse_kind(value: &str) -> Result<ResourceKind, DomainError> {
    match value {
        "document" => Ok(ResourceKind::Document),
        "task" => Ok(ResourceKind::Task),
        other => Err(DomainError::Internal {
            message: format!("unknown semantic resource kind in DB: {other}"),
        }),
    }
}

fn parse_source(value: &str) -> Result<SemanticSearchSource, DomainError> {
    match value {
        "title" => Ok(SemanticSearchSource::Title),
        "content" => Ok(SemanticSearchSource::Content),
        "comment" => Ok(SemanticSearchSource::Comment),
        "attachment_name" => Ok(SemanticSearchSource::AttachmentName),
        "checklist" => Ok(SemanticSearchSource::Checklist),
        "subtask" => Ok(SemanticSearchSource::Subtask),
        "aggregate" => Ok(SemanticSearchSource::Aggregate),
        other => Err(DomainError::Internal {
            message: format!("unknown semantic source in DB: {other}"),
        }),
    }
}

fn principal_sql(principal: &Principal, bypass: bool) -> (&'static str, Uuid, String, String) {
    match principal {
        Principal::User(user_id) => {
            let owner_admin_clause = if bypass {
                "TRUE".to_owned()
            } else {
                "EXISTS (SELECT 1 FROM workspace_memberships WHERE workspace_id = $1 AND user_id = $2 AND role IN ('owner', 'admin'))".to_owned()
            };
            let member_clause =
                "EXISTS (SELECT 1 FROM workspace_memberships WHERE workspace_id = $1 AND user_id = $2)"
                    .to_owned();
            ("user_id", user_id.0, owner_admin_clause, member_clause)
        }
        Principal::ApiKey(api_key_id) => (
            "api_key_id",
            api_key_id.0,
            "FALSE".to_owned(),
            "FALSE".to_owned(),
        ),
        Principal::Group(_) => (
            "user_id",
            Uuid::nil(),
            "FALSE".to_owned(),
            "FALSE".to_owned(),
        ),
    }
}

fn build_doc_permission(
    owner_admin_clause: &str,
    member_clause: &str,
    principal_col: &str,
) -> String {
    format!(
        r#"
        {owner_admin_clause}
        OR ({member_clause} AND p.id IS NOT NULL AND p.visibility <> 'private')
        OR EXISTS (
            SELECT 1 FROM permission_grants
            WHERE workspace_id = $1
              AND {principal_col} = $2
              AND project_id IS NULL
              AND folder_id IS NULL
              AND document_id IS NULL
              AND board_id IS NULL
        )
        OR EXISTS (
            SELECT 1 FROM permission_grants
            WHERE workspace_id = $1
              AND {principal_col} = $2
              AND document_id = d.id
        )
        OR EXISTS (
            SELECT 1 FROM permission_grants
            WHERE workspace_id = $1
              AND {principal_col} = $2
              AND project_id IS NOT NULL
              AND project_id = d.project_id
        )
        OR EXISTS (
            WITH RECURSIVE ancestors AS (
                SELECT f.id, f.parent_folder_id, f.project_id,
                       ARRAY[f.id] AS path, 1 AS depth
                FROM folders f
                WHERE f.id = d.folder_id
                  AND f.workspace_id = $1
                UNION ALL
                SELECT pf.id, pf.parent_folder_id, pf.project_id,
                       a.path || pf.id, a.depth + 1
                FROM folders pf
                JOIN ancestors a ON pf.id = a.parent_folder_id
                WHERE pf.workspace_id = $1
                  AND NOT pf.id = ANY(a.path)
                  AND a.depth < 32
            )
            SELECT 1 FROM permission_grants pg
            WHERE pg.workspace_id = $1
              AND pg.{principal_col} = $2
              AND (
                    pg.folder_id IN (SELECT id FROM ancestors)
                    OR (
                        pg.project_id IS NOT NULL
                        AND pg.project_id IN (
                            SELECT project_id FROM ancestors
                            WHERE project_id IS NOT NULL
                        )
                    )
              )
        )
        "#
    )
}

fn build_task_permission(
    owner_admin_clause: &str,
    member_clause: &str,
    principal_col: &str,
) -> String {
    format!(
        r#"
        {owner_admin_clause}
        OR ({member_clause} AND p.id IS NOT NULL AND p.visibility <> 'private')
        OR EXISTS (
            SELECT 1 FROM permission_grants
            WHERE workspace_id = $1
              AND {principal_col} = $2
              AND project_id IS NULL
              AND folder_id IS NULL
              AND document_id IS NULL
              AND board_id IS NULL
        )
        OR EXISTS (
            SELECT 1 FROM permission_grants
            WHERE workspace_id = $1
              AND {principal_col} = $2
              AND project_id IS NOT NULL
              AND project_id = t.project_id
        )
        OR EXISTS (
            SELECT 1 FROM permission_grants
            WHERE workspace_id = $1
              AND {principal_col} = $2
              AND board_id IS NOT NULL
              AND board_id = t.board_id
        )
        "#
    )
}

fn vector_literal(values: &[f32]) -> String {
    let body = values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

fn token_count(text: &str) -> i32 {
    text.split_whitespace().count() as i32
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
