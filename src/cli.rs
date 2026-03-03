use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "inro", author = "Yangmoooo")]
#[command(version)]
#[command(about = "A personal toolbox for your favorite command-line tools.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Enable verbose output. Use -v for details, -vv for debug logs
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8, // can be used with any command as a counter. `-v` -> 1, `-vv` -> 2
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install packages from configured sources
    ///
    /// Downloads, extracts, and links the package binaries to your local bin
    /// directory. You can specify a version using '@' (e.g.,
    /// 'ripgrep@15.1.0'). If no version is specified, the latest stable
    /// version is installed.
    #[command(alias = "i")]
    Install {
        /// Names of the packages to install (e.g., 'ripgrep', 'just@1.45.0')
        #[arg(num_args = 1.., value_name = "PKG")]
        names: Vec<String>,
    },

    /// Uninstall packages and remove their data
    ///
    /// Removes the package files and unlinks the binaries.
    /// By default, only the currently active version is removed unless --all is
    /// specified.
    #[command(aliases = ["rm", "remove"])]
    Uninstall {
        /// Names of the packages to uninstall
        #[arg(num_args = 1.., value_name = "PKG")]
        names: Vec<String>,

        /// Remove all installed versions of the package, not just the active
        /// one
        #[arg(long)]
        all: bool,
    },

    /// List all installed packages and their versions
    ///
    /// Shows the current active version.
    #[command(alias = "ls")]
    List,

    /// Manage remote package sources (registries)
    Source {
        #[command(subcommand)]
        command: SourceSubCommand,
    },

    /// Search for available packages in the registry
    ///
    /// Searches package names and binary names. Case-insensitive.
    #[command(alias = "s")]
    Search {
        /// The query string to search for
        #[arg(value_name = "QUERY")]
        query: String,
    },

    /// Show detailed information about a package
    ///
    /// Displays local status (installed versions) and fetches remote info
    /// (latest versions).
    Show {
        /// Package name to inspect
        #[arg(value_name = "PKG")]
        name: String,
    },

    /// Switch to a specific version of an installed package
    ///
    /// Updates the symlinks to point to the specified version.
    Use {
        /// Package name
        #[arg(value_name = "PKG")]
        name: String,
        /// The version to switch to (e.g., '1.2.3')
        #[arg(value_name = "VERSION")]
        version: String,
    },

    /// Temporarily unlink a package from your PATH
    ///
    /// Removes the symlinks but keeps the package files on disk.
    /// Use 'inro use' to re-enable it later.
    Unlink {
        /// Package name to unlink
        #[arg(value_name = "PKG")]
        name: String,
    },

    /// Update installed packages to their latest versions
    ///
    /// Checks remote sources for newer versions. If found, installs the new
    /// version and switches to it. Old versions are kept as backups.
    #[command(alias = "upgrade")]
    Update {
        /// Names of packages to update. If omitted, updates all installed
        /// packages.
        #[arg(num_args = 0.., value_name = "PKG")]
        names: Vec<String>,
    },

    /// Clean up unused package versions to free up disk space
    ///
    /// Removes versions that are not currently active (linked).
    /// Use --dry-run to see what would be deleted without actually deleting
    /// anything.
    Clean {
        /// Perform a dry run without deleting any files
        #[arg(long)]
        dry_run: bool,
    },

    /// Generate shell completions and man pages
    #[command(hide = true)]
    Generate {
        /// The type of asset to generate
        #[arg(value_enum)]
        generator: Generator,

        /// Output directory
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum SourceSubCommand {
    /// Download and update the local cache of remote package definitions
    Update,

    /// List all configured remote sources and their status
    #[command(alias = "ls")]
    List,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Generator {
    Man,
    Bash,
    Zsh,
    Fish,
    PowerShell,
}
