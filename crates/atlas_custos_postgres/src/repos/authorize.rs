//! Read-only Postgres adapters behind the authorization service: existence
//! of Custos resources per kind, the actor's live group memberships from the
//! V1 group tables, and the stored-facts load of V2 grants and deny rules.

use std::collections::HashSet;

use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_custos::entities::authorization::GrantAuthority;
use atlas_custos::ids::{GroupId, PrincipalId};
use atlas_custos::ports::authorize::{
    AuthorizationFactsStore, GroupMembershipSource, ProductScope, StoredAuthorizationFacts,
};
use atlas_custos::provider::{CustosKind, CustosResourceStore};
use atlas_postgres::db_err;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait,
    FromQueryResult, QueryFilter, QueryOrder, QuerySelect, RelationTrait, Statement,
};
use uuid::Uuid;

use crate::entities::authorization::{deny_from, deny_rule, grant_from, grant_v2, role, role_from};
use crate::entities::identity::{api_key, session};
use crate::entities::permissions::group;
use crate::entities::principals::principal;

/// The principal kind whose API keys are personal keys; a key of any other
/// principal kind is an agent key.
const USER_PRINCIPAL_KIND: &str = "user";
const AGENT_PRINCIPAL_KIND: &str = "agent";

/// Existence of Custos resources, one query per kind.
pub struct PgCustosResourceStore {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl CustosResourceStore for PgCustosResourceStore {
    /// Users and agents are principals of that kind. A soft-deleted group
    /// is missing. A personal API key is one acting as a user principal,
    /// an agent key one acting as any other principal. Kinds without rows
    /// are never asked and answer nothing.
    async fn existing(&self, kind: CustosKind, ids: &[Uuid]) -> Result<HashSet<Uuid>, DomainError> {
        let conn = &self.conn;

        match kind {
            CustosKind::User => {
                existing_ids::<principal::Entity, _>(
                    conn,
                    principal::Column::Id,
                    ids,
                    Condition::all().add(principal::Column::Kind.eq(USER_PRINCIPAL_KIND)),
                )
                .await
            }
            CustosKind::Agent => {
                existing_ids::<principal::Entity, _>(
                    conn,
                    principal::Column::Id,
                    ids,
                    Condition::all().add(principal::Column::Kind.eq(AGENT_PRINCIPAL_KIND)),
                )
                .await
            }
            CustosKind::Group => {
                existing_ids::<group::Entity, _>(
                    conn,
                    group::Column::Id,
                    ids,
                    Condition::all().add(group::Column::DeletedAt.is_null()),
                )
                .await
            }
            CustosKind::Role => {
                existing_ids::<role::Entity, _>(conn, role::Column::Id, ids, Condition::all()).await
            }
            CustosKind::Grant => {
                existing_ids::<grant_v2::Entity, _>(
                    conn,
                    grant_v2::Column::Id,
                    ids,
                    Condition::all(),
                )
                .await
            }
            CustosKind::Deny => {
                existing_ids::<deny_rule::Entity, _>(
                    conn,
                    deny_rule::Column::Id,
                    ids,
                    Condition::all(),
                )
                .await
            }
            CustosKind::Session => {
                existing_ids::<session::Entity, _>(conn, session::Column::Id, ids, Condition::all())
                    .await
            }
            CustosKind::PersonalApiKey => existing_api_keys(conn, ids, true).await,
            CustosKind::AgentApiKey => existing_api_keys(conn, ids, false).await,
            CustosKind::Platform | CustosKind::Audit | CustosKind::ShareLinkCredential => {
                Ok(HashSet::new())
            }
        }
    }
}

/// The ids among `ids` of rows of `E` matching `condition`.
async fn existing_ids<E, C>(
    conn: &impl ConnectionTrait,
    id: C,
    ids: &[Uuid],
    condition: Condition,
) -> Result<HashSet<Uuid>, DomainError>
where
    E: EntityTrait,
    C: ColumnTrait,
{
    let found: Vec<Uuid> = E::find()
        .select_only()
        .column(id)
        .filter(id.is_in(ids.to_vec()))
        .filter(condition)
        .into_tuple()
        .all(conn)
        .await
        .map_err(db_err)?;

    Ok(found.into_iter().collect())
}

async fn existing_api_keys(
    conn: &impl ConnectionTrait,
    ids: &[Uuid],
    personal: bool,
) -> Result<HashSet<Uuid>, DomainError> {
    let owner_kind = if personal {
        principal::Column::Kind.eq(USER_PRINCIPAL_KIND)
    } else {
        principal::Column::Kind.ne(USER_PRINCIPAL_KIND)
    };

    let found: Vec<Uuid> = api_key::Entity::find()
        .select_only()
        .column(api_key::Column::Id)
        .join(
            sea_orm::JoinType::InnerJoin,
            api_key::Relation::Principal.def(),
        )
        .filter(api_key::Column::Id.is_in(ids.to_vec()))
        .filter(owner_kind)
        .into_tuple()
        .all(conn)
        .await
        .map_err(db_err)?;

    Ok(found.into_iter().collect())
}

/// The actor's live V1 group memberships.
pub struct PgGroupMembershipSource {
    pub conn: DatabaseConnection,
}

#[derive(Debug, FromQueryResult)]
struct GroupIdRow {
    group_id: Uuid,
}

#[async_trait]
impl GroupMembershipSource for PgGroupMembershipSource {
    /// V1 groups hold users only, keyed by the user id, which is the user's
    /// principal id; any other principal matches no membership row and has
    /// no groups. Soft-deleted groups are excluded.
    async fn groups_of(&self, principal: PrincipalId) -> Result<Vec<GroupId>, DomainError> {
        let rows = GroupIdRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT m.group_id FROM custos.group_members m \
             JOIN custos.groups g ON g.id = m.group_id \
             WHERE m.user_id = $1 AND g.deleted_at IS NULL \
             ORDER BY m.group_id",
            [principal.0.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows.into_iter().map(|row| GroupId(row.group_id)).collect())
    }
}

/// The stored-facts load of V2 grants and deny rules.
pub struct PgAuthorizationFactsStore {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl AuthorizationFactsStore for PgAuthorizationFactsStore {
    /// One statement each for grants and deny rules, plus one for the custom
    /// roles the grants reference when there are any.
    async fn load(
        &self,
        scope: &ProductScope,
        principal: PrincipalId,
        groups: &[GroupId],
    ) -> Result<StoredAuthorizationFacts, DomainError> {
        let products = match scope {
            ProductScope::Products(products) if products.is_empty() => {
                return Ok(StoredAuthorizationFacts::default());
            }
            ProductScope::Products(products) => Some(products.clone()),
            ProductScope::All => None,
        };

        let mut grant_query = grant_v2::Entity::find().filter(reaching(
            grant_v2::Column::SubjectPrincipalId,
            grant_v2::Column::SubjectGroupId,
            grant_v2::Column::SubjectPrincipalSet,
            principal,
            groups,
        ));
        let mut deny_query = deny_rule::Entity::find().filter(reaching(
            deny_rule::Column::SubjectPrincipalId,
            deny_rule::Column::SubjectGroupId,
            deny_rule::Column::SubjectPrincipalSet,
            principal,
            groups,
        ));

        if let Some(products) = products {
            grant_query = grant_query.filter(grant_v2::Column::Product.is_in(products.clone()));
            deny_query = deny_query.filter(deny_rule::Column::Product.is_in(products));
        }

        let grants = grant_query
            .order_by_asc(grant_v2::Column::Id)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(grant_from)
            .collect::<Result<Vec<_>, _>>()?;
        let denies = deny_query
            .order_by_asc(deny_rule::Column::Id)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(deny_from)
            .collect::<Result<Vec<_>, _>>()?;

        let role_ids: Vec<Uuid> = grants
            .iter()
            .filter_map(|grant| match &grant.authority {
                GrantAuthority::CustomRole(id) => Some(id.0),
                GrantAuthority::Builtin { .. } | GrantAuthority::Actions(_) => None,
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let custom_roles = if role_ids.is_empty() {
            Vec::new()
        } else {
            role::Entity::find()
                .filter(role::Column::Id.is_in(role_ids))
                .order_by_asc(role::Column::Id)
                .all(&self.conn)
                .await
                .map_err(db_err)?
                .into_iter()
                .map(role_from)
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(StoredAuthorizationFacts {
            grants,
            denies,
            custom_roles,
        })
    }
}

/// Rows addressed to the principal, to one of its groups, or to any
/// principal set.
fn reaching<C: ColumnTrait>(
    principal_column: C,
    group_column: C,
    principal_set_column: C,
    principal: PrincipalId,
    groups: &[GroupId],
) -> Condition {
    let mut condition = Condition::any()
        .add(principal_column.eq(principal.0))
        .add(principal_set_column.is_not_null());

    if !groups.is_empty() {
        let group_ids: Vec<Uuid> = groups.iter().map(|group| group.0).collect();
        condition = condition.add(group_column.is_in(group_ids));
    }

    condition
}
