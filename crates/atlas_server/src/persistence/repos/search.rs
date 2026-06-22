use async_trait::async_trait;
use atlas_domain::{
    DomainError, WorkspaceCtx,
    permissions::Principal,
    ports::search::{SearchAfter, SearchRepo, SortKey},
    search::{SearchFilter, SearchHit, SearchKind, SearchQuery, SearchSort},
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{DatabaseConnection, FromQueryResult};

pub struct PgSearchRepo {
    pub conn: DatabaseConnection,
}

impl PgSearchRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

// ---------------------------------------------------------------------------
// SQL constants
// ---------------------------------------------------------------------------

const HEADLINE_OPTS: &str =
    "MaxFragments=2, MaxWords=24, MinWords=8, StartSel=<mark>, StopSel=</mark>";

const HEADLINE_MAX_CHARS: usize = 50_000;

// ---------------------------------------------------------------------------
// Result row (both arms expose the same column list for UNION ALL)
// ---------------------------------------------------------------------------

#[derive(Debug, FromQueryResult)]
struct SearchRow {
    kind: String,
    id: uuid::Uuid,
    readable_id: Option<String>,
    project_slug: Option<String>,
    title: String,
    snippet: Option<String>,
    score: f32,
    updated_at: DateTime<Utc>,
    column_name: Option<String>,
}

// ---------------------------------------------------------------------------
// SearchRepo implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl SearchRepo for PgSearchRepo {
    async fn search(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        query: &SearchQuery,
        limit: u64,
        after: Option<SearchAfter>,
    ) -> Result<Vec<SearchHit>, DomainError> {
        use sea_orm::Statement;

        let mut values: Vec<sea_orm::Value> = Vec::new();

        // $1 — workspace_id
        values.push(ctx.workspace_id.0.into());

        // $2 — principal id
        //
        // The permission predicate mirrors the domain `resolve()` authority
        // (atlas_domain::permissions). For a User principal, three access paths
        // exist: implicit admin (Owner/Admin membership → sees all), explicit
        // grants (resource/ancestor/workspace-scope), and the member-only
        // visibility contribution (a plain member sees a resource whose effective
        // visibility is non-Private). An ApiKey is never a member and has no
        // visibility contribution, so it sees only grant-backed rows.
        let principal_col: &str;
        let owner_admin_clause: String;
        let member_clause: String;

        match principal {
            Principal::User(uid) => {
                principal_col = "user_id";
                values.push(uid.0.into());
                owner_admin_clause = "EXISTS (
                        SELECT 1 FROM workspace_memberships
                        WHERE workspace_id = $1
                          AND user_id = $2
                          AND role IN ('owner', 'admin')
                    )"
                .to_string();
                member_clause = "EXISTS (
                        SELECT 1 FROM workspace_memberships
                        WHERE workspace_id = $1
                          AND user_id = $2
                    )"
                .to_string();
            }
            Principal::ApiKey(kid) => {
                principal_col = "api_key_id";
                values.push(kid.0.into());
                owner_admin_clause = "FALSE".to_string();
                member_clause = "FALSE".to_string();
            }
        }

        let has_task_only_filter = query.filters.iter().any(|f| {
            matches!(
                f,
                SearchFilter::Status(_) | SearchFilter::Priority(_) | SearchFilter::Assignee(_)
            )
        });

        let emit_docs = query.type_filter.notes && !has_task_only_filter;
        let emit_tasks = query.type_filter.tasks;

        if !emit_docs && !emit_tasks {
            return Ok(vec![]);
        }

        let has_text = !query.text.trim().is_empty();

        let tsq_param: usize = if has_text {
            values.push(query.text.clone().into());
            values.len()
        } else {
            0
        };

        let project_filter_subquery = build_project_filter_subquery(&query.filters, &mut values);
        let updated_after_cond = build_updated_after_cond(&query.filters, &mut values);
        let updated_before_cond = build_updated_before_cond(&query.filters, &mut values);

        let tag_values = collect_filter_strings(&query.filters, |f| {
            if let SearchFilter::Tag(t) = f {
                Some(t)
            } else {
                None
            }
        });
        let status_values = collect_filter_strings(&query.filters, |f| {
            if let SearchFilter::Status(s) = f {
                Some(s)
            } else {
                None
            }
        });
        let priority_values = collect_filter_strings(&query.filters, |f| {
            if let SearchFilter::Priority(p) = f {
                Some(p)
            } else {
                None
            }
        });
        let assignee_values = collect_filter_strings(&query.filters, |f| {
            if let SearchFilter::Assignee(a) = f {
                Some(a)
            } else {
                None
            }
        });

        // Cursor predicate — uses outer alias columns (score or updated_at).
        let cursor_cond = if let Some(ref after) = after {
            match after.key {
                SortKey::Relevance(score) => {
                    values.push(score.into());
                    let ps = values.len();
                    values.push(after.id.into());
                    let pi = values.len();
                    format!("AND (score, id) < (${ps}, ${pi})")
                }
                SortKey::Updated(micros) => {
                    let ts = micros_to_datetime(micros)?;
                    values.push(ts.into());
                    let pts = values.len();
                    values.push(after.id.into());
                    let pi = values.len();
                    format!("AND (updated_at, id) < (${pts}, ${pi})")
                }
            }
        } else {
            String::new()
        };

        let order_clause = match query.sort {
            SearchSort::Relevance => "ORDER BY score DESC, id DESC",
            SearchSort::UpdatedDesc => "ORDER BY updated_at DESC, id DESC",
        };

        let arm_ctx = ArmCtx {
            owner_admin_clause: &owner_admin_clause,
            member_clause: &member_clause,
            principal_col,
            has_text,
            tsq_param,
            project_filter_subquery: &project_filter_subquery,
            updated_after_cond: &updated_after_cond,
            updated_before_cond: &updated_before_cond,
        };

        let mut arm_sqls: Vec<String> = Vec::new();

        if emit_docs {
            arm_sqls.push(build_doc_arm(&mut values, &arm_ctx, &tag_values));
        }

        if emit_tasks {
            arm_sqls.push(build_task_arm(
                &mut values,
                &arm_ctx,
                &tag_values,
                &status_values,
                &priority_values,
                &assignee_values,
            ));
        }

        let union_sql = arm_sqls.join("\nUNION ALL\n");

        values.push((limit as i64).into());
        let limit_param = values.len();

        // Two-stage headline (design Q4 / REQ-7): the inner query resolves the
        // page (FTS predicate, permission filter, cursor, ORDER BY, LIMIT) carrying
        // only the bounded snippet source text; ts_headline runs in the outer SELECT
        // over the <= N+1 surviving rows, never over every matching row. ts_rank_cd
        // stays inside the inner query because it drives ORDER BY.
        let snippet_expr = if has_text {
            format!(
                "ts_headline('simple', page.snippet_source, websearch_to_tsquery('simple', ${tsq_param}), '{HEADLINE_OPTS}')"
            )
        } else {
            "NULL::text".to_string()
        };

        let sql = format!(
            r#"
            SELECT
                page.kind,
                page.id,
                page.readable_id,
                page.project_slug,
                page.title,
                {snippet_expr} AS snippet,
                page.score,
                page.updated_at,
                page.column_name
            FROM (
                SELECT kind, id, readable_id, project_slug, title, snippet_source, score, updated_at, column_name
                FROM ({union_sql}) combined
                WHERE 1=1
                {cursor_cond}
                {order_clause}
                LIMIT ${limit_param}
            ) page
            {order_clause}
            "#,
        );

        let rows = SearchRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        rows.into_iter().map(row_to_hit).collect()
    }
}

