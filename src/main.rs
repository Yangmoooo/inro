mod cli;
mod commands;
mod config;
mod dan;
mod layout;
mod manifest;
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
    let layout = layout::InroLayout::new()?;

    if let Some(command) = cli.command {
        let command_handler: Box<dyn CommandHandler> = match command {
            Command::Install { names } => Box::new(InstallCommand { names, layout }),
            Command::Uninstall { names } => Box::new(UninstallCommand { names, layout }),
            Command::List => Box::new(ListCommand { layout }),
            Command::Source { command } => Box::new(SourceCommand { command, layout }),
            Command::Search { query } => Box::new(SearchCommand { query, layout }),
            Command::Info { name } => Box::new(InfoCommand { name, layout }),
            Command::Use { name, version } => Box::new(UseCommand { name, version, layout }),
            Command::Unlink { name } => Box::new(UnlinkCommand { name, layout }),

            _ => anyhow::bail!("Not implemented yet!"),
        };

        command_handler.handle()?;
    } else {
        Cli::command().print_long_help()?;
    }

    Ok(())
}
