use std::fmt::Write;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "inro", author = "Yangmoooo")]
#[command(version)]
#[command(about = "A personal toolbox for your favorite command-line tools.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Enable verbose output. Use -v for details, -vv for debug tracing
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8, // can be used with any command as a counter. `-v` -> 1, `-vv` -> 2
}

pub fn command_with_grouped_help() -> clap::Command {
    const GROUPS: &[&[&str]] = &[
        &["install", "update", "uninstall"],
        &["use", "unlink", "relink", "pin", "unpin"],
        &["list", "search", "show", "source"],
        &["export", "import"],
        &["clean", "doctor", "env"],
        &["help"],
    ];

    let mut command = Cli::command();
    command.build();
    command = command.mut_subcommand("source", |source| with_grouped_help(source, &[]));
    with_grouped_help(command, GROUPS)
}

fn with_grouped_help(command: clap::Command, groups: &[&[&str]]) -> clap::Command {
    let mut entries: Vec<_> = command
        .get_subcommands()
        .filter(|subcommand| !subcommand.is_hide_set())
        .map(|subcommand| {
            let order = groups
                .iter()
                .enumerate()
                .find_map(|(group, names)| {
                    names
                        .iter()
                        .position(|name| *name == subcommand.get_name())
                        .map(|index| (group, index))
                })
                // New commands remain visible even before being assigned a group.
                .unwrap_or((groups.len(), subcommand.get_display_order()));
            let names = std::iter::once(subcommand.get_name())
                .chain(subcommand.get_visible_aliases())
                .collect::<Vec<_>>()
                .join(", ");
            (order, subcommand, names)
        })
        .collect();
    entries.sort_by_key(|(order, ..)| *order);
    let width = entries.iter().map(|(_, _, names)| names.len()).max().unwrap_or(0);
    let header = command.get_styles().get_header();
    let literal = command.get_styles().get_literal();

    // Let clap render usage and options; only the command list is custom.
    let mut help = command
        .clone()
        .help_template("{before-help}{about-with-newline}\n{usage-heading} {usage}")
        .render_help();
    writeln!(help, "\n{header}Commands:{header:#}").unwrap();
    let mut previous_group = None;
    for ((group, _), subcommand, names) in entries {
        if previous_group.is_some_and(|previous| previous != group) {
            help.push_str("\n");
        }
        let about = subcommand.get_about().or_else(|| subcommand.get_long_about());
        write!(help, "  {literal}{names:width$}{literal:#}  ").unwrap();
        if let Some(about) = about {
            write!(help, "{}", about.ansi()).unwrap();
        }
        help.push_str("\n");
        previous_group = Some(group);
    }
    writeln!(help, "\n{header}Options:{header:#}").unwrap();
    let options = command.clone().help_template("{options}{after-help}").render_help();
    write!(help, "{}", options.ansi()).unwrap();
    command.override_help(help)
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install packages from configured sources
    ///
    /// Downloads, extracts, and links the package binaries to your local bin
    /// directory. You can specify a version using '@' (e.g.,
    /// 'ripgrep@15.1.0'). If no version is specified, the latest stable
    /// version is installed.
    #[command(visible_alias = "i")]
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
    #[command(visible_aliases = ["rm", "remove"])]
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
    #[command(visible_alias = "ls")]
    List,

    /// Manage remote package sources (registries)
    Source {
        #[command(subcommand)]
        command: SourceSubCommand,
    },

    /// Search for available packages in the registry
    ///
    /// Searches package names and binary names. Case-insensitive.
    #[command(visible_alias = "s")]
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

    /// Apply registry binary definitions to an active package's links
    ///
    /// Uses files already installed for the active version, without
    /// downloading. Adds, removes, renames, or repairs links and updates
    /// the installation record. For an unlinked package, first select a
    /// version with 'inro use <PKG> <VERSION>'.
    Relink {
        /// Package whose active version should use the current registry binary
        /// definitions
        #[arg(value_name = "PKG")]
        name: String,
    },

    /// Update installed packages to their latest versions
    ///
    /// Checks remote sources for newer versions. If found, installs the new
    /// version and switches to it. Old versions are kept as backups.
    /// Direct packages are skipped unless
    /// an exact version is specified, e.g. 'inro update mysql@8.4.11'.
    #[command(visible_alias = "upgrade")]
    Update {
        /// Names of packages to update. If omitted, updates all installed
        /// packages.
        #[arg(num_args = 0.., value_name = "PKG")]
        names: Vec<String>,

        /// Force update even if the package is pinned (pin state is preserved)
        #[arg(short, long)]
        force: bool,
    },

    /// Pin a package to prevent it from being auto-updated
    ///
    /// Pinned packages are skipped when running 'inro update'.
    /// Use 'inro unpin' to allow updates again.
    Pin {
        /// Package name to pin
        #[arg(value_name = "PKG")]
        name: String,
    },

    /// Unpin a package to allow it to be auto-updated again
    Unpin {
        /// Package name to unpin
        #[arg(value_name = "PKG")]
        name: String,
    },

    /// Clean up unused package versions or remembered asset selections
    ///
    /// Removes versions that are not currently active (linked).
    /// Use --dry-run to see what would be deleted without actually deleting
    /// anything.
    /// With --assets, only clears remembered asset selections, across all
    /// platforms. Specify a package to reset its selections, or omit it to
    /// reset all selections.
    Clean {
        /// Clear remembered asset selections instead of unused package versions
        #[arg(long)]
        assets: bool,

        /// Limit --assets cleanup to one package
        #[arg(value_name = "PKG", requires = "assets")]
        name: Option<String>,

        /// Perform a dry run without deleting any files
        #[arg(long)]
        dry_run: bool,
    },

    /// Check the health of your inro setup
    ///
    /// Verifies config parsing, source files, bin_dir PATH membership,
    /// symlink integrity, and manifest consistency.
    /// Use --fix to automatically repair bin_dir mismatches.
    Doctor {
        /// Automatically re-link binaries whose symlinks don't match the
        /// configured bin_dir
        #[arg(long)]
        fix: bool,
    },

    /// Print the resolved INRO_HOME and all derived paths
    ///
    /// Useful for debugging path resolution, scripting, or setting up
    /// dotfiles. Respects the INRO_HOME environment variable.
    Env,

    /// Export active packages as reproducible package specifications
    Export {
        /// Write the package set to a file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<PathBuf>,
    },

    /// Install exact package versions from an exported package set
    Import {
        /// Package set file to import
        #[arg(value_name = "PATH")]
        path: PathBuf,
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
    #[command(visible_alias = "ls")]
    List {
        /// Check remote registries for updates
        #[arg(long)]
        check_remote: bool,
    },

    /// Enable a source registry
    Enable {
        /// Name of the source to enable
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Disable a source registry
    Disable {
        /// Name of the source to disable
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Edit a hand-written local registry file
    ///
    /// Uses VISUAL or EDITOR when set; the editor command must wait until the
    /// file is closed before exiting.
    Edit {
        /// Registry file name without the .toml extension
        #[arg(value_name = "NAME")]
        name: Option<String>,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Generator {
    Man,
    Bash,
    Zsh,
    Fish,
    PowerShell,
}
