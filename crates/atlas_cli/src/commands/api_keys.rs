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
use crate::projections::{
    ApiKeyCreatedProjection, ApiKeyGrantProjection, ApiKeyProjection, DeleteByIdProjection,
};

// ---------------------------------------------------------------------------
// ApiKeysArgs + ApiKeysCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `api-keys` subcommand group.
#[derive(Args)]
pub(crate) struct ApiKeysArgs {
    #[command(subcommand)]
    pub(crate) command: ApiKeysCmd,
}

#[derive(Subcommand)]
pub(crate) enum ApiKeysCmd {
    /// List API keys for the current user.
    List(ApiKeysListArgs),
    /// Create a new API key (the secret is shown exactly once).
    Create(ApiKeysCreateArgs),
    /// Revoke an API key permanently (requires --confirm).
    Revoke(ApiKeysRevokeArgs),
    /// Toggle a key's global workspace reach.
    SetGlobal(ApiKeysSetGlobalArgs),
    /// Replace the full capability scope set for a key.
    SetScopes(ApiKeysSetScopesArgs),
    /// List grants associated with an API key.
    Grants(ApiKeysGrantsArgs),
    /// Delete a specific grant from an API key (requires --confirm).
    DeleteGrant(ApiKeysDeleteGrantArgs),
}

