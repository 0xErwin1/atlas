use crate::authz::ResourceRole;
use async_trait::async_trait;
use atlas_acta::ids::BoardId;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::FolderId;
use atlas_acta::ids::ProjectId;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::permissions::ResourceRef;
use atlas_acta::permissions::resource_ref_codec;
use atlas_core::error::DomainError;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::GroupId;
use atlas_core::principal::UserId;
use chrono::Utc;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use uuid::Uuid;

use crate::authz::policy::{NewPermissionGrant, PermissionGrant, PermissionGrantId};
use atlas_custos_postgres::entities::permissions::permission_grant;
use atlas_postgres::db_err;

// R1 scaffolding: `PgGroupRepo` now lives in `atlas_custos_postgres`.
// `PermissionGrant`/`PermissionGrantRepo` stay parked here through S3a/S3b
// (D5): they still name five Acta id fields, so the impl below cannot move
// into a crate that must not depend on `atlas_acta`. S3c unparks the cluster
// once `resource_ref` replaces those fields.
pub use atlas_custos_postgres::repos::permissions::PgGroupRepo;

pub use crate::authz::policy::{PermissionGrantRepo, ResolutionQuery};

fn role_from_str(s: &str) -> Result<ResourceRole, DomainError> {
    match s {
        "viewer" => Ok(ResourceRole::Viewer),
        "editor" => Ok(ResourceRole::Editor),
        "admin" => Ok(ResourceRole::Admin),
        other => Err(DomainError::Internal {
            message: format!("unknown permission role in DB: {other}"),
        }),
    }
}

fn role_to_str(role: ResourceRole) -> &'static str {
    match role {
        ResourceRole::Viewer => "viewer",
        ResourceRole::Editor => "editor",
        ResourceRole::Admin => "admin",
    }
}

/// Decodes a stored `resource_ref` string back to the V1 `ResourceRef` enum,
/// scoped to the grant's own `workspace_id` (the codec validates a decoded
/// `workspace` kind against the workspace it is asked to decode against).
fn decode_resource_ref(
    resource_ref: &str,
    workspace_id: WorkspaceId,
) -> Result<ResourceRef, DomainError> {
    let core: atlas_core::ids::ResourceRef =
        resource_ref.parse().map_err(|_| DomainError::Internal {
            message: format!("permission grant row has an invalid resource_ref: {resource_ref}"),
        })?;

    resource_ref_codec::from_core(&core, workspace_id)
}

