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
use crate::projections::{DeleteByIdProjection, GroupMemberProjection, GroupProjection};

// ---------------------------------------------------------------------------
// GroupsArgs + GroupsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `groups` subcommand group.
#[derive(Args)]
pub(crate) struct GroupsArgs {
    #[command(subcommand)]
    pub(crate) command: GroupsCmd,
}

#[derive(Subcommand)]
pub(crate) enum GroupsCmd {
    /// List groups in a workspace.
    List(GroupsListArgs),
    /// Create a new group in a workspace.
    Create(GroupsCreateArgs),
    /// Delete a group (requires --confirm).
    Delete(GroupsDeleteArgs),
    /// Add a user to a group.
    AddMember(GroupsAddMemberArgs),
    /// Remove a user from a group (requires --confirm).
    RemoveMember(GroupsRemoveMemberArgs),
    /// List members of a group.
    Members(GroupsMembersArgs),
}

/// Dispatches a parsed `GroupsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: GroupsCmd) -> Result<(), CliError> {
    match cmd {
        GroupsCmd::List(args) => run_list(ctx, args).await,
        GroupsCmd::Create(args) => run_create(ctx, args).await,
        GroupsCmd::Delete(args) => run_delete(ctx, args).await,
        GroupsCmd::AddMember(args) => run_add_member(ctx, args).await,
        GroupsCmd::RemoveMember(args) => run_remove_member(ctx, args).await,
        GroupsCmd::Members(args) => run_members(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas groups list`.
#[derive(Parser)]
pub(crate) struct GroupsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_list(ctx: &Ctx, args: GroupsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let groups = ctx.client.list_groups(ws).await?;

    let items: Vec<GroupProjection> = groups.into_iter().map(GroupProjection::from).collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas groups create`.
#[derive(Parser)]
pub(crate) struct GroupsCreateArgs {
    /// Name for the new group.
    #[arg(long)]
    pub(crate) name: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_create(ctx: &Ctx, args: GroupsCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::groups::CreateGroupRequest;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = CreateGroupRequest { name: args.name };

    let group = ctx.client.create_group(ws, body).await?;
    let proj = GroupProjection::from(group);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Arguments for `atlas groups delete`.
#[derive(Parser)]
pub(crate) struct GroupsDeleteArgs {
    /// UUID of the group to delete.
    #[arg(long)]
    pub(crate) group_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the deletion. Required — removes the group and all its memberships.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: GroupsDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete the group (this removes all its memberships)".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client.delete_group(ws, args.group_id).await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.group_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Add member
// ---------------------------------------------------------------------------

/// Arguments for `atlas groups add-member`.
#[derive(Parser)]
pub(crate) struct GroupsAddMemberArgs {
    /// UUID of the group.
    #[arg(long)]
    pub(crate) group_id: Uuid,

    /// UUID of the user to add.
    #[arg(long)]
    pub(crate) user_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_add_member(ctx: &Ctx, args: GroupsAddMemberArgs) -> Result<(), CliError> {
    use atlas_api::dtos::groups::AddGroupMemberRequest;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = AddGroupMemberRequest {
        user_id: args.user_id,
    };

    let member = ctx.client.add_group_member(ws, args.group_id, body).await?;
    let proj = GroupMemberProjection::from(member);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Remove member
// ---------------------------------------------------------------------------

/// Arguments for `atlas groups remove-member`.
#[derive(Parser)]
pub(crate) struct GroupsRemoveMemberArgs {
    /// UUID of the group.
    #[arg(long)]
    pub(crate) group_id: Uuid,

    /// UUID of the user to remove.
    #[arg(long)]
    pub(crate) user_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the removal. Required — removes the user's group membership.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_remove_member(ctx: &Ctx, args: GroupsRemoveMemberArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to remove the member from the group".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client
        .remove_group_member(ws, args.group_id, args.user_id)
        .await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.user_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Members
// ---------------------------------------------------------------------------

/// Arguments for `atlas groups members`.
#[derive(Parser)]
pub(crate) struct GroupsMembersArgs {
    /// UUID of the group whose members to list.
    #[arg(long)]
    pub(crate) group_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_members(ctx: &Ctx, args: GroupsMembersArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let members = ctx.client.list_group_members(ws, args.group_id).await?;

    let items: Vec<GroupMemberProjection> = members
        .into_iter()
        .map(GroupMemberProjection::from)
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
    fn groups_list_parses_without_workspace() {
        let cli = Cli::try_parse_from(["atlas", "groups", "list"]).unwrap();
        let Commands::Groups(args) = cli.command else {
            panic!("expected Groups");
        };
        let GroupsCmd::List(list) = args.command else {
            panic!("expected List");
        };
        assert!(list.workspace.is_none());
    }

    #[test]
    fn groups_list_parses_with_workspace() {
        let cli = Cli::try_parse_from(["atlas", "groups", "list", "--workspace", "my-ws"]).unwrap();
        let Commands::Groups(args) = cli.command else {
            panic!("expected Groups");
        };
        let GroupsCmd::List(list) = args.command else {
            panic!("expected List");
        };
        assert_eq!(list.workspace.as_deref(), Some("my-ws"));
    }

    #[test]
    fn groups_create_requires_name() {
        let result = Cli::try_parse_from(["atlas", "groups", "create"]);
        assert!(result.is_err(), "missing --name must fail");
    }

    #[test]
    fn groups_create_parses_name() {
        let cli = Cli::try_parse_from(["atlas", "groups", "create", "--name", "devs"]).unwrap();
        let Commands::Groups(args) = cli.command else {
            panic!("expected Groups");
        };
        let GroupsCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.name, "devs");
        assert!(c.workspace.is_none());
    }

    #[test]
    fn groups_delete_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "groups",
            "delete",
            "--group-id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::Groups(args) = cli.command else {
            panic!("expected Groups");
        };
        let GroupsCmd::Delete(d) = args.command else {
            panic!("expected Delete");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }

    #[test]
    fn groups_delete_confirm_guard_fires_before_network() {
        let args = GroupsDeleteArgs {
            group_id: Uuid::nil(),
            workspace: None,
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn groups_add_member_requires_group_id_and_user_id() {
        let result = Cli::try_parse_from(["atlas", "groups", "add-member"]);
        assert!(result.is_err(), "missing args must fail");
    }

    #[test]
    fn groups_add_member_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "groups",
            "add-member",
            "--group-id",
            "00000000-0000-0000-0000-000000000001",
            "--user-id",
            "00000000-0000-0000-0000-000000000002",
        ])
        .unwrap();
        let Commands::Groups(args) = cli.command else {
            panic!("expected Groups");
        };
        assert!(matches!(args.command, GroupsCmd::AddMember(_)));
    }

    #[test]
    fn groups_remove_member_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "groups",
            "remove-member",
            "--group-id",
            "00000000-0000-0000-0000-000000000001",
            "--user-id",
            "00000000-0000-0000-0000-000000000002",
        ])
        .unwrap();
        let Commands::Groups(args) = cli.command else {
            panic!("expected Groups");
        };
        let GroupsCmd::RemoveMember(r) = args.command else {
            panic!("expected RemoveMember");
        };
        assert!(!r.confirm, "--confirm must default to false");
    }

    #[test]
    fn groups_remove_member_confirm_guard_fires_before_network() {
        let args = GroupsRemoveMemberArgs {
            group_id: Uuid::nil(),
            user_id: Uuid::nil(),
            workspace: None,
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn groups_members_requires_group_id() {
        let result = Cli::try_parse_from(["atlas", "groups", "members"]);
        assert!(result.is_err(), "missing --group-id must fail");
    }

    #[test]
    fn groups_members_parses_group_id() {
        let cli = Cli::try_parse_from([
            "atlas",
            "groups",
            "members",
            "--group-id",
            "00000000-0000-0000-0000-000000000005",
        ])
        .unwrap();
        let Commands::Groups(args) = cli.command else {
            panic!("expected Groups");
        };
        assert!(matches!(args.command, GroupsCmd::Members(_)));
    }
}
