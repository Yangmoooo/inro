use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "inro", author = "Yangmoooo")]
#[command(version, propagate_version = true)]
#[command(about = "A personal toolbox for your favorite command-line tools.")]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(alias = "i")]
    #[command(about = "Install packages from the source.")]
    Install {
        /// Names of the packages to install
        #[arg(num_args = 1..)]
        names: Vec<String>,
    },

    #[command(aliases = ["rm", "remove"])]
    #[command(about = "Uninstall packages.")]
    Uninstall {
        /// Names of the packages to uninstall
        #[arg(num_args = 1..)]
        names: Vec<String>,
    },

    #[command(alias = "ls")]
    #[command(about = "List all installed packages.")]
    List,

    #[command(alias = "up")]
    #[command(about = "Update packages.")]
    Update {
        /// Names of the packages to update
        #[arg(num_args = 0..)]
        names: Vec<String>,
    },

    #[command(name = "self-update")]
    #[command(about = "Update inro itself to the latest version.")]
    SelfUpdate,
}
