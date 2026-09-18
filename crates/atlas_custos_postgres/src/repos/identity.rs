use async_trait::async_trait;
use atlas_core::Attribution;
use atlas_core::error::DomainError;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::UserId;
use atlas_custos::capability::Capability;
use atlas_custos::entities::identity::ApiKeyKind;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_custos::entities::principals::NewPrincipal;
use atlas_custos::entities::principals::PrincipalKind;
use atlas_custos::ids::ActivationTokenId;
use atlas_custos::ids::PrincipalId;
use atlas_custos::ids::SessionId;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, FromQueryResult, QueryFilter, Statement, TransactionTrait,
};

use crate::entities::identity::{
    activation_token, activation_token_from, api_key, api_key_from, capabilities_from_stored,
    capabilities_to_stored, principal_kind_from, session, session_from, user, user_from,
};
use crate::entities::principals::principal;
use crate::repos::principals::PgPrincipalRepo;
use atlas_postgres::db_err;

pub use atlas_custos::entities::identity::ActivationToken;
pub use atlas_custos::entities::identity::ApiKey;
pub use atlas_custos::entities::identity::NewActivationToken;
pub use atlas_custos::entities::identity::NewApiKey;
pub use atlas_custos::entities::identity::NewSession;
pub use atlas_custos::entities::identity::NewUser;
pub use atlas_custos::entities::identity::Session;
pub use atlas_custos::entities::identity::User;

pub use atlas_custos::ports::identity::ActivationTokenRepo;
pub use atlas_custos::ports::identity::ApiKeyRepo;
pub use atlas_custos::ports::identity::SessionRepo;
pub use atlas_custos::ports::identity::UserRepo;

pub struct PgUserRepo {
    pub conn: DatabaseConnection,
}

/// Inserts the `user` principal backing a new user row on the caller's
/// connection: a user's principal identity is the user row itself
/// (`principals.id = users.id`), so both rows commit or roll back together.
async fn create_user_principal_in<C: ConnectionTrait>(
    conn: &C,
    user_id: UserId,
    display_name: &str,
) -> Result<(), DomainError> {
    PgPrincipalRepo::create_in(
        conn,
        NewPrincipal {
            id: PrincipalId::from(user_id),
            kind: PrincipalKind::User,
            display_name: display_name.to_string(),
            deactivated_at: None,
        },
    )
    .await
    .map(|_| ())
}

/// Moves the principal mirror onto the user row's current `display_name` /
/// `disabled_at` values on the caller's connection. `deactivated_at` is
/// written unconditionally so the mirror stays exactly equal to
/// `users.disabled_at` after every path that touches either column.
async fn sync_user_principal_in<C: ConnectionTrait>(
    conn: &C,
    principal_id: uuid::Uuid,
    display_name: Option<String>,
    deactivated_at: Option<chrono::DateTime<Utc>>,
) -> Result<(), DomainError> {
    let mut active = principal::ActiveModel {
        id: Set(principal_id),
        ..std::default::Default::default()
    };
    if let Some(name) = display_name {
        active.display_name = Set(name);
    }
    active.deactivated_at = Set(deactivated_at);
    active.updated_at = Set(Utc::now());
    active.update(conn).await.map_err(db_err)?;
    Ok(())
}