// ---------------------------------------------------------------------------
// Shared arm context (groups params shared by both arms to stay under the
// clippy::too_many_arguments limit)
// ---------------------------------------------------------------------------

struct ArmCtx<'a> {
    owner_admin_clause: &'a str,
    member_clause: &'a str,
    principal_col: &'a str,
    has_text: bool,
    tsq_param: usize,
    project_filter_subquery: &'a str,
    updated_after_cond: &'a str,
    updated_before_cond: &'a str,
}

// ---------------------------------------------------------------------------
// Documents arm
// ---------------------------------------------------------------------------

fn build_doc_arm(
    values: &mut Vec<sea_orm::Value>,
    ctx: &ArmCtx<'_>,
    tag_values: &[String],
) -> String {
    // The tsquery expression is inlined rather than aliased to avoid a
    // per-row subquery; Postgres evaluates it once for the plan node.
    let tsq_expr = format!("websearch_to_tsquery('simple', ${})", ctx.tsq_param);

    let fts_where = if ctx.has_text {
        format!("AND d.search_vector @@ {tsq_expr}")
    } else {
        String::new()
    };

    let score_expr = if ctx.has_text {
        format!("ts_rank_cd(d.search_vector, {tsq_expr})")
    } else {
        "0::real".to_string()
    };

    // Carry the bounded source text instead of the headline. The headline is
    // computed once per surviving page row in the outer query (after LIMIT), so
    // ts_headline does not run for every matching row.
    let snippet_source_expr = if ctx.has_text {
        format!("left(d.content, {HEADLINE_MAX_CHARS})")
    } else {
        "NULL::text".to_string()
    };

    let doc_tag_cond = build_doc_tag_cond(values, tag_values);

    // Permission pushdown: embedded in WHERE so LIMIT applies after permission filtering.
    let perm = build_doc_permission(ctx.owner_admin_clause, ctx.member_clause, ctx.principal_col);

    let project_filter_subquery = ctx.project_filter_subquery;
    let updated_after_cond = ctx.updated_after_cond;
    let updated_before_cond = ctx.updated_before_cond;

    format!(
        r#"SELECT
            'document'::text AS kind,
            d.id,
            NULL::text AS readable_id,
            p.slug AS project_slug,
            d.title,
            {snippet_source_expr} AS snippet_source,
            {score_expr} AS score,
            d.updated_at,
            NULL::text AS column_name
        FROM documents d
        LEFT JOIN projects p ON p.id = d.project_id AND p.workspace_id = $1 AND p.deleted_at IS NULL
        WHERE d.workspace_id = $1
          AND d.deleted_at IS NULL
          {fts_where}
          AND ({perm})
          {project_filter_subquery}
          {doc_tag_cond}
          {updated_after_cond}
          {updated_before_cond}"#,
    )
}

