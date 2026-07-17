mod archive;
mod cli;
mod client;
mod commands;
mod config;
mod installer;
mod layout;
mod lock;
mod manifest;
mod package;
mod package_set;
mod platform;
mod progress;
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
        match command {
            Command::Install { names } => InstallCommand { names }.handle(),
            Command::Uninstall { names, all } => UninstallCommand { names, all }.handle(),
            Command::List => ListCommand {}.handle(),
            Command::Update { names, force } => UpdateCommand { names, force }.handle(),
            Command::Source { command } => SourceCommand { command }.handle(),
            Command::Search { query } => SearchCommand { query }.handle(),
            Command::Pin { name } => PinCommand { name }.handle(),
            Command::Unpin { name } => UnpinCommand { name }.handle(),
            Command::Show { name } => ShowCommand { name }.handle(),
            Command::Use { name, version } => UseCommand { name, version }.handle(),
            Command::Unlink { name } => UnlinkCommand { name }.handle(),
            Command::Clean { dry_run } => CleanCommand { dry_run }.handle(),
            Command::Doctor { fix } => DoctorCommand { fix }.handle(),
            Command::Env => EnvCommand {}.handle(),
            Command::Export { output } => ExportCommand { output }.handle(),
            Command::Import { path } => ImportCommand { path }.handle(),
            Command::Generate { generator, out } => GenerateCommand { generator, out }.handle(),
        }?;
    } else {
        Cli::command().print_long_help()?;
    }

    Ok(())
}
