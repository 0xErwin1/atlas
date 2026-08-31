//! Repository implementations for the `workspace` and `workspace_membership`
//! ports, moved from `atlas_server::persistence::repos::identity` (S4 PR6).
//!
//! `WorkspaceRepo::list_for_api_key` reads `custos.permission_grants` by raw
//! SQL. This is a read, not a cross-domain write composition, so it moves
//! here unchanged per design D6 — the same discipline that keeps the raw
//! `custos.users`/`custos.api_keys` joins in `boards_tasks.rs`,
//! `workspace_core.rs`, `documents.rs`, and `search.rs` intact as those files
//! move in later PRs. No Acta table name appears in that query, so there is
//! nothing for the later `SET SCHEMA acta` batches to qualify here.
//! `PgUserRepo`/`PgSessionRepo`/`PgApiKeyRepo`/`PgActivationTokenRepo` (the
//! Custos-owned identity repos) and `PgUiStateRepo`/`user_ui_state` (D4) stay
//! in `atlas_server`/`atlas_custos_postgres`; neither belongs to this crate.

use async_trait::async_trait;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::identity::MemberRole;
use atlas_acta::entities::identity::WorkspaceMembership;
use atlas_acta::ids::MembershipId;
use atlas_acta::ids::WorkspaceId;
use atlas_core::error::DomainError;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::UserId;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, FromQueryResult, QueryFilter, Statement,
};
use uuid::Uuid;

use crate::entities::identity::{membership, membership_from, workspace, workspace_from};
use atlas_postgres::db_err;

pub use atlas_acta::entities::identity::NewWorkspace;
pub use atlas_acta::entities::identity::Workspace;

pub use atlas_acta::ports::identity::MembershipRepo;
pub use atlas_acta::ports::identity::WorkspaceRepo;

pub struct PgWorkspaceRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl WorkspaceRepo for PgWorkspaceRepo {
    async fn create(&self, new: NewWorkspace) -> Result<Workspace, DomainError> {
        let model = workspace::ActiveModel {
            id: Set(new.id.0),
            name: Set(new.name),
            slug: Set(new.slug),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map(workspace_from)
            .map_err(db_err)
    }

    async fn find_by_id(&self, id: WorkspaceId) -> Result<Option<Workspace>, DomainError> {
        workspace::Entity::find_by_id(id.0)
            .filter(workspace::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(workspace_from))
            .map_err(db_err)
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Workspace>, DomainError> {
        workspace::Entity::find()
            .filter(workspace::Column::Slug.eq(slug))
            .filter(workspace::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(workspace_from))
            .map_err(db_err)
    }

    async fn list_for_user(&self, user_id: UserId) -> Result<Vec<Workspace>, DomainError> {
        let ids: Vec<Uuid> = membership::Entity::find()
            .filter(membership::Column::UserId.eq(user_id.0))
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|m: membership::Model| m.workspace_id)
            .collect();

        let mut workspaces = Vec::new();
        for id in ids {
            if let Some(ws) = workspace::Entity::find_by_id(id)
                .filter(workspace::Column::DeletedAt.is_null())
                .one(&self.conn)
                .await
                .map_err(db_err)?
            {
                workspaces.push(workspace_from(ws));
            }
        }

        Ok(workspaces)
    }

    async fn list_memberships_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<(Workspace, MemberRole)>, DomainError> {
        let memberships = membership::Entity::find()
            .filter(membership::Column::UserId.eq(user_id.0))
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        let mut result = Vec::new();
        for m in memberships {
            let membership =
                membership_from(m).map_err(|message| DomainError::Internal { message })?;

            if let Some(ws) = workspace::Entity::find_by_id(membership.workspace_id.0)
                .filter(workspace::Column::DeletedAt.is_null())
                .one(&self.conn)
                .await
                .map_err(db_err)?
            {
                result.push((workspace_from(ws), membership.role));
            }
        }

        Ok(result)
    }

    async fn list_for_api_key(&self, api_key_id: ApiKeyId) -> Result<Vec<Workspace>, DomainError> {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct WorkspaceIdRow {
            workspace_id: Uuid,
        }

        let rows = WorkspaceIdRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT DISTINCT workspace_id FROM custos.permission_grants WHERE api_key_id = $1",
            [api_key_id.0.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        let mut workspaces = Vec::new();
        for row in rows {
            if let Some(ws) = workspace::Entity::find_by_id(row.workspace_id)
                .filter(workspace::Column::DeletedAt.is_null())
                .one(&self.conn)
                .await
                .map_err(db_err)?
            {
                workspaces.push(workspace_from(ws));
            }
        }

        Ok(workspaces)
    }

    async fn list_slugs(&self) -> Result<Vec<String>, DomainError> {
        #[derive(Debug, FromQueryResult)]
        struct SlugRow {
            slug: String,
        }

        // Includes soft-deleted workspaces on purpose: the `slug` unique
        // constraint still reserves a deleted workspace's slug, so collision
        // resolution must keep avoiding it.
        let rows = SlugRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT slug FROM workspaces",
            [],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows.into_iter().map(|r| r.slug).collect())
    }

    async fn rename(&self, id: WorkspaceId, name: String) -> Result<Workspace, DomainError> {
        use sea_orm::IntoActiveModel;

        let row = workspace::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "workspace",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.name = Set(name);
        active.updated_at = Set(Utc::now());

        active
            .update(&self.conn)
            .await
            .map(workspace_from)
            .map_err(db_err)
    }

    async fn list_all(&self) -> Result<Vec<Workspace>, DomainError> {
        use sea_orm::QueryOrder;

        workspace::Entity::find()
            .filter(workspace::Column::DeletedAt.is_null())
            .order_by_asc(workspace::Column::CreatedAt)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(workspace_from).collect())
            .map_err(db_err)
    }

