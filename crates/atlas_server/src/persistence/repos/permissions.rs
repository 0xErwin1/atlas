use async_trait::async_trait;
use atlas_domain::{
    DomainError,
    entities::permissions::{NewPermissionGrant, PermissionGrant, PermissionGrantId},
    ids::{ApiKeyId, BoardId, DocumentId, FolderId, ProjectId, UserId, WorkspaceId},
    permissions::{ResourceRef, ResourceRole},
};
use chrono::Utc;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use uuid::Uuid;

use crate::persistence::entities::permissions::permission_grant;

pub use atlas_domain::ports::permission_grant_repo::{PermissionGrantRepo, ResolutionQuery};

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

fn grant_from(m: permission_grant::Model) -> Result<PermissionGrant, DomainError> {
    let role = role_from_str(&m.role)?;
    Ok(PermissionGrant {
        id: PermissionGrantId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        user_id: m.user_id.map(UserId),
        api_key_id: m.api_key_id.map(ApiKeyId),
        project_id: m.project_id.map(ProjectId),
        folder_id: m.folder_id.map(FolderId),
        document_id: m.document_id.map(DocumentId),
        board_id: m.board_id.map(BoardId),
        role,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_by_api_key_id: m.created_by_api_key_id.map(ApiKeyId),
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
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
            project_id: Option<Uuid>,
            folder_id: Option<Uuid>,
            document_id: Option<Uuid>,
            board_id: Option<Uuid>,
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
            SELECT project_id, folder_id, document_id, board_id, role
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
            let resource = if let Some(pid) = row.project_id {
                ResourceRef::Project(ProjectId(pid))
            } else if let Some(fid) = row.folder_id {
                ResourceRef::Folder(FolderId(fid))
            } else if let Some(did) = row.document_id {
                ResourceRef::Document(DocumentId(did))
            } else if let Some(bid) = row.board_id {
                ResourceRef::Board(BoardId(bid))
            } else {
                ResourceRef::Workspace
            };
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

        conn.execute_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            r#"
            INSERT INTO permission_grants
                (id, workspace_id, user_id, api_key_id,
                 project_id, folder_id, document_id, board_id,
                 role, created_by_user_id, created_by_api_key_id,
                 created_at, updated_at)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)
            ON CONFLICT (workspace_id, user_id, api_key_id, project_id, folder_id, document_id, board_id)
            DO UPDATE SET role = EXCLUDED.role, updated_at = EXCLUDED.updated_at
            "#,
            [
                id.0.into(),
                grant.workspace_id.0.into(),
                grant.user_id.map(|u| u.0).into(),
                grant.api_key_id.map(|k| k.0).into(),
                grant.project_id.map(|p| p.0).into(),
                grant.folder_id.map(|f| f.0).into(),
                grant.document_id.map(|d| d.0).into(),
                grant.board_id.map(|b| b.0).into(),
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
            .filter(match grant.project_id {
                Some(pid) => permission_grant::Column::ProjectId.eq(pid.0),
                None => permission_grant::Column::ProjectId.is_null(),
            })
            .filter(match grant.folder_id {
                Some(fid) => permission_grant::Column::FolderId.eq(fid.0),
                None => permission_grant::Column::FolderId.is_null(),
            })
            .filter(match grant.document_id {
                Some(did) => permission_grant::Column::DocumentId.eq(did.0),
                None => permission_grant::Column::DocumentId.is_null(),
            })
            .filter(match grant.board_id {
                Some(bid) => permission_grant::Column::BoardId.eq(bid.0),
                None => permission_grant::Column::BoardId.is_null(),
            })
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
            project_id: Option<Uuid>,
            folder_id: Option<Uuid>,
            document_id: Option<Uuid>,
            board_id: Option<Uuid>,
            role: String,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();

        values.push(query.workspace_id.0.into());
        let ws_param = values.len();

        let principal_condition = if let Some(uid) = query.user_id {
            values.push(uid.into());
            format!("user_id = ${}", values.len())
        } else if let Some(kid) = query.api_key_id {
            values.push(kid.into());
            format!("api_key_id = ${}", values.len())
        } else {
            return Ok(vec![]);
        };

        let projects_cond = if query.chain_projects.is_empty() {
            String::new()
        } else {
            let placeholders: String = query
                .chain_projects
                .iter()
                .map(|id| {
                    values.push((*id).into());
                    format!("${}", values.len())
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("OR project_id = ANY(ARRAY[{placeholders}]::uuid[])")
        };

        let folders_cond = if query.chain_folders.is_empty() {
            String::new()
        } else {
            let placeholders: String = query
                .chain_folders
                .iter()
                .map(|id| {
                    values.push((*id).into());
                    format!("${}", values.len())
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("OR folder_id = ANY(ARRAY[{placeholders}]::uuid[])")
        };

        let doc_cond = if let Some(id) = query.doc_id {
            values.push(id.into());
            format!("OR document_id = ${}", values.len())
        } else {
            String::new()
        };

        let board_cond = if let Some(id) = query.board_id {
            values.push(id.into());
            format!("OR board_id = ${}", values.len())
        } else {
            String::new()
        };

        let sql = format!(
            r#"
            SELECT project_id, folder_id, document_id, board_id, role
            FROM permission_grants
            WHERE workspace_id = ${ws_param}
              AND {principal_condition}
              AND ( num_nonnulls(project_id, folder_id, document_id, board_id) = 0
                   {projects_cond}
                   {folders_cond}
                   {doc_cond}
                   {board_cond} )
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
            let resource = if let Some(pid) = row.project_id {
                ResourceRef::Project(ProjectId(pid))
            } else if let Some(fid) = row.folder_id {
                ResourceRef::Folder(FolderId(fid))
            } else if let Some(did) = row.document_id {
                ResourceRef::Document(DocumentId(did))
            } else if let Some(bid) = row.board_id {
                ResourceRef::Board(BoardId(bid))
            } else {
                ResourceRef::Workspace
            };
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
        let mut query = permission_grant::Entity::find()
            .filter(permission_grant::Column::WorkspaceId.eq(workspace_id.0));

        query = match resource {
            ResourceRef::Workspace => query
                .filter(permission_grant::Column::ProjectId.is_null())
                .filter(permission_grant::Column::FolderId.is_null())
                .filter(permission_grant::Column::DocumentId.is_null())
                .filter(permission_grant::Column::BoardId.is_null()),
            ResourceRef::Project(pid) => {
                query.filter(permission_grant::Column::ProjectId.eq(pid.0))
            }
            ResourceRef::Folder(fid) => query.filter(permission_grant::Column::FolderId.eq(fid.0)),
            ResourceRef::Document(did) => {
                query.filter(permission_grant::Column::DocumentId.eq(did.0))
            }
            ResourceRef::Board(bid) => query.filter(permission_grant::Column::BoardId.eq(bid.0)),
        };

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