fn grant_from(m: permission_grant::Model) -> Result<PermissionGrant, DomainError> {
    let role = role_from_str(&m.role)?;
    let workspace_id = WorkspaceId(m.workspace_id);
    let resource = decode_resource_ref(&m.resource_ref, workspace_id)?;

    Ok(PermissionGrant {
        id: PermissionGrantId(m.id),
        workspace_id,
        user_id: m.user_id.map(UserId),
        api_key_id: m.api_key_id.map(ApiKeyId),
        group_id: m.group_id.map(GroupId),
        project_id: match &resource {
            ResourceRef::Project(id) => Some(*id),
            _ => None,
        },
        folder_id: match &resource {
            ResourceRef::Folder(id) => Some(*id),
            _ => None,
        },
        document_id: match &resource {
            ResourceRef::Document(id) => Some(*id),
            _ => None,
        },
        board_id: match &resource {
            ResourceRef::Board(id) => Some(*id),
            _ => None,
        },
        role,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_by_api_key_id: m.created_by_api_key_id.map(ApiKeyId),
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

/// Narrows a grant query to one resource by its encoded `resource_ref`.
fn filter_by_resource(
    query: sea_orm::Select<permission_grant::Entity>,
    workspace_id: WorkspaceId,
    resource: &ResourceRef,
) -> sea_orm::Select<permission_grant::Entity> {
    let resource_ref = resource_ref_codec::to_core(resource, workspace_id).to_string();
    query.filter(permission_grant::Column::ResourceRef.eq(resource_ref))
}

/// A grant's target column is at most one; the resource it targets falls
/// back to the workspace itself when all four are unset.
fn target_resource(
    project_id: Option<ProjectId>,
    folder_id: Option<FolderId>,
    document_id: Option<DocumentId>,
    board_id: Option<BoardId>,
) -> ResourceRef {
    if let Some(id) = project_id {
        ResourceRef::Project(id)
    } else if let Some(id) = folder_id {
        ResourceRef::Folder(id)
    } else if let Some(id) = document_id {
        ResourceRef::Document(id)
    } else if let Some(id) = board_id {
        ResourceRef::Board(id)
    } else {
        ResourceRef::Workspace
    }
}

pub struct PgPermissionGrantRepo {
    pub conn: DatabaseConnection,
}

impl PgPermissionGrantRepo {
    /// Returns all grants the principal holds anywhere in the given workspace.
    ///
    /// Used by the workspace activity feed handler to collect board-only grants
    /// that would not be returned by `load_grants_for_resolution` when no
    /// specific board_id is passed (because board grants have a non-null board_id
    /// and are not matched by the `num_nonnulls = 0` workspace-scope predicate).
    pub async fn list_all_for_principal_in_workspace(
        &self,
        workspace_id: WorkspaceId,
        user_id: Option<UserId>,
        api_key_id: Option<ApiKeyId>,
    ) -> Result<Vec<(ResourceRef, ResourceRole)>, DomainError> {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct Row {
            resource_ref: String,
            role: String,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();
        values.push(workspace_id.0.into());

        let principal_condition = if let Some(uid) = user_id {
            values.push(uid.0.into());
            format!("user_id = ${}", values.len())
        } else if let Some(kid) = api_key_id {
            values.push(kid.0.into());
            format!("api_key_id = ${}", values.len())
        } else {
            return Ok(vec![]);
        };

        let sql = format!(
            r#"
            SELECT resource_ref, role
            FROM permission_grants
            WHERE workspace_id = $1
              AND {principal_condition}
            "#,
        );

        let rows = Row::find_by_statement(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let role = role_from_str(&row.role)?;
            let resource = decode_resource_ref(&row.resource_ref, workspace_id)?;
            result.push((resource, role));
        }

        Ok(result)
    }

    /// Returns whether the principal holds at least one grant anywhere in the
    /// workspace (workspace-scope, project, folder, document, or board). Used by
    /// the workspace-access gate to admit grant-bearing non-members.
    pub async fn principal_has_any_grant_in_workspace(
        &self,
        workspace_id: WorkspaceId,
        user_id: Option<UserId>,
        api_key_id: Option<ApiKeyId>,
    ) -> Result<bool, DomainError> {
        use sea_orm::FromQueryResult;

        let mut values: Vec<sea_orm::Value> = Vec::new();
        values.push(workspace_id.0.into());

        let principal_condition = if let Some(uid) = user_id {
            values.push(uid.0.into());
            format!("user_id = ${}", values.len())
        } else if let Some(kid) = api_key_id {
            values.push(kid.0.into());
            format!("api_key_id = ${}", values.len())
        } else {
            return Ok(false);
        };

        #[derive(Debug, FromQueryResult)]
        struct Exists {
            present: bool,
        }

        let sql = format!(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM permission_grants
                WHERE workspace_id = $1
                  AND {principal_condition}
            ) AS present
            "#,
        );

        let row = Exists::find_by_statement(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .one(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(row.map(|r| r.present).unwrap_or(false))
    }
}

impl PgPermissionGrantRepo {
    /// Upserts a permission grant using the provided connection or transaction and
    /// returns the resulting row. Both the INSERT and the SELECT-back run on the
    /// same connection so the read sees the uncommitted write.
    ///
    /// Used to run the upsert atomically alongside an audit-log write inside a
    /// transaction, so the audit row is written iff the mutation commits.
    pub async fn upsert_in<C: ConnectionTrait>(
        conn: &C,
        grant: NewPermissionGrant,
    ) -> Result<PermissionGrant, DomainError> {
        let id = PermissionGrantId::new();
        let now = Utc::now();
        let role_str = role_to_str(grant.role);
        let resource = target_resource(
            grant.project_id,
            grant.folder_id,
            grant.document_id,
            grant.board_id,
        );
        let resource_ref = resource_ref_codec::to_core(&resource, grant.workspace_id).to_string();

        conn.execute_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            r#"
            INSERT INTO permission_grants
                (id, workspace_id, user_id, api_key_id, group_id, resource_ref,
                 role, created_by_user_id, created_by_api_key_id,
                 created_at, updated_at)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)
            ON CONFLICT (workspace_id, user_id, api_key_id, group_id, resource_ref)
            DO UPDATE SET role = EXCLUDED.role, updated_at = EXCLUDED.updated_at
            "#,
            [
                id.0.into(),
                grant.workspace_id.0.into(),
                grant.user_id.map(|u| u.0).into(),
                grant.api_key_id.map(|k| k.0).into(),
                grant.group_id.map(|g| g.0).into(),
                resource_ref.clone().into(),
                role_str.into(),
                grant.created_by_user_id.map(|u| u.0).into(),
                grant.created_by_api_key_id.map(|k| k.0).into(),
                now.into(),
            ],
        ))
        .await
        .map_err(db_err)?;

        let row = permission_grant::Entity::find()
            .filter(permission_grant::Column::WorkspaceId.eq(grant.workspace_id.0))
            .filter(match grant.user_id {
                Some(uid) => permission_grant::Column::UserId.eq(uid.0),
                None => permission_grant::Column::UserId.is_null(),
            })
            .filter(match grant.api_key_id {
                Some(kid) => permission_grant::Column::ApiKeyId.eq(kid.0),
                None => permission_grant::Column::ApiKeyId.is_null(),
            })
            .filter(match grant.group_id {
                Some(gid) => permission_grant::Column::GroupId.eq(gid.0),
                None => permission_grant::Column::GroupId.is_null(),
            })
            .filter(permission_grant::Column::ResourceRef.eq(resource_ref))
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or_else(|| DomainError::Internal {
                message: "permission grant not found after upsert".into(),
            })?;

        grant_from(row)
    }

    /// Deletes a permission grant by id + workspace using the provided connection or
    /// transaction.
    ///
    /// Used to run the delete atomically alongside an audit-log write inside a
    /// transaction, so the audit row is written iff the mutation commits.
    pub async fn delete_in<C: ConnectionTrait>(
        conn: &C,
        grant_id: PermissionGrantId,
        workspace_id: WorkspaceId,
    ) -> Result<(), DomainError> {
        permission_grant::Entity::delete_many()
            .filter(permission_grant::Column::Id.eq(grant_id.0))
            .filter(permission_grant::Column::WorkspaceId.eq(workspace_id.0))
            .exec(conn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

#[async_trait]
impl PermissionGrantRepo for PgPermissionGrantRepo {
    async fn upsert(&self, grant: NewPermissionGrant) -> Result<PermissionGrant, DomainError> {
        PgPermissionGrantRepo::upsert_in(&self.conn, grant).await
    }

    async fn load_grants_for_resolution(
        &self,
        query: ResolutionQuery,
    ) -> Result<Vec<(ResourceRef, ResourceRole)>, DomainError> {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct Row {
            resource_ref: String,
            role: String,
        }

        let workspace_id = query.workspace_id;
        let mut values: Vec<sea_orm::Value> = Vec::new();

        values.push(workspace_id.0.into());
        let ws_param = values.len();

        // Group grants are gathered into the same max-role candidate set as direct
        // user grants. Grants whose group is soft-deleted are excluded as a
        // defense-in-depth layer on top of build_resolution_query's membership
        // filter; this covers a group deleted between the two queries.
        let principal_condition = if let Some(uid) = query.user_id {
            values.push(uid.into());
            let uid_param = values.len();

            if query.group_ids.is_empty() {
                format!("user_id = ${uid_param}")
            } else {
                let group_placeholders: String = query
                    .group_ids
                    .iter()
                    .map(|id| {
                        values.push((*id).into());
                        format!("${}", values.len())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                format!(
                    "(user_id = ${uid_param} OR (group_id = ANY(ARRAY[{group_placeholders}]::uuid[]) AND NOT EXISTS (SELECT 1 FROM groups g WHERE g.id = permission_grants.group_id AND g.deleted_at IS NOT NULL)))"
                )
            }
        } else if let Some(kid) = query.api_key_id {
            values.push(kid.into());
            format!("api_key_id = ${}", values.len())
        } else {
            return Ok(vec![]);
        };

        // Every resource this query might match, encoded as the canonical
        // resource_ref string: the workspace-scope ref (always present) plus
        // one ref per chain resource actually requested.
        let mut resource_refs: Vec<String> =
            vec![resource_ref_codec::to_core(&ResourceRef::Workspace, workspace_id).to_string()];
        resource_refs.extend(query.chain_projects.iter().map(|id| {
            resource_ref_codec::to_core(&ResourceRef::Project(ProjectId(*id)), workspace_id)
                .to_string()
        }));
        resource_refs.extend(query.chain_folders.iter().map(|id| {
            resource_ref_codec::to_core(&ResourceRef::Folder(FolderId(*id)), workspace_id)
                .to_string()
        }));
        resource_refs.extend(query.doc_id.map(|id| {
            resource_ref_codec::to_core(&ResourceRef::Document(DocumentId(id)), workspace_id)
                .to_string()
        }));
        resource_refs.extend(query.board_id.map(|id| {
            resource_ref_codec::to_core(&ResourceRef::Board(BoardId(id)), workspace_id).to_string()
        }));

        let resource_ref_placeholders: String = resource_refs
            .into_iter()
            .map(|resource_ref| {
                values.push(resource_ref.into());
                format!("${}", values.len())
            })
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            r#"
            SELECT resource_ref, role
            FROM permission_grants
            WHERE workspace_id = ${ws_param}
              AND {principal_condition}
              AND resource_ref = ANY(ARRAY[{resource_ref_placeholders}]::text[])
            "#,
        );

        let rows = Row::find_by_statement(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let role = role_from_str(&row.role)?;
            let resource = decode_resource_ref(&row.resource_ref, workspace_id)?;
            result.push((resource, role));
        }

        Ok(result)
    }

    async fn delete(
        &self,
        grant_id: PermissionGrantId,
        workspace_id: WorkspaceId,
    ) -> Result<(), DomainError> {
        PgPermissionGrantRepo::delete_in(&self.conn, grant_id, workspace_id).await
    }

    async fn list_for_resource(
        &self,
        workspace_id: WorkspaceId,
        resource: &ResourceRef,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<PermissionGrant>, DomainError> {
        let mut query = filter_by_resource(
            permission_grant::Entity::find()
                .filter(permission_grant::Column::WorkspaceId.eq(workspace_id.0)),
            workspace_id,
            resource,
        );

        if let Some(cursor) = after_id {
            query = query.filter(permission_grant::Column::Id.gt(cursor));
        }

        let rows = query
            .order_by_asc(permission_grant::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter().map(grant_from).collect()
    }

    async fn find_by_id(
        &self,
        workspace_id: WorkspaceId,
        resource: &ResourceRef,
        grant_id: PermissionGrantId,
    ) -> Result<Option<PermissionGrant>, DomainError> {
        let row = filter_by_resource(
            permission_grant::Entity::find()
                .filter(permission_grant::Column::Id.eq(grant_id.0))
                .filter(permission_grant::Column::WorkspaceId.eq(workspace_id.0)),
            workspace_id,
            resource,
        )
        .one(&self.conn)
        .await
        .map_err(db_err)?;

        row.map(grant_from).transpose()
    }

    async fn list_for_api_key(
        &self,
        api_key_id: ApiKeyId,
    ) -> Result<Vec<PermissionGrant>, DomainError> {
        let rows = permission_grant::Entity::find()
            .filter(permission_grant::Column::ApiKeyId.eq(api_key_id.0))
            .order_by_asc(permission_grant::Column::Id)
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter().map(grant_from).collect()
    }

    async fn delete_for_api_key(
        &self,
        grant_id: PermissionGrantId,
        api_key_id: ApiKeyId,
    ) -> Result<bool, DomainError> {
        let existing = permission_grant::Entity::find()
            .filter(permission_grant::Column::Id.eq(grant_id.0))
            .filter(permission_grant::Column::ApiKeyId.eq(api_key_id.0))
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        if existing.is_none() {
            return Ok(false);
        }

        permission_grant::Entity::delete_many()
            .filter(permission_grant::Column::Id.eq(grant_id.0))
            .filter(permission_grant::Column::ApiKeyId.eq(api_key_id.0))
            .exec(&self.conn)
            .await
            .map_err(db_err)?;

        Ok(true)
    }
}
