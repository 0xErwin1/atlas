#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::{DeleteByIdProjection, GrantProjection};

const LIMIT_MIN: u32 = 1;
const LIMIT_MAX: u32 = 200;

// ---------------------------------------------------------------------------
// GrantsArgs (top-level) + GrantsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `grants` subcommand group.
#[derive(Args)]
pub(crate) struct GrantsArgs {
    #[command(subcommand)]
    pub(crate) command: GrantsCmd,
}

#[derive(Subcommand)]
pub(crate) enum GrantsCmd {
    /// Manage workspace-scoped permission grants.
    Workspace(WorkspaceGrantsArgs),
    /// Manage project-scoped permission grants.
    Project(ProjectGrantsArgs),
}

/// Dispatches a parsed `GrantsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: GrantsCmd) -> Result<(), CliError> {
    match cmd {
        GrantsCmd::Workspace(args) => run_workspace(ctx, args.command).await,
        GrantsCmd::Project(args) => run_project(ctx, args.command).await,
    }
}

// ---------------------------------------------------------------------------
// Workspace grant sub-commands
// ---------------------------------------------------------------------------

/// Arguments holder for the `grants workspace` sub-group.
#[derive(Args)]
pub(crate) struct WorkspaceGrantsArgs {
    #[command(subcommand)]
    pub(crate) command: WorkspaceGrantsCmd,
}

#[derive(Subcommand)]
pub(crate) enum WorkspaceGrantsCmd {
    /// List grants on a workspace.
    List(WorkspaceGrantsListArgs),
    /// Grant a principal access to a workspace.
    Create(WorkspaceGrantsCreateArgs),
    /// Revoke a workspace grant (requires --confirm).
    Revoke(WorkspaceGrantsRevokeArgs),
}

async fn run_workspace(ctx: &Ctx, cmd: WorkspaceGrantsCmd) -> Result<(), CliError> {
    match cmd {
        WorkspaceGrantsCmd::List(args) => run_workspace_list(ctx, args).await,
        WorkspaceGrantsCmd::Create(args) => run_workspace_create(ctx, args).await,
        WorkspaceGrantsCmd::Revoke(args) => run_workspace_revoke(ctx, args).await,
    }
}

// --- workspace list ----------------------------------------------------------

/// Arguments for `atlas grants workspace list`.
#[derive(Parser)]
pub(crate) struct WorkspaceGrantsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Maximum number of grants to return (clamped to 1..=200).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Pagination cursor returned by a previous list.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

async fn run_workspace_list(ctx: &Ctx, args: WorkspaceGrantsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let limit = args.limit.map(|l| l.clamp(LIMIT_MIN, LIMIT_MAX));

    let page = ctx
        .client
        .list_workspace_grants(ws, args.cursor.as_deref(), limit)
        .await?;

    let items: Vec<GrantProjection> = page.items.into_iter().map(GrantProjection::from).collect();
    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// --- workspace create --------------------------------------------------------

/// Arguments for `atlas grants workspace create`.
#[derive(Parser)]
pub(crate) struct WorkspaceGrantsCreateArgs {
    /// Principal type: `user` or `api_key`.
    #[arg(long)]
    pub(crate) principal_type: String,

    /// UUID of the principal to grant access to.
    #[arg(long)]
    pub(crate) principal_id: Uuid,

    /// Role to grant: `viewer` or `editor`.
    #[arg(long)]
    pub(crate) role: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_workspace_create(ctx: &Ctx, args: WorkspaceGrantsCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::{CreateGrantRequest, GrantPrincipal};

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: args.principal_type,
            id: args.principal_id,
        },
        role: args.role,
    };

    let grant = ctx.client.create_workspace_grant(ws, body).await?;
    let proj = GrantProjection::from(grant);
    output::emit(ctx.output, &proj)
}

// --- workspace revoke --------------------------------------------------------

