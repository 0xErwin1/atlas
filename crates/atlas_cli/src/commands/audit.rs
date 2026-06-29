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

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::AuditEntryProjection;

const LIMIT_MIN: u32 = 1;
const LIMIT_MAX: u32 = 200;
const LIMIT_DEFAULT: u32 = 20;

// ---------------------------------------------------------------------------
// AuditArgs (top-level) + AuditCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `audit` subcommand group.
#[derive(Args)]
pub(crate) struct AuditArgs {
    #[command(subcommand)]
    pub(crate) command: AuditCmd,
}

#[derive(Subcommand)]
pub(crate) enum AuditCmd {
    /// List security audit events for a workspace.
    Workspace(AuditWorkspaceArgs),
    /// List platform-wide security audit events (admin).
    Platform(AuditPlatformArgs),
}

/// Dispatches a parsed `AuditCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: AuditCmd) -> Result<(), CliError> {
    match cmd {
        AuditCmd::Workspace(args) => run_workspace(ctx, args).await,
        AuditCmd::Platform(args) => run_platform(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// Shared filter arguments (inlined per command to keep Args derives simple)
// ---------------------------------------------------------------------------

/// Arguments for `atlas audit workspace`.
#[derive(Parser)]
pub(crate) struct AuditWorkspaceArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Filter by actor identifier (user email or API key name).
    #[arg(long)]
    pub(crate) actor: Option<String>,

    /// Filter by action name (e.g. `membership.role_changed`).
    #[arg(long)]
    pub(crate) action: Option<String>,

    /// Start of the time range (ISO 8601, inclusive).
    #[arg(long)]
    pub(crate) from: Option<String>,

    /// End of the time range (ISO 8601, exclusive). Ignored when --cursor is set.
    #[arg(long)]
    pub(crate) to: Option<String>,

    /// Maximum number of entries to return (clamped to 1..=200).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Opaque pagination cursor returned by a previous call.
    /// When provided, `--to` is ignored and the next page is fetched.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

/// Arguments for `atlas audit platform`.
#[derive(Parser)]
pub(crate) struct AuditPlatformArgs {
    /// Filter by actor identifier (user email or API key name).
    #[arg(long)]
    pub(crate) actor: Option<String>,

    /// Filter by action name (e.g. `user.created`).
    #[arg(long)]
    pub(crate) action: Option<String>,

    /// Start of the time range (ISO 8601, inclusive).
    #[arg(long)]
    pub(crate) from: Option<String>,

    /// End of the time range (ISO 8601, exclusive). Ignored when --cursor is set.
    #[arg(long)]
    pub(crate) to: Option<String>,

    /// Maximum number of entries to return (clamped to 1..=200).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Opaque pagination cursor returned by a previous call.
    /// When provided, `--to` is ignored and the next page is fetched.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn run_workspace(ctx: &Ctx, args: AuditWorkspaceArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = if args.cursor.is_some() {
        ctx.client
            .list_workspace_audit_with_cursor(
                ws,
                args.actor.as_deref(),
                args.action.as_deref(),
                args.from.as_deref(),
                args.cursor.as_deref(),
                Some(limit),
            )
            .await?
    } else {
        ctx.client
            .list_workspace_audit(
                ws,
                args.actor.as_deref(),
                args.action.as_deref(),
                args.from.as_deref(),
                args.to.as_deref(),
                Some(limit),
            )
            .await?
    };

    let items: Vec<AuditEntryProjection> = page
        .items
        .into_iter()
        .map(AuditEntryProjection::from)
        .collect();
    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

async fn run_platform(ctx: &Ctx, args: AuditPlatformArgs) -> Result<(), CliError> {
    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = if args.cursor.is_some() {
        ctx.client
            .list_platform_audit_with_cursor(
                args.actor.as_deref(),
                args.action.as_deref(),
                args.from.as_deref(),
                args.cursor.as_deref(),
                Some(limit),
            )
            .await?
    } else {
        ctx.client
            .list_platform_audit(
                args.actor.as_deref(),
                args.action.as_deref(),
                args.from.as_deref(),
                args.to.as_deref(),
                Some(limit),
            )
            .await?
    };

    let items: Vec<AuditEntryProjection> = page
        .items
        .into_iter()
        .map(AuditEntryProjection::from)
        .collect();
    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
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
    // Workspace audit parse tests
    // -----------------------------------------------------------------------

    #[test]
    fn workspace_parses_with_workspace() {
        let cli =
            Cli::try_parse_from(["atlas", "audit", "workspace", "--workspace", "my-ws"]).unwrap();
        let crate::cli::Commands::Audit(audit) = cli.command else {
            panic!("expected Audit");
        };
        let AuditCmd::Workspace(ws) = audit.command else {
            panic!("expected Workspace");
        };
        assert_eq!(ws.workspace.as_deref(), Some("my-ws"));
    }

    #[test]
    fn workspace_parses_all_filter_flags() {
        let cli = Cli::try_parse_from([
            "atlas",
            "audit",
            "workspace",
            "--workspace",
            "ws",
            "--actor",
            "alice@example.com",
            "--action",
            "membership.role_changed",
            "--from",
            "2026-01-01T00:00:00Z",
            "--to",
            "2026-02-01T00:00:00Z",
            "--limit",
            "100",
        ])
        .unwrap();
        let crate::cli::Commands::Audit(audit) = cli.command else {
            panic!("expected Audit");
        };
        let AuditCmd::Workspace(ws) = audit.command else {
            panic!("expected Workspace");
        };
        assert_eq!(ws.actor.as_deref(), Some("alice@example.com"));
        assert_eq!(ws.action.as_deref(), Some("membership.role_changed"));
        assert_eq!(ws.from.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(ws.to.as_deref(), Some("2026-02-01T00:00:00Z"));
        assert_eq!(ws.limit, Some(100));
        assert!(ws.cursor.is_none());
    }

    #[test]
    fn workspace_parses_cursor_flag() {
        let cli = Cli::try_parse_from([
            "atlas",
            "audit",
            "workspace",
            "--workspace",
            "ws",
            "--cursor",
            "opaque-cursor-abc",
        ])
        .unwrap();
        let crate::cli::Commands::Audit(audit) = cli.command else {
            panic!("expected Audit");
        };
        let AuditCmd::Workspace(ws) = audit.command else {
            panic!("expected Workspace");
        };
        assert_eq!(ws.cursor.as_deref(), Some("opaque-cursor-abc"));
    }

    // -----------------------------------------------------------------------
    // Platform audit parse tests
    // -----------------------------------------------------------------------

    #[test]
    fn platform_parses_without_workspace() {
        let cli = Cli::try_parse_from(["atlas", "audit", "platform"]).unwrap();
        let crate::cli::Commands::Audit(audit) = cli.command else {
            panic!("expected Audit");
        };
        assert!(matches!(audit.command, AuditCmd::Platform(_)));
    }

    #[test]
    fn platform_parses_all_filter_flags() {
        let cli = Cli::try_parse_from([
            "atlas",
            "audit",
            "platform",
            "--actor",
            "admin@example.com",
            "--action",
            "user.created",
            "--from",
            "2026-01-01T00:00:00Z",
            "--to",
            "2026-02-01T00:00:00Z",
            "--limit",
            "50",
        ])
        .unwrap();
        let crate::cli::Commands::Audit(audit) = cli.command else {
            panic!("expected Audit");
        };
        let AuditCmd::Platform(plat) = audit.command else {
            panic!("expected Platform");
        };
        assert_eq!(plat.actor.as_deref(), Some("admin@example.com"));
        assert_eq!(plat.action.as_deref(), Some("user.created"));
        assert_eq!(plat.limit, Some(50));
        assert!(plat.cursor.is_none());
    }

    #[test]
    fn platform_parses_cursor_flag() {
        let cli =
            Cli::try_parse_from(["atlas", "audit", "platform", "--cursor", "next-page-token"])
                .unwrap();
        let crate::cli::Commands::Audit(audit) = cli.command else {
            panic!("expected Audit");
        };
        let AuditCmd::Platform(plat) = audit.command else {
            panic!("expected Platform");
        };
        assert_eq!(plat.cursor.as_deref(), Some("next-page-token"));
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

    #[test]
    fn limit_default_is_20() {
        assert_eq!(LIMIT_DEFAULT, 20);
    }
}
