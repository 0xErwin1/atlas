//! Postgres adapter for the reverse-grant discovery port (E11-S8 D-S8-1,
//! design D3).
//!
//! Group membership is resolved inside this adapter's SQL, not passed in the
//! port input (design D3, deviating from the spec/proposal's "resolved group
//! ids" seam): no cross-workspace group-membership query exists elsewhere in
//! the codebase, and adding one would make `atlas_server` issue a second raw
//! query over Custos-owned tables. A soft-deleted group's grants are
//! excluded by the join's `g.deleted_at IS NULL` filter — the *only* filter
//! here, not a defense-in-depth layer on top of another check (unlike
//! `load_grants_for_resolution`'s membership-filtered equivalent).
//!
//! Deliberately queries across every workspace: this is the reverse
//! direction of `load_grants_for_resolution` (workspace-forward), which is
//! exactly why `m20260906_000052_grant_principal_idx` exists — every
//! pre-existing index on `permission_grants` is workspace-leading and cannot
//! serve a bare `WHERE user_id = $1`.
//!
//! **`UNION`, not `OR` (post-verify fix)**: the first cut of this query used
//! a single `WHERE user_id = $1 OR group_id IN (SELECT ...)` predicate. An
//! `OR` across two different columns, where the right-hand side is an
//! uncorrelated subquery, is a well-known planner trap — Postgres cannot
//! generally satisfy that shape with a single index, and a bitmap-OR plan
//! across two *different* partial indexes is not guaranteed. Splitting into
//! a `UNION` of two independently-indexable branches lets the planner pick
//! `permission_grants_user_idx` for the direct branch and
//! `permission_grants_group_idx` for the group branch, each via a plain
//! index (or bitmap index) scan; `UNION` (not `UNION ALL`) keeps the
//! `DISTINCT` semantics the single-query version had, since a principal can
//! reach the same `resource_ref` both directly and through a group.
//!
//! `build_query` is a standalone `pub` function rather than folded straight
//! into `granted_scopes` so the container-backed `EXPLAIN` guard test in
//! `tests/discovery_query_plan.rs` can run the *exact* statement this
//! adapter issues, not a hand-copied approximation that could drift.

use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::ids::ResourceRef;
use atlas_custos::ports::discovery::MAX_DISCOVERY_SCOPES;
use atlas_custos::ports::discovery::{DiscoveryPort, DiscoveryPrincipal, GrantedScopes};
use sea_orm::{DatabaseConnection, FromQueryResult, Statement};
use uuid::Uuid;

use atlas_postgres::db_err;

#[derive(Debug, FromQueryResult)]
struct ResourceRefRow {
    resource_ref: String,
}

/// The direct-grant branch, matching `permission_grants_user_idx` /
/// `permission_grants_api_key_idx` exactly (a bare equality on the indexed
/// column, no `OR`).
const DIRECT_USER_SQL: &str =
    "SELECT resource_ref FROM custos.permission_grants WHERE user_id = $1";
const DIRECT_API_KEY_SQL: &str =
    "SELECT resource_ref FROM custos.permission_grants WHERE api_key_id = $1";

/// The group-derived branch: joins from `custos.group_members` (filtered by
/// `gm.user_id = $1`) through to `custos.permission_grants` on `group_id`,
/// so the join's inner side can use `permission_grants_group_idx`. Excludes
/// grants reached only through a soft-deleted group.
const GROUP_SQL: &str = "SELECT pg.resource_ref \
     FROM custos.permission_grants pg \
     JOIN custos.group_members gm ON gm.group_id = pg.group_id \
     JOIN custos.groups g ON g.id = gm.group_id \
     WHERE gm.user_id = $1 AND g.deleted_at IS NULL";

/// Wraps `inner` in an outer, ordered, capped `SELECT` (review CRITICAL,
/// resilience), so the driver never receives more than
/// `MAX_DISCOVERY_SCOPES + 1` rows.
fn capped(inner: &str) -> String {
    format!(
        "SELECT resource_ref FROM ({inner}) AS capped ORDER BY resource_ref LIMIT {}",
        MAX_DISCOVERY_SCOPES + 1
    )
}

/// Builds the exact SQL and its single positional parameter for a given
/// principal, or `None` when the principal carries neither id (nothing to
/// query). Exposed so tests can run `EXPLAIN` on the identical statement
/// `granted_scopes` executes.
pub fn build_query(principal: &DiscoveryPrincipal) -> Option<(String, Uuid)> {
    match (principal.user_id, principal.api_key_id) {
        (Some(user_id), _) => Some((
            capped(&format!("{DIRECT_USER_SQL} UNION {GROUP_SQL}")),
            user_id.0,
        )),
        (None, Some(api_key_id)) => Some((capped(DIRECT_API_KEY_SQL), api_key_id.0)),
        (None, None) => None,
    }
}

pub struct PgDiscoveryRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl DiscoveryPort for PgDiscoveryRepo {
    async fn granted_scopes(
        &self,
        principal: &DiscoveryPrincipal,
    ) -> Result<GrantedScopes, DomainError> {
        let Some((sql, param)) = build_query(principal) else {
            return Ok(GrantedScopes::new());
        };

        let rows = ResourceRefRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            [param.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        let mut scopes = GrantedScopes::new();
        for row in rows {
            let resource_ref: ResourceRef =
                row.resource_ref
                    .parse()
                    .map_err(|_| DomainError::Internal {
                        message: format!(
                            "permission grant row has an invalid resource_ref: {}",
                            row.resource_ref
                        ),
                    })?;
            scopes.insert(resource_ref);
        }

        Ok(scopes)
    }
}