#[async_trait]
impl UserRepo for PgUserRepo {
    async fn create(&self, new: NewUser) -> Result<User, DomainError> {
        let uid = UserId::new();
        let model = user::ActiveModel {
            id: Set(uid.0),
            username: Set(new.username),
            display_name: Set(new.display_name.clone()),
            email: Set(new.email),
            password_hash: Set(new.password_hash),
            is_root: Set(new.is_root),
            is_system_admin: Set(new.is_system_admin),
            disabled_at: Set(None),
            activated_at: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            principal_id: Set(uid.0),
        };
        let txn = self.conn.begin().await.map_err(db_err)?;
        create_user_principal_in(&txn, uid, &new.display_name).await?;
        let user = model.insert(&txn).await.map(user_from).map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;
        Ok(user)
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<User>, DomainError> {
        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            username: String,
            display_name: String,
            email: Option<String>,
            password_hash: Option<String>,
            is_root: bool,
            is_system_admin: bool,
            disabled_at: Option<chrono::DateTime<Utc>>,
            activated_at: Option<chrono::DateTime<Utc>>,
            created_at: chrono::DateTime<Utc>,
            updated_at: chrono::DateTime<Utc>,
        }

        let lower = username.to_lowercase();
        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT id, username, display_name, email, password_hash, is_root, is_system_admin, \
                    disabled_at, activated_at, created_at, updated_at \
             FROM custos.users WHERE lower(username) = $1 LIMIT 1",
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
            is_system_admin: r.is_system_admin,
            disabled_at: r.disabled_at,
            activated_at: r.activated_at,
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

    async fn list_by_ids(&self, ids: &[UserId]) -> Result<Vec<User>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let raw: Vec<uuid::Uuid> = ids.iter().map(|id| id.0).collect();

        user::Entity::find()
            .filter(user::Column::Id.is_in(raw))
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
        let principal_id = row.principal_id;
        let now = Utc::now();
        let mut active = row.into_active_model();
        active.disabled_at = Set(Some(now));
        active.updated_at = Set(now);
        let txn = self.conn.begin().await.map_err(db_err)?;
        active.update(&txn).await.map_err(db_err)?;
        sync_user_principal_in(&txn, principal_id, None, Some(now)).await?;
        txn.commit().await.map_err(db_err)?;
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
        let principal_id = row.principal_id;
        let now = Utc::now();
        let mut active = row.into_active_model();
        active.disabled_at = Set(None);
        active.updated_at = Set(now);
        let txn = self.conn.begin().await.map_err(db_err)?;
        active.update(&txn).await.map_err(db_err)?;
        sync_user_principal_in(&txn, principal_id, None, None).await?;
        txn.commit().await.map_err(db_err)?;
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
        active.password_hash = Set(Some(hash));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn activate(&self, id: UserId, password_hash: String) -> Result<User, DomainError> {
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
        active.password_hash = Set(Some(password_hash));
        active.activated_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(user_from)
            .map_err(db_err)
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

        let principal_id = row.principal_id;
        let current_disabled_at = row.disabled_at;
        let mut active = row.into_active_model();

        if let Some(email) = email {
            active.email = Set(Some(email));
        }
        let renamed = display_name.clone();
        if let Some(display_name) = display_name {
            active.display_name = Set(display_name);
        }
        active.updated_at = Set(Utc::now());

        let txn = self.conn.begin().await.map_err(db_err)?;
        let user = active.update(&txn).await.map(user_from).map_err(db_err)?;
        if let Some(name) = renamed {
            sync_user_principal_in(&txn, principal_id, Some(name), current_disabled_at).await?;
        }
        txn.commit().await.map_err(db_err)?;
        Ok(user)
    }

    async fn set_system_admin(
        &self,
        id: UserId,
        is_system_admin: bool,
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
        active.is_system_admin = Set(is_system_admin);
        active.updated_at = Set(Utc::now());

        active
            .update(&self.conn)
            .await
            .map(user_from)
            .map_err(db_err)
    }
}

impl PgUserRepo {
    /// Disables the given user using the provided connection or transaction.
    ///
    /// Used when the mutation must be atomic with an audit-log append inside
    /// an existing transaction.
    pub async fn disable_in<C: ConnectionTrait>(conn: &C, id: UserId) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;
        let principal_id = row.principal_id;
        let now = Utc::now();
        let mut active = row.into_active_model();
        active.disabled_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(conn).await.map_err(db_err)?;
        sync_user_principal_in(conn, principal_id, None, Some(now)).await
    }

