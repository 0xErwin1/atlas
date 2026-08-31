use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::principal::UserId;
use sea_orm::{DatabaseConnection, EntityTrait, Statement};

use crate::persistence::entities::identity::{user_ui_state, user_ui_state_from};
use atlas_postgres::db_err;

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

// R1 scaffolding: the identity CRUD repos (`PgUserRepo`, `PgSessionRepo`,
// `PgApiKeyRepo`, `PgActivationTokenRepo`) now live in `atlas_custos_postgres`.
// Re-exporting them here keeps every existing `crate::persistence::repos::*`
// call site unaffected by the move.
pub use atlas_custos_postgres::repos::identity::{
    PgActivationTokenRepo, PgApiKeyRepo, PgSessionRepo, PgUserRepo,
};

// R1 scaffolding: `PgWorkspaceRepo`/`PgMembershipRepo` (and the
// `WorkspaceRepo`/`MembershipRepo`/`NewWorkspace`/`Workspace` types they use)
// now live in `atlas_acta_postgres::repos::identity` (S4 PR6). Re-exporting
// them here keeps every existing `crate::persistence::repos::*` call site
// unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::repos::identity::{
    MembershipRepo, NewWorkspace, PgMembershipRepo, PgWorkspaceRepo, Workspace, WorkspaceRepo,
};

pub use crate::platform::{UiStateRepo, UserUiState};

pub struct PgUiStateRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl UiStateRepo for PgUiStateRepo {
    async fn find(&self, user_id: UserId) -> Result<Option<UserUiState>, DomainError> {
        user_ui_state::Entity::find_by_id(user_id.0)
            .one(&self.conn)
            .await
            .map(|opt| opt.map(user_ui_state_from))
            .map_err(db_err)
    }

    async fn upsert(
        &self,
        user_id: UserId,
        state: serde_json::Value,
    ) -> Result<UserUiState, DomainError> {
        use sea_orm::ConnectionTrait;

        self.conn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                r#"
                INSERT INTO user_ui_state (user_id, state, updated_at)
                VALUES ($1, $2, now())
                ON CONFLICT (user_id)
                DO UPDATE SET state = EXCLUDED.state, updated_at = EXCLUDED.updated_at
                "#,
                [user_id.0.into(), state.into()],
            ))
            .await
            .map_err(db_err)?;

        user_ui_state::Entity::find_by_id(user_id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(user_ui_state_from)
            .ok_or(DomainError::Internal {
                message: "user_ui_state row missing after upsert".into(),
            })
    }
}