/// Permission disjunction for documents, mirroring the domain `resolve()` authority.
///
/// A row is returned to the principal IFF `resolve()` would grant it >= Viewer:
/// 1. `owner_admin_clause` — implicit admin (Owner/Admin members see all rows).
/// 2. Explicit grants — workspace-scope, per-document, per-project, or folder
///    ancestry (the same grant branches verified correct previously).
/// 3. Visibility contribution — a plain member sees a row whose effective
///    visibility is non-Private. A document's visibility derives from its
///    project; a document with no project has no visibility contribution (the
///    LEFT JOIN yields NULL `p.visibility`), exactly as `build_document_chain`
///    produces no Project segment in that case. ApiKey principals are never
///    members (`member_clause` = FALSE), so they get no visibility contribution.
///
/// Permission pushdown: the disjunction is embedded in the WHERE clause so that
/// LIMIT applies only to rows the principal can actually see. Fetching then
/// filtering in Rust would require unbounded row reads and could leak row counts.
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

fn build_doc_tag_cond(values: &mut Vec<sea_orm::Value>, tag_values: &[String]) -> String {
    if tag_values.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for tag in tag_values {
        let json_fragment = serde_json::json!({"tags": [tag]}).to_string();
        values.push(json_fragment.into());
        parts.push(format!("d.frontmatter @> ${}::jsonb", values.len()));
    }
    format!("AND ({})", parts.join(" OR "))
}

// ---------------------------------------------------------------------------
// Tasks arm
// ---------------------------------------------------------------------------