/// Arguments for `atlas grants workspace revoke`.
#[derive(Parser)]
pub(crate) struct WorkspaceGrantsRevokeArgs {
    /// UUID of the grant to revoke.
    #[arg(long)]
    pub(crate) grant_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the revocation. Required — revocation is non-reversible.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_workspace_revoke(ctx: &Ctx, args: WorkspaceGrantsRevokeArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to revoke the grant (this removes the principal's access)".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client.delete_workspace_grant(ws, args.grant_id).await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.grant_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Project grant sub-commands
// ---------------------------------------------------------------------------

/// Arguments holder for the `grants project` sub-group.
#[derive(Args)]
pub(crate) struct ProjectGrantsArgs {
    #[command(subcommand)]
    pub(crate) command: ProjectGrantsCmd,
}

#[derive(Subcommand)]
pub(crate) enum ProjectGrantsCmd {
    /// List grants on a project.
    List(ProjectGrantsListArgs),
    /// Grant a principal access to a project.
    Create(ProjectGrantsCreateArgs),
    /// Revoke a project grant (requires --confirm).
    Revoke(ProjectGrantsRevokeArgs),
}

async fn run_project(ctx: &Ctx, cmd: ProjectGrantsCmd) -> Result<(), CliError> {
    match cmd {
        ProjectGrantsCmd::List(args) => run_project_list(ctx, args).await,
        ProjectGrantsCmd::Create(args) => run_project_create(ctx, args).await,
        ProjectGrantsCmd::Revoke(args) => run_project_revoke(ctx, args).await,
    }
}

// --- project list ------------------------------------------------------------

/// Arguments for `atlas grants project list`.
#[derive(Parser)]
pub(crate) struct ProjectGrantsListArgs {
    /// Project slug.
    #[arg(long)]
    pub(crate) project: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Maximum number of grants to return (clamped to 1..=200).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Pagination cursor returned by a previous list.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

async fn run_project_list(ctx: &Ctx, args: ProjectGrantsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let limit = args.limit.map(|l| l.clamp(LIMIT_MIN, LIMIT_MAX));

    let page = ctx
        .client
        .list_project_grants(ws, &args.project, args.cursor.as_deref(), limit)
        .await?;

    let items: Vec<GrantProjection> = page.items.into_iter().map(GrantProjection::from).collect();
    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// --- project create ----------------------------------------------------------

/// Arguments for `atlas grants project create`.
#[derive(Parser)]
pub(crate) struct ProjectGrantsCreateArgs {
    /// Project slug.
    #[arg(long)]
    pub(crate) project: String,

    /// Principal type: `user` or `api_key`.
    #[arg(long)]
    pub(crate) principal_type: String,

    /// UUID of the principal to grant access to.
    #[arg(long)]
    pub(crate) principal_id: Uuid,

    /// Role to grant: `viewer` or `editor`.
    #[arg(long)]
    pub(crate) role: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_project_create(ctx: &Ctx, args: ProjectGrantsCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::{CreateGrantRequest, GrantPrincipal};

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: args.principal_type,
            id: args.principal_id,
        },
        role: args.role,
    };

    let grant = ctx
        .client
        .create_project_grant(ws, &args.project, body)
        .await?;
    let proj = GrantProjection::from(grant);
    output::emit(ctx.output, &proj)
}

// --- project revoke ----------------------------------------------------------

/// Arguments for `atlas grants project revoke`.
#[derive(Parser)]
pub(crate) struct ProjectGrantsRevokeArgs {
    /// Project slug.
    #[arg(long)]
    pub(crate) project: String,

    /// UUID of the grant to revoke.
    #[arg(long)]
    pub(crate) grant_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the revocation. Required — revocation is non-reversible.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_project_revoke(ctx: &Ctx, args: ProjectGrantsRevokeArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to revoke the grant (this removes the principal's project access)"
                .to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client
        .delete_project_grant(ws, &args.project, args.grant_id)
        .await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.grant_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser as ClapParser;

    // -----------------------------------------------------------------------
    // Workspace grants parse tests
    // -----------------------------------------------------------------------

    #[test]
    fn workspace_list_parses_without_workspace() {
        let cli = Cli::try_parse_from(["atlas", "grants", "workspace", "list"]).unwrap();
        let crate::cli::Commands::Grants(grants) = cli.command else {
            panic!("expected Grants");
        };
        let GrantsCmd::Workspace(ws) = grants.command else {
            panic!("expected Workspace");
        };
        let WorkspaceGrantsCmd::List(list) = ws.command else {
            panic!("expected List");
        };
        assert!(list.workspace.is_none());
        assert!(list.limit.is_none());
        assert!(list.cursor.is_none());
    }

    #[test]
    fn workspace_list_parses_with_all_flags() {
        let cli = Cli::try_parse_from([
            "atlas",
            "grants",
            "workspace",
            "list",
            "--workspace",
            "my-ws",
            "--limit",
            "50",
            "--cursor",
            "abc123",
        ])
        .unwrap();
        let crate::cli::Commands::Grants(grants) = cli.command else {
            panic!("expected Grants");
        };
        let GrantsCmd::Workspace(ws) = grants.command else {
            panic!("expected Workspace");
        };
        let WorkspaceGrantsCmd::List(list) = ws.command else {
            panic!("expected List");
        };
        assert_eq!(list.workspace.as_deref(), Some("my-ws"));
        assert_eq!(list.limit, Some(50));
        assert_eq!(list.cursor.as_deref(), Some("abc123"));
    }

    #[test]
    fn workspace_create_parses_required_flags() {
        let id = Uuid::now_v7();
        let cli = Cli::try_parse_from([
            "atlas",
            "grants",
            "workspace",
            "create",
            "--principal-type",
            "user",
            "--principal-id",
            &id.to_string(),
            "--role",
            "viewer",
            "--workspace",
            "ws",
        ])
        .unwrap();
        let crate::cli::Commands::Grants(grants) = cli.command else {
            panic!("expected Grants");
        };
        let GrantsCmd::Workspace(ws) = grants.command else {
            panic!("expected Workspace");
        };
        let WorkspaceGrantsCmd::Create(create) = ws.command else {
            panic!("expected Create");
        };
        assert_eq!(create.principal_type, "user");
        assert_eq!(create.principal_id, id);
        assert_eq!(create.role, "viewer");
    }

    #[test]
    fn workspace_create_requires_principal_type() {
        let id = Uuid::now_v7();
        let result = Cli::try_parse_from([
            "atlas",
            "grants",
            "workspace",
            "create",
            "--principal-id",
            &id.to_string(),
            "--role",
            "viewer",
        ]);
        assert!(result.is_err(), "--principal-type is required");
    }

    #[test]
    fn workspace_revoke_parses_with_confirm() {
        let id = Uuid::now_v7();
        let cli = Cli::try_parse_from([
            "atlas",
            "grants",
            "workspace",
            "revoke",
            "--grant-id",
            &id.to_string(),
            "--confirm",
            "--workspace",
            "ws",
        ])
        .unwrap();
        let crate::cli::Commands::Grants(grants) = cli.command else {
            panic!("expected Grants");
        };
        let GrantsCmd::Workspace(ws) = grants.command else {
            panic!("expected Workspace");
        };
        let WorkspaceGrantsCmd::Revoke(rev) = ws.command else {
            panic!("expected Revoke");
        };
        assert_eq!(rev.grant_id, id);
        assert!(rev.confirm);
    }

    #[test]
    fn workspace_revoke_parses_without_confirm() {
        let id = Uuid::now_v7();
        let cli = Cli::try_parse_from([
            "atlas",
            "grants",
            "workspace",
            "revoke",
            "--grant-id",
            &id.to_string(),
            "--workspace",
            "ws",
        ])
        .unwrap();
        let crate::cli::Commands::Grants(grants) = cli.command else {
            panic!("expected Grants");
        };
        let GrantsCmd::Workspace(ws) = grants.command else {
            panic!("expected Workspace");
        };
        let WorkspaceGrantsCmd::Revoke(rev) = ws.command else {
            panic!("expected Revoke");
        };
        assert!(!rev.confirm, "confirm defaults to false");
    }

    #[test]
    fn workspace_revoke_confirm_guard_fires_before_network() {
        let args = WorkspaceGrantsRevokeArgs {
            grant_id: Uuid::now_v7(),
            workspace: Some("ws".to_owned()),
            confirm: false,
        };
        let err = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                use crate::ctx::Ctx;
                use crate::output::OutputFormat;
                use atlas_client::AtlasClient;
                let client = AtlasClient::new("http://localhost:1");
                let ctx = Ctx {
                    client,
                    output: OutputFormat::Human,
                    workspace: None,
                };
                run_workspace_revoke(&ctx, args).await
            });
        assert!(
            matches!(err, Err(CliError::Validation(_))),
            "confirm guard must fire before any network call"
        );
    }

    // -----------------------------------------------------------------------
    // Project grants parse tests
    // -----------------------------------------------------------------------

    #[test]
    fn project_list_requires_project() {
        let result = Cli::try_parse_from(["atlas", "grants", "project", "list"]);
        assert!(result.is_err(), "--project is required");
    }

    #[test]
    fn project_list_parses_with_project() {
        let cli =
            Cli::try_parse_from(["atlas", "grants", "project", "list", "--project", "my-proj"])
                .unwrap();
        let crate::cli::Commands::Grants(grants) = cli.command else {
            panic!("expected Grants");
        };
        let GrantsCmd::Project(proj) = grants.command else {
            panic!("expected Project");
        };
        let ProjectGrantsCmd::List(list) = proj.command else {
            panic!("expected List");
        };
        assert_eq!(list.project, "my-proj");
        assert!(list.workspace.is_none());
    }

    #[test]
    fn project_create_requires_project_and_principal_fields() {
        let result = Cli::try_parse_from([
            "atlas",
            "grants",
            "project",
            "create",
            "--principal-type",
            "user",
            "--role",
            "viewer",
        ]);
        assert!(result.is_err(), "--project and --principal-id are required");
    }

    #[test]
    fn project_revoke_parses_with_confirm() {
        let grant_id = Uuid::now_v7();
        let cli = Cli::try_parse_from([
            "atlas",
            "grants",
            "project",
            "revoke",
            "--project",
            "my-proj",
            "--grant-id",
            &grant_id.to_string(),
            "--confirm",
            "--workspace",
            "ws",
        ])
        .unwrap();
        let crate::cli::Commands::Grants(grants) = cli.command else {
            panic!("expected Grants");
        };
        let GrantsCmd::Project(proj) = grants.command else {
            panic!("expected Project");
        };
        let ProjectGrantsCmd::Revoke(rev) = proj.command else {
            panic!("expected Revoke");
        };
        assert_eq!(rev.project, "my-proj");
        assert_eq!(rev.grant_id, grant_id);
        assert!(rev.confirm);
    }

    #[test]
    fn project_revoke_confirm_guard_fires_before_network() {
        let args = ProjectGrantsRevokeArgs {
            project: "my-proj".to_owned(),
            grant_id: Uuid::now_v7(),
            workspace: Some("ws".to_owned()),
            confirm: false,
        };
        let err = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                use crate::ctx::Ctx;
                use crate::output::OutputFormat;
                use atlas_client::AtlasClient;
                let client = AtlasClient::new("http://localhost:1");
                let ctx = Ctx {
                    client,
                    output: OutputFormat::Human,
                    workspace: None,
                };
                run_project_revoke(&ctx, args).await
            });
        assert!(
            matches!(err, Err(CliError::Validation(_))),
            "confirm guard must fire before any network call"
        );
    }

    // -----------------------------------------------------------------------
    // Limit clamping
    // -----------------------------------------------------------------------

    #[test]
    fn limit_clamp_zero_becomes_one() {
        assert_eq!(0u32.clamp(LIMIT_MIN, LIMIT_MAX), 1);
    }

    #[test]
    fn limit_clamp_over_max_becomes_200() {
        assert_eq!(9999u32.clamp(LIMIT_MIN, LIMIT_MAX), 200);
    }
}
