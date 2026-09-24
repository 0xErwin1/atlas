use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::identity::MemberRole;
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::entities::workspace_core::NewProject;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::permissions::{ResourceRef, Visibility, VisibilityRole, resource_ref_codec};
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos::ids::PrincipalId;
use sea_orm::{DatabaseConnection, TransactionTrait};

use crate::auth::password;
use crate::authz::v2_access;
use crate::persistence::repos::{PgProjectRepo, ProjectRepo, UserRepo};
use atlas_acta_postgres::repos::identity::{
    MembershipRepo, PgMembershipRepo, PgWorkspaceRepo, WorkspaceRepo,
};
use atlas_custos_postgres::repos::identity::PgUserRepo;

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

    let ctx = WorkspaceCtx::new(
        ws.id,
        Actor::User(atlas_acta::actor::UserAttributionId(root.id.0)),
    );
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

    let ctx = WorkspaceCtx::new(
        ws.id,
        Actor::User(atlas_acta::actor::UserAttributionId(root.id.0)),
    );

    let existing = project_repo
        .find_by_slug(&ctx, "sandbox")
        .await
        .map_err(|e| e.to_string())?;

    if existing.is_none() {
        seed_sandbox_project(conn, &ctx, root.id).await?;
    }

    Ok(())
}

/// Creates the dev `Sandbox` project, visible to every member as editor,
/// together with the members-set grant that visibility stands for in V2, in
/// one transaction.
async fn seed_sandbox_project(
    conn: &DatabaseConnection,
    ctx: &WorkspaceCtx,
    root: UserId,
) -> Result<(), String> {
    let visibility = Visibility::Workspace(VisibilityRole::Editor);

    let txn = conn.begin().await.map_err(|e| e.to_string())?;
    let project = PgProjectRepo::create_in(
        &txn,
        ctx,
        NewProject {
            name: "Sandbox".to_string(),
            slug: "sandbox".to_string(),
            task_prefix: "SBX".to_string(),
            visibility: visibility.clone(),
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    v2_access::visibility_written(
        &txn,
        ctx.workspace_id,
        resource_ref_codec::to_core(&ResourceRef::Project(project.id), ctx.workspace_id),
        &visibility,
        PrincipalId::from(root),
    )
    .await
    .map_err(|e| format!("{e:?}"))?;

    txn.commit().await.map_err(|e| e.to_string())
}
