mod commands;
mod output;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::commands::Cli;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    let filter = if cli.quiet {
        "error"
    } else if cli.verbose > 0 {
        "cb_cli=debug,cb_adapters=debug,sqlx=warn"
    } else {
        "cb_cli=warn,cb_adapters=warn,sqlx=error"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .with_ansi(!cli.no_color)
        .try_init()
        .map_err(|error| miette::miette!("could not initialize logging: {error}"))?;

    let exit_code = commands::execute(cli).await?;
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}
