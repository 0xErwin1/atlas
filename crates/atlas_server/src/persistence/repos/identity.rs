use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::identity::{MemberRole, WorkspaceMembership},
    ids::{ApiKeyId, MembershipId, SessionId, UserId, WorkspaceId},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    FromQueryResult, QueryFilter, Statement,
};
use uuid::Uuid;

use crate::persistence::entities::identity::{
    api_key, api_key_from, membership, membership_from, session, session_from, user, user_from,
    workspace, workspace_from,
};

pub use atlas_domain::entities::identity::{
    ApiKey, NewApiKey, NewSession, NewUser, NewWorkspace, Session, User, Workspace,
};

pub use atlas_domain::ports::identity::{
    ApiKeyRepo, MembershipRepo, SessionRepo, UserRepo, WorkspaceRepo,
};

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
        };
        model
            .insert(&self.conn)
            .await
            .map(workspace_from)
            .map_err(db_err)
    }

    async fn find_by_id(&self, id: WorkspaceId) -> Result<Option<Workspace>, DomainError> {
        workspace::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map(|opt| opt.map(workspace_from))
            .map_err(db_err)
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<Workspace>, DomainError> {
        workspace::Entity::find()
            .filter(workspace::Column::Slug.eq(slug))
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
                .one(&self.conn)
                .await
                .map_err(db_err)?
            {
                workspaces.push(workspace_from(ws));
            }
        }

        Ok(workspaces)
    }
}

pub struct PgUserRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl UserRepo for PgUserRepo {
    async fn create(&self, new: NewUser) -> Result<User, DomainError> {
        let uid = atlas_domain::ids::UserId::new();
        let model = user::ActiveModel {
            id: Set(uid.0),
            username: Set(new.username),
            display_name: Set(new.display_name),
            email: Set(new.email),
            password_hash: Set(new.password_hash),
            is_root: Set(new.is_root),
            disabled_at: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        };
        model
            .insert(&self.conn)
            .await
            .map(user_from)
            .map_err(db_err)
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<User>, DomainError> {
        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: Uuid,
            username: String,
            display_name: String,
            email: Option<String>,
            password_hash: String,
            is_root: bool,
            disabled_at: Option<chrono::DateTime<Utc>>,
            created_at: chrono::DateTime<Utc>,
            updated_at: chrono::DateTime<Utc>,
        }

        let lower = username.to_lowercase();
        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT id, username, display_name, email, password_hash, is_root, disabled_at, created_at, updated_at
             FROM users WHERE lower(username) = $1 LIMIT 1",
            [lower.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows.into_iter().next().map(|r| User {
            id: UserId(r.id),
            username: r.username,
            display_name: r.display_name,
            email: r.email,
            password_hash: r.password_hash,
            is_root: r.is_root,
            disabled_at: r.disabled_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, DomainError> {
        user::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map(|opt| opt.map(user_from))
            .map_err(db_err)
    }

    async fn find_root(&self) -> Result<Option<User>, DomainError> {
        user::Entity::find()
            .filter(user::Column::IsRoot.eq(true))
            .one(&self.conn)
            .await
            .map(|opt| opt.map(user_from))
            .map_err(db_err)
    }

    async fn list(&self) -> Result<Vec<User>, DomainError> {
        use sea_orm::QueryOrder;
        user::Entity::find()
            .order_by_asc(user::Column::CreatedAt)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(user_from).collect())
            .map_err(db_err)
    }

    async fn disable(&self, id: UserId) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;
        let mut active = row.into_active_model();
        active.disabled_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn enable(&self, id: UserId) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;
        let mut active = row.into_active_model();
        active.disabled_at = Set(None);
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn set_password_hash(&self, id: UserId, hash: String) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;
        let mut active = row.into_active_model();
        active.password_hash = Set(hash);
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn update_profile(
        &self,
        id: UserId,
        email: Option<String>,
        display_name: Option<String>,
    ) -> Result<User, DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;

        let mut active = row.into_active_model();

        if let Some(email) = email {
            active.email = Set(Some(email));
        }
        if let Some(display_name) = display_name {
            active.display_name = Set(display_name);
        }
        active.updated_at = Set(Utc::now());

        active
            .update(&self.conn)
            .await
            .map(user_from)
            .map_err(db_err)
    }
}

pub struct PgSessionRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl SessionRepo for PgSessionRepo {
    async fn create(&self, new: NewSession) -> Result<Session, DomainError> {
        let model = session::ActiveModel {
            id: Set(SessionId::new().0),
            user_id: Set(new.user_id.0),
            token_hash: Set(new.token_hash),
            expires_at: Set(new.expires_at),
            last_used_at: Set(None),
            revoked_at: Set(None),
            created_at: Set(Utc::now()),
        };
        model
            .insert(&self.conn)
            .await
            .map(session_from)
            .map_err(db_err)
    }

    async fn find_active_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Session>, DomainError> {
        session::Entity::find()
            .filter(session::Column::TokenHash.eq(token_hash))
            .filter(session::Column::RevokedAt.is_null())
            .filter(session::Column::ExpiresAt.gt(Utc::now()))
            .one(&self.conn)
            .await
            .map(|opt| opt.map(session_from))
            .map_err(db_err)
    }

    async fn revoke(&self, id: SessionId) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = session::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "session",
                id: id.0,
            })?;
        let mut active = row.into_active_model();
        active.revoked_at = Set(Some(Utc::now()));
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn revoke_all_for_user(&self, user_id: UserId) -> Result<(), DomainError> {
        use sea_orm::ConnectionTrait;
        self.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE sessions SET revoked_at = now()
                 WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()",
                [user_id.0.into()],
            ))
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn touch(
        &self,
        id: SessionId,
        ttl_hours: i64,
        max_ttl_hours: i64,
    ) -> Result<(), DomainError> {
        use sea_orm::ConnectionTrait;
        self.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE sessions
                 SET last_used_at = now(),
                     expires_at = LEAST(now() + ($2 * interval '1 hour'), created_at + ($3 * interval '1 hour'))
                 WHERE id = $1
                   AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')",
                [id.0.into(), ttl_hours.into(), max_ttl_hours.into()],
            ))
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

