pub(crate) mod activity;
pub(crate) mod api_keys;
pub(crate) mod boards;
pub(crate) mod columns;
pub(crate) mod docs;
pub(crate) mod folders;
pub(crate) mod groups;
pub(crate) mod members;
pub(crate) mod projects;
pub(crate) mod search;
pub(crate) mod tags;
pub(crate) mod tasks;
pub(crate) mod users;
pub(crate) mod workspaces;

use crate::cli::Commands;
use crate::ctx::Ctx;
use crate::error::CliError;

/// Dispatches a parsed command to its handler module.
pub(crate) async fn dispatch(ctx: &Ctx, cmd: Commands) -> Result<(), CliError> {
    match cmd {
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Commands::Search(args) => search::run(ctx, args).await,
        Commands::Tasks(args) => tasks::run(ctx, args.command).await,
        Commands::Docs(args) => docs::run(ctx, args.command).await,
        Commands::Workspaces(args) => workspaces::run(ctx, args.command).await,
        Commands::Projects(args) => projects::run(ctx, args.command).await,
        Commands::Boards(args) => boards::run(ctx, args.command).await,
        Commands::Columns(args) => columns::run(ctx, args.command).await,
        Commands::Tags(args) => tags::run(ctx, args.command).await,
        Commands::Members(args) => members::run(ctx, args.command).await,
        Commands::Folders(args) => folders::run(ctx, args.command).await,
        Commands::Activity(args) => activity::run(ctx, args.command).await,
        Commands::Users(args) => users::run(ctx, args.command).await,
        Commands::ApiKeys(args) => api_keys::run(ctx, args.command).await,
        Commands::Groups(args) => groups::run(ctx, args.command).await,
    }
}
