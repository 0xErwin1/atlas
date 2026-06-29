#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use clap::{Parser, Subcommand};

use crate::commands::activity::ActivityArgs;
use crate::commands::api_keys::ApiKeysArgs;
use crate::commands::audit::AuditArgs;
use crate::commands::boards::BoardsArgs;
use crate::commands::columns::ColumnsArgs;
use crate::commands::docs::DocsArgs;
use crate::commands::folders::FoldersArgs;
use crate::commands::grants::GrantsArgs;
use crate::commands::groups::GroupsArgs;
use crate::commands::members::MembersArgs;
use crate::commands::projects::ProjectsArgs;
use crate::commands::property_definitions::PropertyDefinitionsArgs;
use crate::commands::saved_searches::SavedSearchesArgs;
use crate::commands::status_templates::StatusTemplatesArgs;
use crate::commands::tags::TagsArgs;
use crate::commands::task_views::TaskViewsArgs;
use crate::commands::tasks::TasksArgs;
use crate::commands::users::UsersArgs;
use crate::commands::workspaces::WorkspacesArgs;

/// Atlas CLI — personal knowledge base client.
#[derive(Parser)]
#[command(name = "atlas", about = "Atlas CLI")]
pub(crate) struct Cli {
    /// Override the Atlas server URL.
    #[arg(long, global = true)]
    pub(crate) base_url: Option<String>,

    /// Bearer token for authentication.
    #[arg(long, global = true)]
    pub(crate) token: Option<String>,

    /// Emit JSON output (implied when stdout is not a TTY).
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Default workspace slug used when `--workspace` is not given per-command.
    #[arg(long, global = true)]
    pub(crate) workspace: Option<String>,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Print the version of the CLI binary.
    Version,
    /// Search across documents and tasks in a workspace.
    Search(SearchArgs),
    /// Manage tasks (create, list, get, update, move, delete).
    Tasks(TasksArgs),
    /// Manage documents (list, get, create, update-metadata, update-content, delete).
    Docs(DocsArgs),
    /// Inspect workspaces (list, get).
    Workspaces(WorkspacesArgs),
    /// Inspect projects (list, get).
    Projects(ProjectsArgs),
    /// Inspect boards (list).
    Boards(BoardsArgs),
    /// Inspect columns on a board (list).
    Columns(ColumnsArgs),
    /// Inspect workspace tags (list).
    Tags(TagsArgs),
    /// Inspect workspace members (list).
    Members(MembersArgs),
    /// Inspect folders (list, get).
    Folders(FoldersArgs),
    /// List workspace-level activity (audit log).
    Activity(ActivityArgs),
    /// Manage system users (admin; list, create, disable, enable, reset-password, regenerate-link, memberships).
    Users(UsersArgs),
    /// Manage personal API keys (list, create, revoke, set-global, grants, delete-grant).
    ApiKeys(ApiKeysArgs),
    /// Manage workspace groups (list, create, delete, add-member, remove-member, members).
    Groups(GroupsArgs),
    /// Manage workspace status templates (list, create, update, delete, apply).
    StatusTemplates(StatusTemplatesArgs),
    /// Manage workspace saved searches (list, create, rename, delete).
    SavedSearches(SavedSearchesArgs),
    /// Manage workspace task views (list, get, create, update, delete).
    TaskViews(TaskViewsArgs),
    /// Manage workspace property definitions / custom fields (list, create, delete).
    PropertyDefinitions(PropertyDefinitionsArgs),
    /// Manage workspace and project permission grants (workspace/project list, create, revoke).
    Grants(GrantsArgs),
    /// Query security audit logs (workspace, platform).
    Audit(AuditArgs),
}

/// Arguments for the `search` subcommand.
#[derive(Parser)]
pub(crate) struct SearchArgs {
    /// Workspace slug to search in.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Query string (required). Supports token filters such as
    /// `status:open`, `tag:rust`, `project:atlas`.
    #[arg(index = 1)]
    pub(crate) query: String,

    /// Filter results by kind: `all` (default), `note`, or `task`.
    #[arg(long, default_value = "all")]
    pub(crate) r#type: String,

    /// Sort order: `relevance` (default) or `updated`.
    #[arg(long, default_value = "relevance")]
    pub(crate) sort: String,

    /// Maximum number of results (clamped to 1..=200).
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_json_flag_parses_before_subcommand() {
        let cli = Cli::try_parse_from(["atlas", "--json", "version"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn global_json_flag_parses_after_subcommand() {
        let cli =
            Cli::try_parse_from(["atlas", "search", "--json", "--workspace", "ws", "q"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn global_workspace_flag_parses() {
        let cli = Cli::try_parse_from(["atlas", "--workspace", "my-ws", "version"]).unwrap();
        assert_eq!(cli.workspace.as_deref(), Some("my-ws"));
    }

    #[test]
    fn base_url_absence_leaves_field_as_none() {
        let cli = Cli::try_parse_from(["atlas", "version"]).unwrap();
        assert!(
            cli.base_url.is_none(),
            "--base-url must default to None (no clap default)"
        );
    }

    #[test]
    fn token_absence_leaves_field_as_none() {
        let cli = Cli::try_parse_from(["atlas", "version"]).unwrap();
        assert!(cli.token.is_none());
    }

    #[test]
    fn base_url_can_be_set_via_flag() {
        let cli =
            Cli::try_parse_from(["atlas", "--base-url", "https://example.com", "version"]).unwrap();
        assert_eq!(cli.base_url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn unknown_global_flag_fails_with_nonzero_exit() {
        let result = Cli::try_parse_from(["atlas", "--confirm", "version"]);
        assert!(
            result.is_err(),
            "--confirm is not a global flag and must fail at parse time"
        );
    }

    #[test]
    fn version_subcommand_parses() {
        let cli = Cli::try_parse_from(["atlas", "version"]).unwrap();
        assert!(matches!(cli.command, Commands::Version));
    }

    #[test]
    fn search_subcommand_parses_required_args() {
        let cli = Cli::try_parse_from(["atlas", "search", "--workspace", "ws", "hello"]).unwrap();
        if let Commands::Search(args) = cli.command {
            assert_eq!(args.workspace.as_deref(), Some("ws"));
            assert_eq!(args.query, "hello");
        } else {
            panic!("expected Search command");
        }
    }

    #[test]
    fn search_limit_parses_as_option() {
        let cli =
            Cli::try_parse_from(["atlas", "search", "--workspace", "ws", "--limit", "50", "q"])
                .unwrap();
        if let Commands::Search(args) = cli.command {
            assert_eq!(args.limit, Some(50));
        } else {
            panic!("expected Search");
        }
    }
}