pub struct PgApiKeyRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl ApiKeyRepo for PgApiKeyRepo {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewApiKey) -> Result<ApiKey, DomainError> {
        let created_by_user_id = match ctx.actor {
            Actor::User(uid) => uid.0,
            Actor::ApiKey(_) => {
                return Err(DomainError::InvalidInput {
                    message: "api keys must be created by a user actor".into(),
                });
            }
        };
        let model = api_key::ActiveModel {
            id: Set(ApiKeyId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            created_by_user_id: Set(created_by_user_id),
            name: Set(new.name),
            token_hash: Set(new.token_hash),
            expires_at: Set(new.expires_at),
            last_used_at: Set(None),
            revoked_at: Set(None),
            created_at: Set(Utc::now()),
        };
        model
            .insert(&self.conn)
            .await
            .map(api_key_from)
            .map_err(db_err)
    }

    async fn find_active_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiKey>, DomainError> {
        #[derive(Debug, sea_orm::FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            workspace_id: uuid::Uuid,
            created_by_user_id: uuid::Uuid,
            name: String,
            token_hash: String,
            expires_at: Option<chrono::DateTime<Utc>>,
            last_used_at: Option<chrono::DateTime<Utc>>,
            revoked_at: Option<chrono::DateTime<Utc>>,
            created_at: chrono::DateTime<Utc>,
        }

        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT k.id, k.workspace_id, k.created_by_user_id, k.name, k.token_hash,
                    k.expires_at, k.last_used_at, k.revoked_at, k.created_at
             FROM api_keys k
             JOIN users u ON u.id = k.created_by_user_id
             WHERE k.token_hash = $1
               AND k.revoked_at IS NULL
               AND (k.expires_at IS NULL OR k.expires_at > now())
               AND u.disabled_at IS NULL
             LIMIT 1",
            [token_hash.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows.into_iter().next().map(|r| ApiKey {
            id: ApiKeyId(r.id),
            workspace_id: crate::persistence::repos::identity::WorkspaceId(r.workspace_id),
            created_by_user_id: crate::persistence::repos::identity::UserId(r.created_by_user_id),
            name: r.name,
            token_hash: r.token_hash,
            expires_at: r.expires_at,
            last_used_at: r.last_used_at,
            revoked_at: r.revoked_at,
            created_at: r.created_at,
        }))
    }

    async fn revoke(&self, ctx: &WorkspaceCtx, id: ApiKeyId) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = api_key::Entity::find_by_id(id.0)
            .filter(api_key::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "api_key",
                id: id.0,
            })?;
        let mut active = row.into_active_model();
        active.revoked_at = Set(Some(Utc::now()));
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<ApiKey>, DomainError> {
        api_key::Entity::find()
            .filter(api_key::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(api_key::Column::RevokedAt.is_null())
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(api_key_from).collect())
            .map_err(db_err)
    }
}

impl PgApiKeyRepo {
    /// Updates `last_used_at = now()` for the given api key, throttled to at most
    /// once per 60 seconds (same debounce the session `touch` uses).
    pub async fn touch(&self, id: ApiKeyId) -> Result<(), DomainError> {
        use sea_orm::ConnectionTrait;
        self.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE api_keys
                 SET last_used_at = now()
                 WHERE id = $1
                   AND (last_used_at IS NULL OR last_used_at < now() - interval '60 seconds')",
                [id.0.into()],
            ))
            .await
            .map_err(db_err)?;
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
        membership::Entity::delete_many()
            .filter(membership::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(membership::Column::UserId.eq(user_id.0))
            .exec(&self.conn)
            .await
            .map(|_| ())
            .map_err(db_err)
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