    /// Enables the given user using the provided connection or transaction.
    pub async fn enable_in<C: ConnectionTrait>(conn: &C, id: UserId) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;
        let principal_id = row.principal_id;
        let now = Utc::now();
        let mut active = row.into_active_model();
        active.disabled_at = Set(None);
        active.updated_at = Set(now);
        active.update(conn).await.map_err(db_err)?;
        sync_user_principal_in(conn, principal_id, None, None).await
    }

    /// Updates `password_hash` using the provided connection or transaction.
    pub async fn set_password_hash_in<C: ConnectionTrait>(
        conn: &C,
        id: UserId,
        hash: String,
    ) -> Result<(), DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;
        let mut active = row.into_active_model();
        active.password_hash = Set(Some(hash));
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;
        Ok(())
    }

    /// Updates `is_system_admin` using the provided connection or transaction.
    pub async fn set_system_admin_in<C: ConnectionTrait>(
        conn: &C,
        id: UserId,
        is_system_admin: bool,
    ) -> Result<User, DomainError> {
        use sea_orm::IntoActiveModel;
        let row = user::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "user",
                id: id.0,
            })?;
        let mut active = row.into_active_model();
        active.is_system_admin = Set(is_system_admin);
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map(user_from).map_err(db_err)
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
                "UPDATE custos.sessions SET revoked_at = now()
                 WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()",
                [user_id.0.into()],
            ))
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_all_for_user_except(
        &self,
        user_id: UserId,
        keep_session_id: SessionId,
    ) -> Result<(), DomainError> {
        use sea_orm::ConnectionTrait;
        self.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE custos.sessions SET revoked_at = now()
                 WHERE user_id = $1 AND id <> $2
                   AND revoked_at IS NULL AND expires_at > now()",
                [user_id.0.into(), keep_session_id.0.into()],
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
                "UPDATE custos.sessions
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

/// Shared tail of the api-key revoke paths: stamps `revoked_at` on the key
/// row and deactivates its agent principal on the caller's connection, so both
/// rows move in one transaction. The caller has already applied its own
/// authorization/ownership filter to `row`.
async fn revoke_api_key_row_in<C: ConnectionTrait>(
    conn: &C,
    row: api_key::Model,
    principal_kind: PrincipalKind,
) -> Result<ApiKey, DomainError> {
    use sea_orm::IntoActiveModel;

    let key_snapshot = api_key_from(row.clone(), principal_kind);
    let principal_id = row.principal_id;
    let now = Utc::now();

    let mut active = row.into_active_model();
    active.revoked_at = Set(Some(now));
    active.update(conn).await.map_err(db_err)?;

    // The agent principal mirrors the key's lifetime: revoking the key
    // deactivates the principal, in the same caller transaction.
    let principal_active = principal::ActiveModel {
        id: Set(principal_id),
        deactivated_at: Set(Some(now)),
        updated_at: Set(now),
        ..std::default::Default::default()
    };
    principal_active.update(conn).await.map_err(db_err)?;

    Ok(key_snapshot)
}

pub struct PgApiKeyRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl ApiKeyRepo for PgApiKeyRepo {
    async fn create(
        &self,
        scope: atlas_custos::WorkspaceScope,
        created_by: &Attribution,
        new: NewApiKey,
    ) -> Result<ApiKey, DomainError> {
        let created_by_user_id = match created_by {
            Attribution::User(uid) => uid.0,
            Attribution::ApiKey(_) => {
                return Err(DomainError::InvalidInput {
                    message: "api keys must be created by a user actor".into(),
                });
            }
        };
        let principal_id = PrincipalId::new();
        let model = api_key::ActiveModel {
            id: Set(ApiKeyId::new().0),
            workspace_id: Set(Some(scope.0)),
            created_by_user_id: Set(created_by_user_id),
            name: Set(new.name.clone()),
            token_hash: Set(new.token_hash),
            type_: Set(new.type_.as_str().to_string()),
            expires_at: Set(new.expires_at),
            last_used_at: Set(None),
            revoked_at: Set(None),
            created_at: Set(Utc::now()),
            is_global: Set(false),
            scopes: Set(capabilities_to_stored(&new.scopes)),
            principal_id: Set(principal_id.0),
        };
        let txn = self.conn.begin().await.map_err(db_err)?;
        PgApiKeyRepo::create_agent_principal_in(&txn, principal_id, &new.name).await?;
        let key = model
            .insert(&txn)
            .await
            .map(|m| api_key_from(m, PrincipalKind::Agent))
            .map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;
        Ok(key)
    }

    async fn create_for_user(
        &self,
        user_id: UserId,
        new: NewApiKey,
    ) -> Result<ApiKey, DomainError> {
        let principal_id = PrincipalId::new();
        let model = api_key::ActiveModel {
            id: Set(ApiKeyId::new().0),
            workspace_id: Set(None),
            created_by_user_id: Set(user_id.0),
            name: Set(new.name.clone()),
            token_hash: Set(new.token_hash),
            type_: Set(new.type_.as_str().to_string()),
            expires_at: Set(new.expires_at),
            last_used_at: Set(None),
            revoked_at: Set(None),
            created_at: Set(Utc::now()),
            is_global: Set(false),
            scopes: Set(capabilities_to_stored(&new.scopes)),
            principal_id: Set(principal_id.0),
        };
        let txn = self.conn.begin().await.map_err(db_err)?;
        PgApiKeyRepo::create_agent_principal_in(&txn, principal_id, &new.name).await?;
        let key = model
            .insert(&txn)
            .await
            .map(|m| api_key_from(m, PrincipalKind::Agent))
            .map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;
        Ok(key)
    }

    async fn find_active_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiKey>, DomainError> {
        #[derive(Debug, sea_orm::FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            workspace_id: Option<uuid::Uuid>,
            created_by_user_id: uuid::Uuid,
            name: String,
            token_hash: String,
            type_: String,
            expires_at: Option<chrono::DateTime<Utc>>,
            last_used_at: Option<chrono::DateTime<Utc>>,
            revoked_at: Option<chrono::DateTime<Utc>>,
            created_at: chrono::DateTime<Utc>,
            is_global: bool,
            scopes: Vec<String>,
            principal_kind: String,
        }

        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT k.id, k.workspace_id, k.created_by_user_id, k.name, k.token_hash,
                    k.type AS type_, k.expires_at, k.last_used_at, k.revoked_at, k.created_at,
                    k.is_global, k.scopes, p.kind AS principal_kind
             FROM custos.api_keys k
             JOIN custos.users u ON u.id = k.created_by_user_id
             JOIN custos.principals p ON p.id = k.principal_id
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

        rows.into_iter()
            .next()
            .map(|r| {
                Ok(ApiKey {
                    id: ApiKeyId(r.id),
                    workspace_id: r.workspace_id.map(atlas_custos::WorkspaceScope),
                    created_by_user_id: UserId(r.created_by_user_id),
                    name: r.name,
                    token_hash: r.token_hash,
                    type_: r.type_.parse::<ApiKeyType>().unwrap_or_default(),
                    expires_at: r.expires_at,
                    last_used_at: r.last_used_at,
                    revoked_at: r.revoked_at,
                    created_at: r.created_at,
                    is_global: r.is_global,
                    scopes: capabilities_from_stored(&r.scopes),
                    principal_kind: r.principal_kind.parse::<PrincipalKind>().map_err(|e| {
                        DomainError::InvalidInput {
                            message: format!("stored principal kind is invalid: {e}"),
                        }
                    })?,
                })
            })
            .transpose()
    }

    async fn list(&self, scope: atlas_custos::WorkspaceScope) -> Result<Vec<ApiKey>, DomainError> {
        api_key::Entity::find()
            .filter(api_key::Column::WorkspaceId.eq(scope.0))
            .filter(api_key::Column::RevokedAt.is_null())
            .find_also_related(principal::Entity)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|(k, p)| Ok(api_key_from(k, principal_kind_from(&p)?)))
            .collect()
    }

    async fn list_for_user(&self, user_id: UserId) -> Result<Vec<ApiKey>, DomainError> {
        api_key::Entity::find()
            .filter(api_key::Column::CreatedByUserId.eq(user_id.0))
            .filter(api_key::Column::RevokedAt.is_null())
            .find_also_related(principal::Entity)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|(k, p)| Ok(api_key_from(k, principal_kind_from(&p)?)))
            .collect()
    }

    async fn get_by_id(&self, id: ApiKeyId) -> Result<Option<ApiKey>, DomainError> {
        api_key::Entity::find_by_id(id.0)
            .find_also_related(principal::Entity)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(|(k, p)| Ok(api_key_from(k, principal_kind_from(&p)?)))
            .transpose()
    }

    async fn list_by_ids(&self, ids: &[ApiKeyId]) -> Result<Vec<ApiKey>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let raw: Vec<uuid::Uuid> = ids.iter().map(|id| id.0).collect();

        api_key::Entity::find()
            .filter(api_key::Column::Id.is_in(raw))
            .find_also_related(principal::Entity)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|(k, p)| Ok(api_key_from(k, principal_kind_from(&p)?)))
            .collect()
    }

    async fn list_granted_in_workspace(
        &self,
        scope: atlas_custos::WorkspaceScope,
    ) -> Result<Vec<ApiKey>, DomainError> {
        #[derive(Debug, sea_orm::FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            workspace_id: Option<uuid::Uuid>,
            created_by_user_id: uuid::Uuid,
            name: String,
            token_hash: String,
            type_: String,
            expires_at: Option<chrono::DateTime<Utc>>,
            last_used_at: Option<chrono::DateTime<Utc>>,
            revoked_at: Option<chrono::DateTime<Utc>>,
            created_at: chrono::DateTime<Utc>,
            is_global: bool,
            scopes: Vec<String>,
            principal_kind: String,
        }

        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT DISTINCT k.id, k.workspace_id, k.created_by_user_id, k.name, k.token_hash,
                    k.type AS type_, k.expires_at, k.last_used_at, k.revoked_at, k.created_at,
                    k.is_global, k.scopes, p.kind AS principal_kind
             FROM custos.api_keys k
             JOIN custos.principals p ON p.id = k.principal_id
             JOIN custos.permission_grants g ON g.api_key_id = k.id
             WHERE g.workspace_id = $1
               AND k.revoked_at IS NULL
             ORDER BY k.created_at",
            [scope.0.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        rows.into_iter()
            .map(|r| {
                Ok(ApiKey {
                    id: ApiKeyId(r.id),
                    workspace_id: r.workspace_id.map(atlas_custos::WorkspaceScope),
                    created_by_user_id: UserId(r.created_by_user_id),
                    name: r.name,
                    token_hash: r.token_hash,
                    type_: r.type_.parse::<ApiKeyType>().unwrap_or_default(),
                    expires_at: r.expires_at,
                    last_used_at: r.last_used_at,
                    revoked_at: r.revoked_at,
                    created_at: r.created_at,
                    is_global: r.is_global,
                    scopes: capabilities_from_stored(&r.scopes),
                    principal_kind: r.principal_kind.parse::<PrincipalKind>().map_err(|e| {
                        DomainError::InvalidInput {
                            message: format!("stored principal kind is invalid: {e}"),
                        }
                    })?,
                })
            })
            .collect()
    }
}

