mod cli;
mod command;
mod package;
mod platform;
mod reporter;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::LazyLock;

use clap::{CommandFactory, Parser};

use cli::{Cli, Command};
use command::*;

static VERBOSITY: LazyLock<AtomicU8> = LazyLock::new(AtomicU8::default);

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    VERBOSITY.store(cli.verbose, Ordering::Relaxed);

    if let Some(command) = cli.command {
        let command_handler: Box<dyn CommandHandler> = match command {

            Command::Install { names } => Box::new(InstallCommand { names }),

            _ => anyhow::bail!("Not implemented yet!"),
        };

        command_handler.handle()?;
    } else {
        Cli::command().print_long_help()?;
    }

    Ok(())
}