fn build_task_arm(
    values: &mut Vec<sea_orm::Value>,
    ctx: &ArmCtx<'_>,
    tag_values: &[String],
    status_values: &[String],
    priority_values: &[String],
    assignee_values: &[String],
) -> String {
    let tsq_expr = format!("websearch_to_tsquery('simple', ${})", ctx.tsq_param);

    let fts_where = if ctx.has_text {
        format!("AND t.search_vector @@ {tsq_expr}")
    } else {
        String::new()
    };

    let score_expr = if ctx.has_text {
        format!("ts_rank_cd(t.search_vector, {tsq_expr})")
    } else {
        "0::real".to_string()
    };

    // See build_doc_arm: carry the bounded source text; the headline is computed
    // once per surviving page row in the outer query (after LIMIT).
    let snippet_source_expr = if ctx.has_text {
        format!("left(t.description, {HEADLINE_MAX_CHARS})")
    } else {
        "NULL::text".to_string()
    };

    let task_tag_cond = build_task_tag_cond(values, tag_values);
    let status_cond = build_status_cond(values, status_values);
    let priority_cond = build_priority_cond(values, priority_values);
    let assignee_cond = build_assignee_cond(values, assignee_values);

    let perm = build_task_permission(ctx.owner_admin_clause, ctx.member_clause, ctx.principal_col);

    let project_filter_subquery = ctx.project_filter_subquery;
    let updated_after_cond = ctx.updated_after_cond;
    let updated_before_cond = ctx.updated_before_cond;

    format!(
        r#"SELECT
            'task'::text AS kind,
            t.id,
            t.readable_id AS readable_id,
            p.slug AS project_slug,
            t.title,
            {snippet_source_expr} AS snippet_source,
            {score_expr} AS score,
            t.updated_at,
            bc.name AS column_name
        FROM tasks t
        LEFT JOIN projects p ON p.id = t.project_id AND p.workspace_id = $1 AND p.deleted_at IS NULL
        LEFT JOIN board_columns bc ON bc.id = t.column_id AND bc.workspace_id = $1 AND bc.deleted_at IS NULL
        WHERE t.workspace_id = $1
          AND t.deleted_at IS NULL
          {fts_where}
          AND ({perm})
          {project_filter_subquery}
          {task_tag_cond}
          {status_cond}
          {priority_cond}
          {assignee_cond}
          {updated_after_cond}
          {updated_before_cond}"#,
    )
}

/// Permission disjunction for tasks, mirroring the domain `resolve()` authority.
///
/// A task's permission chain is Board -> Project -> Workspace (no folder, no own
/// visibility). A row is returned to the principal IFF `resolve()` would grant it
/// >= Viewer:
/// 1. `owner_admin_clause` — implicit admin (Owner/Admin members see all rows).
/// 2. Explicit grants — workspace-scope, per-project, or per-board.
/// 3. Visibility contribution — a plain member sees a task whose project is
///    non-Private. A soft-deleted project yields NULL `p.visibility` via the
///    LEFT JOIN, removing the contribution exactly as a soft-deleted project
///    drops out of the resolved chain (`PgProjectRepo::find` filters deleted
///    rows). ApiKey principals get no visibility contribution.
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

fn build_task_tag_cond(values: &mut Vec<sea_orm::Value>, tag_values: &[String]) -> String {
    if tag_values.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for tag in tag_values {
        values.push(tag.clone().into());
        parts.push(format!("${} = ANY(t.labels)", values.len()));
    }
    format!("AND ({})", parts.join(" OR "))
}

fn build_status_cond(values: &mut Vec<sea_orm::Value>, status_values: &[String]) -> String {
    if status_values.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for status in status_values {
        values.push(status.clone().into());
        let pn = values.len();
        parts.push(format!(
            "t.column_id IN (
                SELECT bc.id FROM board_columns bc
                WHERE bc.workspace_id = $1
                  AND bc.board_id = t.board_id
                  AND bc.deleted_at IS NULL
                  AND lower(bc.name) = lower(${pn})
            )"
        ));
    }
    format!("AND ({})", parts.join(" OR "))
}