/// Dispatches a parsed `ApiKeysCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: ApiKeysCmd) -> Result<(), CliError> {
    match cmd {
        ApiKeysCmd::List(args) => run_list(ctx, args).await,
        ApiKeysCmd::Create(args) => run_create(ctx, args).await,
        ApiKeysCmd::Revoke(args) => run_revoke(ctx, args).await,
        ApiKeysCmd::SetGlobal(args) => run_set_global(ctx, args).await,
        ApiKeysCmd::SetScopes(args) => run_set_scopes(ctx, args).await,
        ApiKeysCmd::Grants(args) => run_grants(ctx, args).await,
        ApiKeysCmd::DeleteGrant(args) => run_delete_grant(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas api-keys list`.
#[derive(Parser)]
pub(crate) struct ApiKeysListArgs {
    /// Maximum number of keys to return (clamped to 1..=200).
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

async fn run_list(ctx: &Ctx, args: ApiKeysListArgs) -> Result<(), CliError> {
    let limit = args.limit.map(|l| l.clamp(1, 200));

    let page = ctx.client.list_user_api_keys(None, limit).await?;

    let items: Vec<ApiKeyProjection> = page.items.into_iter().map(ApiKeyProjection::from).collect();

    let cursor = page.next_cursor.as_deref();
    output::emit_list(ctx.output, &items, cursor, page.has_more)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas api-keys create`.
#[derive(Parser)]
pub(crate) struct ApiKeysCreateArgs {
    /// Name for the new API key.
    #[arg(long)]
    pub(crate) name: String,

    /// Key purpose: `agent`, `cli`, `bot`, or `integration` (defaults to `agent`).
    #[arg(long)]
    pub(crate) r#type: Option<String>,

    /// Optional expiry date-time in RFC 3339 format (e.g. `2025-12-31T00:00:00Z`).
    #[arg(long)]
    pub(crate) expires_at: Option<String>,

    /// Workspace slug for an initial grant (must be combined with --initial-grant-role).
    #[arg(long)]
    pub(crate) initial_grant_workspace: Option<String>,

    /// Role for the initial grant: `viewer` or `editor`.
    #[arg(long)]
    pub(crate) initial_grant_role: Option<String>,

    /// Capability scope to grant, in `family:action` form (repeatable).
    ///
    /// Families: `tasks`, `docs`, `boards`, `folders`, `projects`. Actions:
    /// `read`, `create`, `update`, `delete`. Omit entirely to receive the
    /// server's read-only default.
    #[arg(long = "scope", value_name = "FAMILY:ACTION")]
    pub(crate) scopes: Vec<String>,
}

async fn run_create(ctx: &Ctx, args: ApiKeysCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::{CreateUserApiKeyRequest, InitialGrantRequest};

    let expires_at = args
        .expires_at
        .as_deref()
        .map(|s| {
            s.parse::<chrono::DateTime<chrono::Utc>>()
                .map_err(|e| CliError::Validation(format!("invalid --expires-at: {e}")))
        })
        .transpose()?;

    let initial_grant = match (args.initial_grant_workspace, args.initial_grant_role) {
        (Some(workspace), Some(role)) => Some(InitialGrantRequest { workspace, role }),
        (None, None) => None,
        _ => {
            return Err(CliError::Validation(
                "--initial-grant-workspace and --initial-grant-role must be provided together"
                    .to_owned(),
            ));
        }
    };

    let scopes = collect_scopes(&args.scopes)?;

    let body = CreateUserApiKeyRequest {
        name: args.name,
        r#type: args.r#type,
        expires_at,
        initial_grant,
        // Omitting `--scope` sends `None`, so the server applies its read-only
        // default; any provided scopes are the exact set to grant.
        scopes: if scopes.is_empty() {
            None
        } else {
            Some(scopes)
        },
    };

    let created = ctx.client.create_user_api_key(body).await?;
    let proj = ApiKeyCreatedProjection::from(created);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Revoke
// ---------------------------------------------------------------------------

/// Arguments for `atlas api-keys revoke`.
#[derive(Parser)]
pub(crate) struct ApiKeysRevokeArgs {
    /// UUID of the API key to revoke.
    #[arg(long)]
    pub(crate) key_id: Uuid,

    /// Confirm the revocation. Required — revocation is permanent.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_revoke(ctx: &Ctx, args: ApiKeysRevokeArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to revoke the key (this is permanent)".to_owned(),
        ));
    }

    ctx.client.revoke_user_api_key(args.key_id).await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.key_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Set-global
// ---------------------------------------------------------------------------

/// Arguments for `atlas api-keys set-global`.
#[derive(Parser)]
pub(crate) struct ApiKeysSetGlobalArgs {
    /// UUID of the API key to update.
    #[arg(long)]
    pub(crate) key_id: Uuid,

    /// Enable (`true`) or disable (`false`) global reach for this key.
    #[arg(long)]
    pub(crate) global: bool,
}

async fn run_set_global(ctx: &Ctx, args: ApiKeysSetGlobalArgs) -> Result<(), CliError> {
    let key = ctx
        .client
        .set_api_key_global(args.key_id, args.global)
        .await?;
    let proj = ApiKeyProjection::from(key);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Set-scopes
// ---------------------------------------------------------------------------

/// Arguments for `atlas api-keys set-scopes`.
#[derive(Parser)]
pub(crate) struct ApiKeysSetScopesArgs {
    /// UUID of the API key to update.
    #[arg(long)]
    pub(crate) key_id: Uuid,

    /// Capability scope in `family:action` form (repeatable). At least one is
    /// required; the provided set fully replaces the key's existing scopes. To
    /// remove all access, revoke the key instead.
    #[arg(long = "scope", value_name = "FAMILY:ACTION", required = true)]
    pub(crate) scopes: Vec<String>,
}

async fn run_set_scopes(ctx: &Ctx, args: ApiKeysSetScopesArgs) -> Result<(), CliError> {
    let scopes = collect_scopes(&args.scopes)?;

    let key = ctx.client.set_api_key_scopes(args.key_id, scopes).await?;
    let proj = ApiKeyProjection::from(key);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Scope parsing
// ---------------------------------------------------------------------------

/// Parses a repeatable `--scope family:action` list into wire capability scopes.
///
/// Each token is validated against the closed `ApiKeyScope` catalog through its
/// own serde mapping, so an unknown `family:action` produces a clean CLI
/// validation error rather than a panic.
fn collect_scopes(raw: &[String]) -> Result<Vec<atlas_api::dtos::ApiKeyScope>, CliError> {
    raw.iter().map(|s| parse_scope(s)).collect()
}

/// Maps a single `family:action` token to its `ApiKeyScope` variant.
fn parse_scope(raw: &str) -> Result<atlas_api::dtos::ApiKeyScope, CliError> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned())).map_err(|_| {
        CliError::Validation(format!(
            "invalid --scope '{raw}': expected `family:action` \
             (families: tasks, docs, boards, folders, projects; \
             actions: read, create, update, delete)"
        ))
    })
}

