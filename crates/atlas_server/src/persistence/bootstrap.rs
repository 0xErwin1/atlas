use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{MemberRole, NewUser, NewWorkspace},
    ids::{UserId, WorkspaceId},
};
use sea_orm::DatabaseConnection;

use crate::persistence::repos::{
    MembershipRepo, PgMembershipRepo, PgUserRepo, PgWorkspaceRepo, UserRepo, WorkspaceRepo,
};

pub struct BootstrapConfig {
    pub root_password: Option<String>,
}

pub async fn run_bootstrap(cfg: &BootstrapConfig, conn: &DatabaseConnection) -> Result<(), String> {
    let user_repo = PgUserRepo { conn: conn.clone() };
    let ws_repo = PgWorkspaceRepo { conn: conn.clone() };
    let membership_repo = PgMembershipRepo { conn: conn.clone() };

    if user_repo
        .find_root()
        .await
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(());
    }

    let password = cfg.root_password.as_deref().ok_or_else(|| {
        "ATLAS_ROOT_PASSWORD is required on first boot but was not set".to_string()
    })?;

    let password_hash = hash_password(password)?;

    let workspace_id = WorkspaceId::new();
    let root_user_id = UserId::new();

    let root = user_repo
        .create(NewUser {
            username: "root".to_string(),
            display_name: "Root".to_string(),
            password_hash,
            is_root: true,
        })
        .await
        .map_err(|e| e.to_string())?;

    let ws = ws_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: "Atlas".to_string(),
            slug: "atlas".to_string(),
        })
        .await
        .map_err(|e| e.to_string())?;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(root.id));
    membership_repo
        .add(&ctx, root.id, MemberRole::Owner)
        .await
        .map_err(|e| e.to_string())?;

    let _ = root_user_id;

    Ok(())
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("password hashing failed: {e}"))
}