impl PgApiKeyRepo {
    /// Inserts the `agent` principal backing a new api key row on the caller's
    /// connection, so the principal and its key commit or roll back together.
    /// Agent principals get a fresh id (unlike users, whose principal is the
    /// user row itself); the key row links to it via `api_keys.principal_id`.
    async fn create_agent_principal_in<C: ConnectionTrait>(
        conn: &C,
        principal_id: PrincipalId,
        name: &str,
    ) -> Result<(), DomainError> {
        PgPrincipalRepo::create_in(
            conn,
            NewPrincipal {
                id: principal_id,
                kind: PrincipalKind::Agent,
                display_name: name.to_string(),
                deactivated_at: None,
            },
        )
        .await
        .map(|_| ())
    }

    /// Creates a user-owned API key using the provided connection or transaction.
    /// Agent is the default credential kind: a fresh agent principal is minted.
    ///
    /// Used when the insert must be atomic with an audit-log append inside an
    /// existing transaction.
    pub async fn create_for_user_in<C: ConnectionTrait>(
        conn: &C,
        user_id: UserId,
        new: NewApiKey,
    ) -> Result<ApiKey, DomainError> {
        Self::create_for_user_in_with_kind(conn, user_id, ApiKeyKind::Agent, new).await
    }

    /// Creates a user-owned API key of the given credential kind using the
    /// provided connection or transaction.
    ///
    /// A `Personal` key links to the owner's existing user principal
    /// (`custos.principals`, kind `user`) — no new principal row is written —
    /// while an `Agent` key keeps today's fresh-agent-principal behaviour. The
    /// caller derives the token prefix from the same `key_kind` value, so the
    /// prefix and the principal linkage cannot disagree.
    pub async fn create_for_user_in_with_kind<C: ConnectionTrait>(
        conn: &C,
        user_id: UserId,
        key_kind: ApiKeyKind,
        new: NewApiKey,
    ) -> Result<ApiKey, DomainError> {
        let (principal_id, principal_kind) = match key_kind {
            ApiKeyKind::Personal => {
                let principal = principal::Entity::find_by_id(user_id.0)
                    .one(conn)
                    .await
                    .map_err(db_err)?
                    .ok_or(DomainError::NotFound {
                        entity: "principal",
                        id: user_id.0,
                    })?;
                let kind = principal.kind.parse::<PrincipalKind>().map_err(|e| {
                    DomainError::InvalidInput {
                        message: format!("stored principal kind is invalid: {e}"),
                    }
                })?;
                if kind != PrincipalKind::User {
                    return Err(DomainError::InvalidInput {
                        message: "personal keys require the owner's user principal".into(),
                    });
                }
                (principal.id, PrincipalKind::User)
            }
            ApiKeyKind::Agent => {
                let principal_id = PrincipalId::new();
                PgApiKeyRepo::create_agent_principal_in(conn, principal_id, &new.name).await?;
                (principal_id.0, PrincipalKind::Agent)
            }
        };
        let model = api_key::ActiveModel {
            id: Set(ApiKeyId::new().0),
            workspace_id: Set(None),
            created_by_user_id: Set(user_id.0),
            name: Set(new.name.clone()),
            token_hash: Set(new.token_hash),
            type_: Set(new.type_.as_str().to_string()),
            expires_at: Set(new.expires_at),
            last_used_at: Set(None),
            revoked_at: Set(None),
            created_at: Set(Utc::now()),
            is_global: Set(false),
            scopes: Set(capabilities_to_stored(&new.scopes)),
            principal_id: Set(principal_id),
        };
        model
            .insert(conn)
            .await
            .map(|m| api_key_from(m, principal_kind))
            .map_err(db_err)
    }

