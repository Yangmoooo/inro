use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "inro", author = "Yangmoooo")]
#[command(version, propagate_version = true)]
#[command(about = "A personal toolbox for your favorite command-line tools.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Enable verbose output for more details.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    // allows it to be used with any subcommand and makes it a counter. `-v` -> 2, `-vv` -> 2
    pub verbose: u8,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(alias = "i")]
    #[command(about = "Install packages from the sources")]
    Install {
        /// Names of the packages to install
        #[arg(num_args = 1..)]
        names: Vec<String>,
    },

    #[command(aliases = ["rm", "remove"])]
    #[command(about = "Uninstall packages")]
    Uninstall {
        /// Names of the packages to uninstall
        #[arg(num_args = 1..)]
        names: Vec<String>,
    },

    #[command(alias = "ls")]
    #[command(about = "List all installed packages")]
    List,

    #[command(about = "Manage sources")]
    Source {
        #[command(subcommand)]
        command: SourceSubCommand,
    },

    #[command(alias = "s")]
    #[command(about = "Search packages")]
    Search {
        /// Search query
        query: String,
    },

    #[command(alias = "upgrade")]
    #[command(about = "Update packages")]
    Update {
        /// Names of the packages to update
        #[arg(num_args = 0..)]
        names: Vec<String>,
    },

    #[command(name = "self-update")]
    #[command(about = "Update inro itself to the latest version")]
    SelfUpdate,
}

#[derive(Subcommand, Debug)]
pub enum SourceSubCommand {
    #[command(about = "Update local cache of remote sources")]
    Update,

    #[command(alias = "ls")]
    #[command(about = "List configured sources")]
    List,
}
