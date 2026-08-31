//! Orphaned-grant doctor finding (design §S3c, T6.8).
//!
//! Ships as a repo-level query only: no `doctor` module exists in
//! `atlas_server` yet (only `atlas_core::capabilities::diagnostics` and the
//! registry declaration), and S3 does not build one. This query is
//! registered under SHELL-OPS in E3 tracking instead.
//!
//! A grant is orphaned when its `resource_ref` names a project, folder,
//! document, or board that no longer exists. This composition-layer query is
//! the reason the check cannot live in `atlas_custos_postgres`: it joins
//! against Acta-owned tables, which that crate must never reference.
//! Workspace-scope grants are never counted — a workspace is soft-deleted,
//! never hard-deleted (the same invariant `dead_cascade_evidence.rs`
//! documents for D1's dropped `workspace_id` FKs).

use atlas_core::error::DomainError;
use atlas_postgres::db_err;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement};

pub async fn count_orphaned_grants(conn: &impl ConnectionTrait) -> Result<u64, DomainError> {
    #[derive(FromQueryResult)]
    struct Row {
        count: i64,
    }

    let row = Row::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT count(*) AS count
        FROM custos.permission_grants g
        WHERE (g.resource_ref LIKE 'acta::project::%'
                AND NOT EXISTS (
                    SELECT 1 FROM acta.projects p
                    WHERE p.id = substring(g.resource_ref FROM 16)::uuid
                ))
           OR (g.resource_ref LIKE 'acta::folder::%'
                AND NOT EXISTS (
                    SELECT 1 FROM acta.folders f
                    WHERE f.id = substring(g.resource_ref FROM 15)::uuid
                ))
           OR (g.resource_ref LIKE 'acta::document::%'
                AND NOT EXISTS (
                    SELECT 1 FROM acta.documents d
                    WHERE d.id = substring(g.resource_ref FROM 17)::uuid
                ))
           OR (g.resource_ref LIKE 'acta::board::%'
                AND NOT EXISTS (
                    SELECT 1 FROM boards b
                    WHERE b.id = substring(g.resource_ref FROM 14)::uuid
                ))
        "#,
        [],
    ))
    .one(conn)
    .await
    .map_err(db_err)?
    .ok_or_else(|| DomainError::Internal {
        message: "count_orphaned_grants query returned no row".into(),
    })?;

    u64::try_from(row.count).map_err(|_| DomainError::Internal {
        message: "count_orphaned_grants produced a negative count".into(),
    })
}