fn build_priority_cond(values: &mut Vec<sea_orm::Value>, priority_values: &[String]) -> String {
    if priority_values.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for priority in priority_values {
        values.push(priority.clone().into());
        parts.push(format!("lower(t.priority) = lower(${})", values.len()));
    }
    format!("AND ({})", parts.join(" OR "))
}

fn build_assignee_cond(values: &mut Vec<sea_orm::Value>, assignee_values: &[String]) -> String {
    if assignee_values.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for assignee in assignee_values {
        values.push(assignee.clone().into());
        let pn = values.len();
        parts.push(format!(
            "EXISTS (
                SELECT 1 FROM task_assignees ta
                JOIN users u ON u.id = ta.assignee_user_id
                WHERE ta.task_id = t.id
                  AND lower(u.username) = lower(${pn})
            )"
        ));
    }
    format!("AND ({})", parts.join(" OR "))
}

// ---------------------------------------------------------------------------
// Shared filter helpers
// ---------------------------------------------------------------------------

fn build_project_filter_subquery(
    filters: &[SearchFilter],
    values: &mut Vec<sea_orm::Value>,
) -> String {
    let slugs: Vec<&str> = filters
        .iter()
        .filter_map(|f| {
            if let SearchFilter::Project(s) = f {
                Some(s.as_str())
            } else {
                None
            }
        })
        .collect();

    if slugs.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    for slug in slugs {
        values.push(slug.to_string().into());
        let pn = values.len();
        parts.push(format!(
            "SELECT id FROM projects
             WHERE workspace_id = $1
               AND lower(slug) = lower(${pn})
               AND deleted_at IS NULL"
        ));
    }
    let sub = parts.join(" UNION ");
    format!("AND project_id IN ({sub})")
}

fn build_updated_after_cond(filters: &[SearchFilter], values: &mut Vec<sea_orm::Value>) -> String {
    let mut cond = String::new();
    for f in filters {
        if let SearchFilter::UpdatedAfter(date) = f {
            let ts: DateTime<Utc> = date.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
            values.push(ts.into());
            cond = format!("AND updated_at > ${}", values.len());
        }
    }
    cond
}

fn build_updated_before_cond(filters: &[SearchFilter], values: &mut Vec<sea_orm::Value>) -> String {
    let mut cond = String::new();
    for f in filters {
        if let SearchFilter::UpdatedBefore(date) = f {
            let ts: DateTime<Utc> = date.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
            values.push(ts.into());
            cond = format!("AND updated_at < ${}", values.len());
        }
    }
    cond
}

fn collect_filter_strings<'a, F>(filters: &'a [SearchFilter], extract: F) -> Vec<String>
where
    F: Fn(&'a SearchFilter) -> Option<&'a String>,
{
    filters.iter().filter_map(extract).cloned().collect()
}

// ---------------------------------------------------------------------------
// Row mapping and utilities
// ---------------------------------------------------------------------------

fn row_to_hit(row: SearchRow) -> Result<SearchHit, DomainError> {
    let kind = match row.kind.as_str() {
        "document" => SearchKind::Document,
        "task" => SearchKind::Task,
        other => {
            return Err(DomainError::Internal {
                message: format!("unknown search result kind in DB: {other}"),
            });
        }
    };

    // A genuine body match always produces a ts_headline result containing
    // <mark>...</mark> (StartSel/StopSel markers). A title-only match yields
    // a leading body fragment with no markers — exactly the case REQ-7 says
    // must produce an absent snippet rather than an unhighlighted fragment.
    let snippet = row.snippet.filter(|s| s.contains("<mark>"));

    Ok(SearchHit {
        kind,
        id: row.id,
        readable_id: row.readable_id,
        title: row.title,
        snippet,
        score: row.score,
        updated_at: row.updated_at,
        project_slug: row.project_slug,
        column_name: row.column_name,
    })
}

fn micros_to_datetime(micros: i64) -> Result<DateTime<Utc>, DomainError> {
    Utc.timestamp_micros(micros)
        .single()
        .ok_or_else(|| DomainError::Internal {
            message: format!("cursor timestamp {micros} is out of range"),
        })
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