    /// Replaces the full scope set on a user-owned key using the provided
    /// connection or transaction, so the update and its audit append share one
    /// transaction. Mirrors `set_global_for_user_in`'s full-replacement semantics.
    ///
    /// Scoped to `user_id`: returns `DomainError::NotFound` when the key does not
    /// exist or is owned by someone else, so a non-owner cannot probe key existence.
    pub async fn set_scopes_for_user_in<C: ConnectionTrait>(
        conn: &C,
        user_id: UserId,
        id: ApiKeyId,
        scopes: Vec<Capability>,
    ) -> Result<ApiKey, DomainError> {
        use sea_orm::IntoActiveModel;

        let (key, principal) = api_key::Entity::find_by_id(id.0)
            .filter(api_key::Column::CreatedByUserId.eq(user_id.0))
            .find_also_related(principal::Entity)
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "api_key",
                id: id.0,
            })?;
        let principal_kind = principal_kind_from(&principal)?;

        let mut active = key.into_active_model();
        active.scopes = Set(capabilities_to_stored(&scopes));

        active
            .update(conn)
            .await
            .map(|m| api_key_from(m, principal_kind))
            .map_err(db_err)
    }

    /// Sets the `is_global` flag on a user-owned key using the provided connection
    /// or transaction, so the update and its audit append share one transaction.
    ///
    /// Scoped to `user_id`: returns `DomainError::NotFound` when the key does not
    /// exist or is owned by someone else, so a non-owner cannot probe key existence.
    pub async fn set_global_for_user_in<C: ConnectionTrait>(
        conn: &C,
        user_id: UserId,
        id: ApiKeyId,
        is_global: bool,
    ) -> Result<ApiKey, DomainError> {
        use sea_orm::IntoActiveModel;

        let (key, principal) = api_key::Entity::find_by_id(id.0)
            .filter(api_key::Column::CreatedByUserId.eq(user_id.0))
            .find_also_related(principal::Entity)
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "api_key",
                id: id.0,
            })?;
        let principal_kind = principal_kind_from(&principal)?;

        let mut active = key.into_active_model();
        active.is_global = Set(is_global);

        active
            .update(conn)
            .await
            .map(|m| api_key_from(m, principal_kind))
            .map_err(db_err)
    }

    /// Marks a user-owned API key revoked using the provided connection or
    /// transaction. Returns the key record as it existed before the revoke,
    /// for use in audit event metadata.
    ///
    /// This performs only the Custos-owned row update. It intentionally does
    /// not touch `task_assignees` — that table is Acta-owned, and Acta code no
    /// longer lives in this crate (D4). The caller composes the task-assignee
    /// cleanup in the same transaction: see
    /// `routes::api_keys::revoke_user_api_key` in `atlas_server`, which pairs
    /// this call with `PgTaskAssigneeRepo::unassign_api_key_in` before commit.
    pub async fn revoke_for_user_in<C: ConnectionTrait>(
        conn: &C,
        user_id: UserId,
        id: ApiKeyId,
    ) -> Result<ApiKey, DomainError> {
        let Some((row, principal)) = api_key::Entity::find_by_id(id.0)
            .filter(api_key::Column::RevokedAt.is_null())
            .find_also_related(principal::Entity)
            .one(conn)
            .await
            .map_err(db_err)?
        else {
            return Err(DomainError::NotFound {
                entity: "api_key",
                id: id.0,
            });
        };

        if row.created_by_user_id != user_id.0 {
            return Err(DomainError::Forbidden {
                message: "api key is not owned by this user".into(),
            });
        }

        revoke_api_key_row_in(conn, row, principal_kind_from(&principal)?).await
    }

    /// Revokes an api key by id, regardless of owner, using the provided
    /// connection or transaction. This is the composition-layer path for
    /// callers that hold a key reference without its owner's attribution —
    /// e.g. an integration config revoking its provisioned key — so every
    /// revoke shares the adapter's principal-mirror discipline instead of
    /// raw-updating `custos.api_keys` directly.
    ///
    /// Returns the key as it existed before the revoke, or `Ok(None)` when the
    /// key is missing or already revoked: the idempotent-update semantics of
    /// the raw statement this method replaced.
    pub async fn revoke_by_id_in<C: ConnectionTrait>(
        conn: &C,
        id: ApiKeyId,
    ) -> Result<Option<ApiKey>, DomainError> {
        let Some((row, principal)) = api_key::Entity::find_by_id(id.0)
            .filter(api_key::Column::RevokedAt.is_null())
            .find_also_related(principal::Entity)
            .one(conn)
            .await
            .map_err(db_err)?
        else {
            return Ok(None);
        };

        let principal_kind = principal_kind_from(&principal)?;
        revoke_api_key_row_in(conn, row, principal_kind)
            .await
            .map(Some)
    }

    /// Updates `last_used_at = now()` for the given api key, throttled to at most
    /// once per 60 seconds (same debounce the session `touch` uses).
    pub async fn touch(&self, id: ApiKeyId) -> Result<(), DomainError> {
        use sea_orm::ConnectionTrait;
        self.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE custos.api_keys
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

pub struct PgActivationTokenRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl ActivationTokenRepo for PgActivationTokenRepo {
    async fn create(&self, new: NewActivationToken) -> Result<ActivationToken, DomainError> {
        let model = activation_token::ActiveModel {
            id: Set(ActivationTokenId::new().0),
            user_id: Set(new.user_id.0),
            token_hash: Set(new.token_hash),
            expires_at: Set(new.expires_at),
            consumed_at: Set(None),
            created_at: Set(Utc::now()),
        };
        model
            .insert(&self.conn)
            .await
            .map(activation_token_from)
            .map_err(db_err)
    }

    async fn find_active_by_token_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ActivationToken>, DomainError> {
        activation_token::Entity::find()
            .filter(activation_token::Column::TokenHash.eq(hash))
            .filter(activation_token::Column::ConsumedAt.is_null())
            .filter(activation_token::Column::ExpiresAt.gt(Utc::now()))
            .one(&self.conn)
            .await
            .map(|opt| opt.map(activation_token_from))
            .map_err(db_err)
    }

    async fn consume(&self, id: ActivationTokenId) -> Result<(), DomainError> {
        // Guard on `consumed_at IS NULL` so this can never double-consume a
        // token under a race. The production activate path uses its own guarded
        // SQL; this trait method (test-only callers) is aligned to the same
        // invariant. A missing or already-consumed token is a NotFound.
        let result = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE custos.user_activation_tokens \
                 SET consumed_at = $1 \
                 WHERE id = $2 AND consumed_at IS NULL",
                [Utc::now().into(), id.0.into()],
            ))
            .await
            .map_err(db_err)?;

        if result.rows_affected() == 0 {
            return Err(DomainError::NotFound {
                entity: "activation_token",
                id: id.0,
            });
        }

        Ok(())
    }

    async fn invalidate_unconsumed_for_user(&self, user_id: UserId) -> Result<(), DomainError> {
        self.conn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE custos.user_activation_tokens \
                 SET consumed_at = now() \
                 WHERE user_id = $1 AND consumed_at IS NULL",
                [user_id.0.into()],
            ))
            .await
            .map_err(db_err)?;
        Ok(())
    }
}
