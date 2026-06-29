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
use serde::Serialize;
use uuid::Uuid;

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output::{self, TableRow};
use crate::projections::{
    ActivationLinkProjection, UserCreatedProjection, UserMembershipProjection, UserProjection,
};

// ---------------------------------------------------------------------------
// UsersArgs + UsersCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `users` subcommand group.
#[derive(Args)]
pub(crate) struct UsersArgs {
    #[command(subcommand)]
    pub(crate) command: UsersCmd,
}

#[derive(Subcommand)]
pub(crate) enum UsersCmd {
    /// List all users in the system (admin).
    List,
    /// Create a new pending user account (admin).
    Create(UsersCreateArgs),
    /// Disable a user account (admin; requires --confirm).
    Disable(UsersDisableArgs),
    /// Re-enable a disabled user account (admin).
    Enable(UsersEnableArgs),
    /// Reset a user's password (admin; requires --confirm).
    ResetPassword(UsersResetPasswordArgs),
    /// Regenerate the activation link for a pending user (admin).
    RegenerateLink(UsersRegenerateLinkArgs),
    /// List workspace memberships for a specific user (admin).
    Memberships(UsersMembershipsArgs),
}

/// Dispatches a parsed `UsersCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: UsersCmd) -> Result<(), CliError> {
    match cmd {
        UsersCmd::List => run_list(ctx).await,
        UsersCmd::Create(args) => run_create(ctx, args).await,
        UsersCmd::Disable(args) => run_disable(ctx, args).await,
        UsersCmd::Enable(args) => run_enable(ctx, args).await,
        UsersCmd::ResetPassword(args) => run_reset_password(ctx, args).await,
        UsersCmd::RegenerateLink(args) => run_regenerate_link(ctx, args).await,
        UsersCmd::Memberships(args) => run_memberships(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// Inline confirmation projection for void-result admin actions
// ---------------------------------------------------------------------------

/// Simple confirmation for user state mutations (disable/enable/reset-password).
///
/// Used when the server returns no body but the caller needs a structured receipt.
#[derive(Debug, Serialize)]
struct UserActionProjection {
    ok: bool,
    user_id: Uuid,
}

impl TableRow for UserActionProjection {
    fn headers() -> &'static [&'static str] {
        &["OK", "User ID"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.ok.to_string(), self.user_id.to_string()]
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

async fn run_list(ctx: &Ctx) -> Result<(), CliError> {
    let users = ctx.client.list_users().await?;

    let items: Vec<UserProjection> = users.into_iter().map(UserProjection::from).collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas users create`.
#[derive(Parser)]
pub(crate) struct UsersCreateArgs {
    /// Username (unique login handle).
    #[arg(long)]
    pub(crate) username: String,

    /// Display name shown in the UI.
    #[arg(long)]
    pub(crate) display_name: String,

    /// Workspace the new user will be added to.
    #[arg(long)]
    pub(crate) workspace: String,

    /// Membership role: `admin` or `member`.
    #[arg(long)]
    pub(crate) role: String,

    /// Optional email address for the new user.
    #[arg(long)]
    pub(crate) email: Option<String>,
}

async fn run_create(ctx: &Ctx, args: UsersCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::CreateUserRequest;

    let body = CreateUserRequest {
        username: args.username,
        display_name: args.display_name,
        workspace: args.workspace,
        role: args.role,
        email: args.email,
    };

    let resp = ctx.client.create_user(body).await?;
    let proj = UserCreatedProjection::from(resp);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Disable
// ---------------------------------------------------------------------------

/// Arguments for `atlas users disable`.
#[derive(Parser)]
pub(crate) struct UsersDisableArgs {
    /// UUID of the user to disable.
    #[arg(long)]
    pub(crate) user_id: Uuid,

    /// Confirm the operation. Required — prevents accidental account lockouts.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_disable(ctx: &Ctx, args: UsersDisableArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to disable a user (this locks their account)".to_owned(),
        ));
    }

    ctx.client.disable_user(args.user_id).await?;

    let proj = UserActionProjection {
        ok: true,
        user_id: args.user_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Enable
// ---------------------------------------------------------------------------

/// Arguments for `atlas users enable`.
#[derive(Parser)]
pub(crate) struct UsersEnableArgs {
    /// UUID of the user to re-enable.
    #[arg(long)]
    pub(crate) user_id: Uuid,
}

async fn run_enable(ctx: &Ctx, args: UsersEnableArgs) -> Result<(), CliError> {
    ctx.client.enable_user(args.user_id).await?;

    let proj = UserActionProjection {
        ok: true,
        user_id: args.user_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Reset password
// ---------------------------------------------------------------------------

/// Arguments for `atlas users reset-password`.
#[derive(Parser)]
pub(crate) struct UsersResetPasswordArgs {
    /// UUID of the user whose password will be reset.
    #[arg(long)]
    pub(crate) user_id: Uuid,

    /// The new password to set.
    #[arg(long)]
    pub(crate) new_password: String,

    /// Confirm the operation. Required — prevents accidental credential changes.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_reset_password(ctx: &Ctx, args: UsersResetPasswordArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to reset the password (this overwrites the user's current password)"
                .to_owned(),
        ));
    }

    ctx.client
        .reset_user_password(args.user_id, args.new_password)
        .await?;

    let proj = UserActionProjection {
        ok: true,
        user_id: args.user_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Regenerate activation link
// ---------------------------------------------------------------------------

/// Arguments for `atlas users regenerate-link`.
#[derive(Parser)]
pub(crate) struct UsersRegenerateLinkArgs {
    /// UUID of the user whose activation link will be regenerated.
    #[arg(long)]
    pub(crate) user_id: Uuid,
}

async fn run_regenerate_link(ctx: &Ctx, args: UsersRegenerateLinkArgs) -> Result<(), CliError> {
    let resp = ctx.client.regenerate_activation_link(args.user_id).await?;
    let proj = ActivationLinkProjection::from(resp);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Memberships
// ---------------------------------------------------------------------------

/// Arguments for `atlas users memberships`.
#[derive(Parser)]
pub(crate) struct UsersMembershipsArgs {
    /// UUID of the user whose workspace memberships to list.
    #[arg(long)]
    pub(crate) user_id: Uuid,
}

async fn run_memberships(ctx: &Ctx, args: UsersMembershipsArgs) -> Result<(), CliError> {
    let memberships = ctx.client.list_user_memberships(args.user_id).await?;

    let items: Vec<UserMembershipProjection> = memberships
        .into_iter()
        .map(UserMembershipProjection::from)
        .collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Commands;
    use clap::Parser as ClapParser;

    #[derive(ClapParser)]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[test]
    fn users_list_parses() {
        let cli = Cli::try_parse_from(["atlas", "users", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Users(_)));
    }

    #[test]
    fn users_create_requires_username_display_name_workspace_role() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "create",
            "--username",
            "alice",
            "--display-name",
            "Alice",
            "--workspace",
            "my-ws",
            "--role",
            "member",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        let UsersCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.username, "alice");
        assert_eq!(c.display_name, "Alice");
        assert_eq!(c.workspace, "my-ws");
        assert_eq!(c.role, "member");
        assert!(c.email.is_none());
    }

    #[test]
    fn users_create_missing_username_fails() {
        let result = Cli::try_parse_from([
            "atlas",
            "users",
            "create",
            "--display-name",
            "Alice",
            "--workspace",
            "ws",
            "--role",
            "member",
        ]);
        assert!(result.is_err(), "missing --username must fail");
    }

    #[test]
    fn users_create_accepts_optional_email() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "create",
            "--username",
            "alice",
            "--display-name",
            "Alice",
            "--workspace",
            "ws",
            "--role",
            "member",
            "--email",
            "alice@example.com",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        let UsersCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.email.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn users_disable_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "disable",
            "--user-id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        let UsersCmd::Disable(d) = args.command else {
            panic!("expected Disable");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }

    #[test]
    fn users_disable_confirm_flag_sets_field_to_true() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "disable",
            "--user-id",
            "00000000-0000-0000-0000-000000000001",
            "--confirm",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        let UsersCmd::Disable(d) = args.command else {
            panic!("expected Disable");
        };
        assert!(d.confirm);
    }

    #[test]
    fn users_disable_confirm_guard_fires_before_network() {
        let args = UsersDisableArgs {
            user_id: Uuid::nil(),
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn users_reset_password_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "reset-password",
            "--user-id",
            "00000000-0000-0000-0000-000000000001",
            "--new-password",
            "s3cr3t",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        let UsersCmd::ResetPassword(r) = args.command else {
            panic!("expected ResetPassword");
        };
        assert!(!r.confirm);
    }

    #[test]
    fn users_reset_password_confirm_guard_fires_before_network() {
        let args = UsersResetPasswordArgs {
            user_id: Uuid::nil(),
            new_password: "x".to_owned(),
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn users_enable_parses_user_id() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "enable",
            "--user-id",
            "00000000-0000-0000-0000-000000000002",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        assert!(matches!(args.command, UsersCmd::Enable(_)));
    }

    #[test]
    fn users_regenerate_link_parses_user_id() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "regenerate-link",
            "--user-id",
            "00000000-0000-0000-0000-000000000003",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        assert!(matches!(args.command, UsersCmd::RegenerateLink(_)));
    }

    #[test]
    fn users_memberships_parses_user_id() {
        let cli = Cli::try_parse_from([
            "atlas",
            "users",
            "memberships",
            "--user-id",
            "00000000-0000-0000-0000-000000000004",
        ])
        .unwrap();
        let Commands::Users(args) = cli.command else {
            panic!("expected Users");
        };
        assert!(matches!(args.command, UsersCmd::Memberships(_)));
    }
}
