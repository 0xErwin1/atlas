use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{MemberRole, NewUser, NewWorkspace},
    entities::workspace_core::NewProject,
    ids::{UserId, WorkspaceId},
};
use sea_orm::DatabaseConnection;

use crate::auth::password;
use crate::persistence::repos::{
    MembershipRepo, PgMembershipRepo, PgProjectRepo, PgUserRepo, PgWorkspaceRepo, ProjectRepo,
    UserRepo, WorkspaceRepo,
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

    let password_hash = password::hash(password.to_string())
        .await
        .map_err(|e| e.to_string())?;

    let workspace_id = WorkspaceId::new();
    let root_user_id = UserId::new();

    // Keep the hash in scope so we can pass it to activate().
    let root = user_repo
        .create(NewUser {
            username: "root".to_string(),
            display_name: "Root".to_string(),
            email: None,
            password_hash: Some(password_hash.clone()),
            is_root: true,
            is_system_admin: false,
        })
        .await
        .map_err(|e| e.to_string())?;

    // Root is created by the system administrator, not via the invitation flow,
    // so activate immediately using the same hash.
    user_repo
        .activate(root.id, password_hash)
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

pub async fn run_dev_seed(cfg: &BootstrapConfig, conn: &DatabaseConnection) -> Result<(), String> {
    run_bootstrap(cfg, conn).await?;

    let user_repo = PgUserRepo { conn: conn.clone() };
    let ws_repo = PgWorkspaceRepo { conn: conn.clone() };
    let project_repo = PgProjectRepo { conn: conn.clone() };

    let root = user_repo
        .find_root()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "root user must exist after bootstrap".to_string())?;

    let workspaces = ws_repo
        .list_for_user(root.id)
        .await
        .map_err(|e| e.to_string())?;

    let ws = workspaces
        .into_iter()
        .next()
        .ok_or_else(|| "root workspace must exist after bootstrap".to_string())?;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(root.id));

    let existing = project_repo
        .find_by_slug(&ctx, "sandbox")
        .await
        .map_err(|e| e.to_string())?;

    if existing.is_none() {
        project_repo
            .create(
                &ctx,
                NewProject {
                    name: "Sandbox".to_string(),
                    slug: "sandbox".to_string(),
                    task_prefix: "SBX".to_string(),
                    visibility: atlas_domain::permissions::Visibility::Workspace(
                        atlas_domain::permissions::VisibilityRole::Editor,
                    ),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}