// ---------------------------------------------------------------------------
// Grants
// ---------------------------------------------------------------------------

/// Arguments for `atlas api-keys grants`.
#[derive(Parser)]
pub(crate) struct ApiKeysGrantsArgs {
    /// UUID of the API key whose grants to list.
    #[arg(long)]
    pub(crate) key_id: Uuid,
}

async fn run_grants(ctx: &Ctx, args: ApiKeysGrantsArgs) -> Result<(), CliError> {
    let grants = ctx.client.list_api_key_grants(args.key_id).await?;

    let items: Vec<ApiKeyGrantProjection> = grants
        .into_iter()
        .map(ApiKeyGrantProjection::from)
        .collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Delete-grant
// ---------------------------------------------------------------------------

/// Arguments for `atlas api-keys delete-grant`.
#[derive(Parser)]
pub(crate) struct ApiKeysDeleteGrantArgs {
    /// UUID of the API key.
    #[arg(long)]
    pub(crate) key_id: Uuid,

    /// UUID of the grant to delete.
    #[arg(long)]
    pub(crate) grant_id: Uuid,

    /// Confirm the deletion. Required — removes access to the scoped resource.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete_grant(ctx: &Ctx, args: ApiKeysDeleteGrantArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete the grant (this removes scoped access)".to_owned(),
        ));
    }

    ctx.client
        .delete_api_key_grant(args.key_id, args.grant_id)
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
    use crate::cli::Commands;
    use clap::Parser as ClapParser;

    #[derive(ClapParser)]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[test]
    fn api_keys_list_parses_without_limit() {
        let cli = Cli::try_parse_from(["atlas", "api-keys", "list"]).unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::List(list) = args.command else {
            panic!("expected List");
        };
        assert!(list.limit.is_none());
    }

    #[test]
    fn api_keys_list_parses_with_limit() {
        let cli = Cli::try_parse_from(["atlas", "api-keys", "list", "--limit", "50"]).unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::List(list) = args.command else {
            panic!("expected List");
        };
        assert_eq!(list.limit, Some(50));
    }

    #[test]
    fn api_keys_create_requires_name() {
        let result = Cli::try_parse_from(["atlas", "api-keys", "create"]);
        assert!(result.is_err(), "missing --name must fail");
    }

    #[test]
    fn api_keys_create_parses_name() {
        let cli = Cli::try_parse_from(["atlas", "api-keys", "create", "--name", "my-key"]).unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.name, "my-key");
        assert!(c.r#type.is_none());
        assert!(c.expires_at.is_none());
        assert!(c.initial_grant_workspace.is_none());
        assert!(c.initial_grant_role.is_none());
        assert!(
            c.scopes.is_empty(),
            "no --scope means an empty list (server applies the read-only default)"
        );
    }

    #[test]
    fn api_keys_revoke_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "api-keys",
            "revoke",
            "--key-id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::Revoke(r) = args.command else {
            panic!("expected Revoke");
        };
        assert!(!r.confirm, "--confirm must default to false");
    }

    #[test]
    fn api_keys_revoke_confirm_guard_fires_before_network() {
        let args = ApiKeysRevokeArgs {
            key_id: Uuid::nil(),
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn api_keys_delete_grant_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "api-keys",
            "delete-grant",
            "--key-id",
            "00000000-0000-0000-0000-000000000001",
            "--grant-id",
            "00000000-0000-0000-0000-000000000002",
        ])
        .unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::DeleteGrant(d) = args.command else {
            panic!("expected DeleteGrant");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }

    #[test]
    fn api_keys_delete_grant_confirm_guard_fires_before_network() {
        let args = ApiKeysDeleteGrantArgs {
            key_id: Uuid::nil(),
            grant_id: Uuid::nil(),
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn api_keys_set_global_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "api-keys",
            "set-global",
            "--key-id",
            "00000000-0000-0000-0000-000000000001",
            "--global",
        ])
        .unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::SetGlobal(sg) = args.command else {
            panic!("expected SetGlobal");
        };
        assert!(sg.global);
    }

    #[test]
    fn api_keys_grants_requires_key_id() {
        let result = Cli::try_parse_from(["atlas", "api-keys", "grants"]);
        assert!(result.is_err(), "missing --key-id must fail");
    }

    /// Every `family:action` token in the closed catalog must round-trip through
    /// `parse_scope` into an `ApiKeyScope` variant.
    #[test]
    fn parse_scope_accepts_every_family_action() {
        let all = [
            "tasks:read",
            "tasks:create",
            "tasks:update",
            "tasks:delete",
            "docs:read",
            "docs:create",
            "docs:update",
            "docs:delete",
            "boards:read",
            "boards:create",
            "boards:update",
            "boards:delete",
            "folders:read",
            "folders:create",
            "folders:update",
            "folders:delete",
            "projects:read",
            "projects:create",
            "projects:update",
            "projects:delete",
        ];
        for token in all {
            assert!(parse_scope(token).is_ok(), "expected `{token}` to parse");
        }
    }

    #[test]
    fn parse_scope_rejects_unknown_tokens() {
        for token in ["tasks:manage", "foo:read", "tasks", ""] {
            assert!(
                parse_scope(token).is_err(),
                "expected `{token}` to be rejected"
            );
        }
    }

    #[test]
    fn collect_scopes_rejects_the_whole_list_on_one_bad_token() {
        let raw = vec!["tasks:read".to_owned(), "tasks:manage".to_owned()];
        assert!(collect_scopes(&raw).is_err());
    }

    #[test]
    fn api_keys_create_collects_repeated_scopes() {
        let cli = Cli::try_parse_from([
            "atlas",
            "api-keys",
            "create",
            "--name",
            "k",
            "--scope",
            "tasks:read",
            "--scope",
            "docs:update",
        ])
        .unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.scopes, vec!["tasks:read", "docs:update"]);
    }

    #[test]
    fn api_keys_set_scopes_parses_multiple() {
        let cli = Cli::try_parse_from([
            "atlas",
            "api-keys",
            "set-scopes",
            "--key-id",
            "00000000-0000-0000-0000-000000000001",
            "--scope",
            "tasks:read",
            "--scope",
            "tasks:create",
        ])
        .unwrap();
        let Commands::ApiKeys(args) = cli.command else {
            panic!("expected ApiKeys");
        };
        let ApiKeysCmd::SetScopes(sc) = args.command else {
            panic!("expected SetScopes");
        };
        assert_eq!(sc.key_id, Uuid::from_u128(1));
        assert_eq!(sc.scopes, vec!["tasks:read", "tasks:create"]);
    }

    #[test]
    fn api_keys_set_scopes_requires_at_least_one_scope() {
        let result = Cli::try_parse_from([
            "atlas",
            "api-keys",
            "set-scopes",
            "--key-id",
            "00000000-0000-0000-0000-000000000001",
        ]);
        assert!(result.is_err(), "set-scopes with no --scope must fail");
    }

    #[test]
    fn api_keys_set_scopes_requires_key_id() {
        let result =
            Cli::try_parse_from(["atlas", "api-keys", "set-scopes", "--scope", "tasks:read"]);
        assert!(result.is_err(), "missing --key-id must fail");
    }
}