    async fn set_slug(&self, id: WorkspaceId, slug: String) -> Result<Workspace, DomainError> {
        use sea_orm::IntoActiveModel;

        let row = workspace::Entity::find_by_id(id.0)
            .filter(workspace::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "workspace",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.slug = Set(slug);
        active.updated_at = Set(Utc::now());

        active
            .update(&self.conn)
            .await
            .map(workspace_from)
            .map_err(db_err)
    }

    async fn soft_delete(&self, id: WorkspaceId) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;

        let row = workspace::Entity::find_by_id(id.0)
            .filter(workspace::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "workspace",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());

        active.update(&self.conn).await.map_err(db_err)?;

        Ok(())
    }
}

pub struct PgMembershipRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl MembershipRepo for PgMembershipRepo {
    async fn add(
        &self,
        ctx: &WorkspaceCtx,
        user_id: UserId,
        role: MemberRole,
    ) -> Result<WorkspaceMembership, DomainError> {
        let model = membership::ActiveModel {
            id: Set(MembershipId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            user_id: Set(user_id.0),
            role: Set(role.as_str().to_string()),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        };
        model
            .insert(&self.conn)
            .await
            .map_err(db_err)
            .and_then(|m: membership::Model| {
                membership_from(m).map_err(|e| DomainError::Internal { message: e })
            })
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        user_id: UserId,
    ) -> Result<Option<WorkspaceMembership>, DomainError> {
        membership::Entity::find()
            .filter(membership::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(membership::Column::UserId.eq(user_id.0))
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(membership_from)
            .transpose()
            .map_err(|e| DomainError::Internal { message: e })
    }

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<WorkspaceMembership>, DomainError> {
        let rows = membership::Entity::find()
            .filter(membership::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter()
            .map(|m: membership::Model| {
                membership_from(m).map_err(|e| DomainError::Internal { message: e })
            })
            .collect()
    }

    async fn remove(&self, ctx: &WorkspaceCtx, user_id: UserId) -> Result<(), DomainError> {
        let retained_draft = crate::entities::documents::comment_attachment_draft::Entity::find()
            .filter(
                crate::entities::documents::comment_attachment_draft::Column::WorkspaceId
                    .eq(ctx.workspace_id.0),
            )
            .filter(
                crate::entities::documents::comment_attachment_draft::Column::CreatedByUserId
                    .eq(user_id.0),
            )
            .one(&self.conn)
            .await
            .map_err(db_err)?;
        if retained_draft.is_some() {
            return Err(DomainError::CommentDraftConflict {
                reason: "user has retained comment draft state".into(),
            });
        }

        membership::Entity::delete_many()
            .filter(membership::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(membership::Column::UserId.eq(user_id.0))
            .exec(&self.conn)
            .await
            .map(|_| ())
            .map_err(db_err)
    }

    async fn update_role(
        &self,
        ctx: &WorkspaceCtx,
        user_id: UserId,
        role: MemberRole,
    ) -> Result<WorkspaceMembership, DomainError> {
        let existing = membership::Entity::find()
            .filter(membership::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(membership::Column::UserId.eq(user_id.0))
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "WorkspaceMembership",
                id: user_id.0,
            })?;

        let mut active: membership::ActiveModel = existing.into();
        active.role = Set(role.as_str().to_string());
        active.updated_at = Set(Utc::now());

        active
            .update(&self.conn)
            .await
            .map_err(db_err)
            .and_then(|m: membership::Model| {
                membership_from(m).map_err(|e| DomainError::Internal { message: e })
            })
    }
}

impl PgMembershipRepo {
    /// Inserts a workspace membership using the provided connection or transaction.
    ///
    /// Used when the caller needs to run the insert atomically inside an existing
    /// transaction alongside an audit-log write.
    pub async fn add_in<C: ConnectionTrait>(
        conn: &C,
        ctx: &WorkspaceCtx,
        user_id: UserId,
        role: MemberRole,
    ) -> Result<WorkspaceMembership, DomainError> {
        let model = membership::ActiveModel {
            id: Set(MembershipId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            user_id: Set(user_id.0),
            role: Set(role.as_str().to_string()),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        };
        model
            .insert(conn)
            .await
            .map_err(db_err)
            .and_then(|m: membership::Model| {
                membership_from(m).map_err(|e| DomainError::Internal { message: e })
            })
    }

    /// Removes a workspace membership using the provided connection or transaction.
    ///
    /// Used when the caller needs to run the delete atomically inside an existing
    /// transaction alongside an audit-log write.
    pub async fn remove_in<C: ConnectionTrait>(
        conn: &C,
        ctx: &WorkspaceCtx,
        user_id: UserId,
    ) -> Result<(), DomainError> {
        membership::Entity::delete_many()
            .filter(membership::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(membership::Column::UserId.eq(user_id.0))
            .exec(conn)
            .await
            .map(|_| ())
            .map_err(db_err)
    }

    /// Updates a workspace member's role using the provided connection or transaction.
    ///
    /// Used when the caller needs to run the update atomically inside an existing
    /// transaction alongside an audit-log write.
    pub async fn update_role_in<C: ConnectionTrait>(
        conn: &C,
        ctx: &WorkspaceCtx,
        user_id: UserId,
        role: MemberRole,
    ) -> Result<WorkspaceMembership, DomainError> {
        let existing = membership::Entity::find()
            .filter(membership::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(membership::Column::UserId.eq(user_id.0))
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "WorkspaceMembership",
                id: user_id.0,
            })?;

        let mut active: membership::ActiveModel = existing.into();
        active.role = Set(role.as_str().to_string());
        active.updated_at = Set(Utc::now());

        active
            .update(conn)
            .await
            .map_err(db_err)
            .and_then(|m: membership::Model| {
                membership_from(m).map_err(|e| DomainError::Internal { message: e })
            })
    }
}
