mod cli;
mod commands;
mod config;
mod installer;
mod layout;
mod manifest;
mod package;
mod platform;
mod registry;
mod remotes;
mod reporter;
mod utils;

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use clap::{CommandFactory, Parser};
use cli::{Cli, Command};
use commands::*;

static VERBOSITY: LazyLock<AtomicU8> = LazyLock::new(AtomicU8::default);

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    VERBOSITY.store(cli.verbose, Ordering::Relaxed);

    if let Some(command) = cli.command {
        let command_handler: Box<dyn CommandHandler> = match command {
            Command::Install { names } => Box::new(InstallCommand { names }),
            Command::Uninstall { names, all } => Box::new(UninstallCommand { names, all }),
            Command::List => Box::new(ListCommand {}),
            Command::Update { names } => Box::new(UpdateCommand { names }),
            Command::Source { command } => Box::new(SourceCommand { command }),
            Command::Search { query } => Box::new(SearchCommand { query }),
            Command::Info { name } => Box::new(InfoCommand { name }),
            Command::Use { name, version } => Box::new(UseCommand { name, version }),
            Command::Unlink { name } => Box::new(UnlinkCommand { name }),
            Command::Clean { dry_run } => Box::new(CleanCommand { dry_run }),

            _ => anyhow::bail!("Not implemented yet!"),
        };

        command_handler.handle()?;
    } else {
        Cli::command().print_long_help()?;
    }

    Ok(())
}
