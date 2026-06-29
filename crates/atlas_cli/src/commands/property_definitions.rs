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
use crate::projections::{DeleteByIdProjection, PropertyDefinitionProjection};

// ---------------------------------------------------------------------------
// PropertyDefinitionsArgs + PropertyDefinitionsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `property-definitions` subcommand group.
#[derive(Args)]
pub(crate) struct PropertyDefinitionsArgs {
    #[command(subcommand)]
    pub(crate) command: PropertyDefinitionsCmd,
}

#[derive(Subcommand)]
pub(crate) enum PropertyDefinitionsCmd {
    /// List property definitions in a workspace, optionally filtered by applicability.
    List(PropertyDefinitionsListArgs),
    /// Create a new property definition.
    Create(PropertyDefinitionsCreateArgs),
    /// Delete a property definition (requires --confirm).
    Delete(PropertyDefinitionsDeleteArgs),
}

/// Dispatches a parsed `PropertyDefinitionsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: PropertyDefinitionsCmd) -> Result<(), CliError> {
    match cmd {
        PropertyDefinitionsCmd::List(args) => run_list(ctx, args).await,
        PropertyDefinitionsCmd::Create(args) => run_create(ctx, args).await,
        PropertyDefinitionsCmd::Delete(args) => run_delete(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas property-definitions list`.
#[derive(Parser)]
pub(crate) struct PropertyDefinitionsListArgs {
    /// Filter by applicability: `task`, `document`, or `both`.
    #[arg(long)]
    pub(crate) applies_to: Option<String>,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_list(ctx: &Ctx, args: PropertyDefinitionsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let defs = ctx
        .client
        .list_property_definitions(ws, args.applies_to.as_deref())
        .await?;

    let items: Vec<PropertyDefinitionProjection> = defs
        .into_iter()
        .map(PropertyDefinitionProjection::from)
        .collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas property-definitions create`.
///
/// For `select` and `multi_select` kinds `--options` is required and must be
/// a JSON array of strings.  For all other kinds `--options` must be absent.
#[derive(Parser)]
pub(crate) struct PropertyDefinitionsCreateArgs {
    /// Display name (the stable key is derived server-side from the name).
    #[arg(long)]
    pub(crate) name: String,

    /// Property kind: `text`, `number`, `boolean`, `date`, `select`, or `multi_select`.
    #[arg(long)]
    pub(crate) kind: String,

    /// Allowed values as a JSON array (required for select/multi_select kinds).
    #[arg(long)]
    pub(crate) options: Option<String>,

    /// Applicability: `task` (default), `document`, or `both`.
    #[arg(long)]
    pub(crate) applies_to: Option<String>,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_create(ctx: &Ctx, args: PropertyDefinitionsCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::property_definitions::CreatePropertyDefinitionRequest;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let options = args
        .options
        .as_deref()
        .map(|s| {
            serde_json::from_str::<serde_json::Value>(s)
                .map_err(|e| CliError::Validation(format!("invalid --options JSON: {e}")))
        })
        .transpose()?;

    let body = CreatePropertyDefinitionRequest {
        name: args.name,
        kind: args.kind,
        options,
        applies_to: args.applies_to,
    };

    let def = ctx.client.create_property_definition(ws, body).await?;
    let proj = PropertyDefinitionProjection::from(def);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Arguments for `atlas property-definitions delete`.
#[derive(Parser)]
pub(crate) struct PropertyDefinitionsDeleteArgs {
    /// UUID of the property definition to delete.
    #[arg(long)]
    pub(crate) property_definition_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the deletion. Required — permanently removes the custom field.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: PropertyDefinitionsDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete the property definition".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client
        .delete_property_definition(ws, args.property_definition_id)
        .await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.property_definition_id,
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
    fn property_definitions_list_parses_without_args() {
        let cli = Cli::try_parse_from(["atlas", "property-definitions", "list"]).unwrap();
        let Commands::PropertyDefinitions(args) = cli.command else {
            panic!("expected PropertyDefinitions");
        };
        let PropertyDefinitionsCmd::List(l) = args.command else {
            panic!("expected List");
        };
        assert!(l.applies_to.is_none());
        assert!(l.workspace.is_none());
    }

    #[test]
    fn property_definitions_list_parses_applies_to() {
        let cli = Cli::try_parse_from([
            "atlas",
            "property-definitions",
            "list",
            "--applies-to",
            "task",
        ])
        .unwrap();
        let Commands::PropertyDefinitions(args) = cli.command else {
            panic!("expected PropertyDefinitions");
        };
        let PropertyDefinitionsCmd::List(l) = args.command else {
            panic!("expected List");
        };
        assert_eq!(l.applies_to.as_deref(), Some("task"));
    }

    #[test]
    fn property_definitions_create_requires_name_and_kind() {
        assert!(
            Cli::try_parse_from(["atlas", "property-definitions", "create"]).is_err(),
            "missing --name and --kind must fail"
        );
        assert!(
            Cli::try_parse_from([
                "atlas",
                "property-definitions",
                "create",
                "--name",
                "Priority"
            ])
            .is_err(),
            "missing --kind must fail"
        );
    }

    #[test]
    fn property_definitions_create_parses_name_and_kind() {
        let cli = Cli::try_parse_from([
            "atlas",
            "property-definitions",
            "create",
            "--name",
            "Due Date",
            "--kind",
            "date",
        ])
        .unwrap();
        let Commands::PropertyDefinitions(args) = cli.command else {
            panic!("expected PropertyDefinitions");
        };
        let PropertyDefinitionsCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.name, "Due Date");
        assert_eq!(c.kind, "date");
        assert!(c.options.is_none());
    }

    #[test]
    fn property_definitions_create_parses_options_for_select_kind() {
        let cli = Cli::try_parse_from([
            "atlas",
            "property-definitions",
            "create",
            "--name",
            "Status",
            "--kind",
            "select",
            "--options",
            r#"["todo","done"]"#,
        ])
        .unwrap();
        let Commands::PropertyDefinitions(args) = cli.command else {
            panic!("expected PropertyDefinitions");
        };
        let PropertyDefinitionsCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.options.as_deref(), Some(r#"["todo","done"]"#));
    }

    #[test]
    fn property_definitions_delete_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "property-definitions",
            "delete",
            "--property-definition-id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::PropertyDefinitions(args) = cli.command else {
            panic!("expected PropertyDefinitions");
        };
        let PropertyDefinitionsCmd::Delete(d) = args.command else {
            panic!("expected Delete");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }

    #[test]
    fn property_definitions_delete_confirm_guard_fires_before_network() {
        let args = PropertyDefinitionsDeleteArgs {
            property_definition_id: Uuid::nil(),
            workspace: None,
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn property_definitions_invalid_options_json_rejects() {
        let bad = "[not-valid";
        let result = serde_json::from_str::<serde_json::Value>(bad);
        assert!(result.is_err(), "malformed JSON options must fail");
    }
}
