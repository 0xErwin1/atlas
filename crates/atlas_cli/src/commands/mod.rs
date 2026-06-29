pub(crate) mod search;

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
    }
}
