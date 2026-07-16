use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::installer::{BatchOutcome, InstallRequest, execute_install_batch};
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::registry::Registry;
use crate::utils::{ensure_unique_package_args, parse_package_version};
use crate::warn;

pub struct InstallCommand {
    /// Package names, optionally with version (e.g. "ripgrep@15.1.0")
    pub names: Vec<String>,
}

impl CommandHandler for InstallCommand {
    fn handle(&self) -> Result<()> {
        ensure_unique_package_args(&self.names)?;

        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let config = Config::load(&layout)?;
        let registry = Registry::load(&layout)?;
        if registry.pkgs.is_empty() {
            eprintln!(
                "{} Registry is empty. Run 'inro source update' to fetch packages",
                "warning:".yellow().bold()
            );
            return Ok(());
        }
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        let requests = self
            .names
            .iter()
            .map(|n| {
                let (name, version) = parse_package_version(n);
                InstallRequest::install(name.to_string(), version.map(str::to_string))
            })
            .collect();
        let BatchOutcome { receipts, write_backs, failed, unchanged: _ } =
            execute_install_batch(requests, &registry, &config, &layout)?;
        let success_count = receipts.len();
        for receipt in receipts {
            manifest.add(receipt);
        }

        if !write_backs.is_empty()
            && let Err(e) = Registry::write_asset_selections(&layout, &write_backs)
        {
            warn!("Failed to save asset selections: {e}");
        }

        manifest.save(&layout.manifest_path)?;
        print_summary(success_count, failed);

        if failed > 0 {
            anyhow::bail!("{failed} package(s) failed to install");
        }
        Ok(())
    }
}

fn print_summary(success: usize, failed: usize) {
    eprintln!();
    if success > 0 && failed == 0 {
        eprintln!("{} Installed {} package(s)", "✓".green().bold(), success);
    } else if success > 0 && failed > 0 {
        eprintln!("{} Installed {} package(s), {} failed", "!".yellow().bold(), success, failed);
    } else if failed > 0 {
        eprintln!("{} All {} package(s) failed", "✗".red().bold(), failed);
    } else {
        eprintln!("{} Nothing to install", "·".dimmed());
    }
}
