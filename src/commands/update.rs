use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::installer::{BatchOutcome, InstallRequest, execute_install_batch};
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::progress::ProgressManager;
use crate::registry::Registry;
use crate::utils::{ensure_unique_package_args, parse_package_version};
use crate::warn;

pub struct UpdateCommand {
    pub names: Vec<String>,
    pub force: bool,
}

impl CommandHandler for UpdateCommand {
    fn handle(&self) -> Result<()> {
        ensure_unique_package_args(&self.names)?;
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let config = Config::load(&layout)?;
        let registry = Registry::load(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        let names: Vec<String> = if self.names.is_empty() {
            manifest.pkgs.keys().cloned().collect()
        } else {
            self.names.clone()
        };

        if names.is_empty() {
            eprintln!("{} No packages installed", "·".dimmed());
            return Ok(());
        }

        // Parse and collect package names
        let parsed: Vec<_> = names
            .iter()
            .map(|n| {
                let (pkg_name, pkg_ver) = parse_package_version(n);
                (pkg_name.to_string(), pkg_ver.map(|s| s.to_string()))
            })
            .collect();

        let pkg_names: Vec<&str> = parsed.iter().map(|(name, _)| name.as_str()).collect();
        let pm = ProgressManager::new(&pkg_names);

        let mut requests = Vec::new();
        let mut not_installed = 0usize;

        for (pkg_name, pkg_ver) in &parsed {
            match manifest.pkgs.get(pkg_name) {
                Some(state) => {
                    if state.pinned && !self.force {
                        pm.add_package(pkg_name).finish_error("pinned, skipping");
                        continue;
                    }
                    let Some(current_ver) = state.current_version.as_deref() else {
                        pm.add_package(pkg_name).finish_error("unlinked, skipping");
                        continue;
                    };
                    requests.push(InstallRequest::update(
                        pkg_name.clone(),
                        pkg_ver.clone(),
                        current_ver.to_string(),
                    ));
                }
                None => {
                    pm.add_package(pkg_name).finish_error("not installed");
                    not_installed += 1;
                }
            }
        }

        let BatchOutcome { receipts, write_backs, unchanged, failed: batch_failed } =
            execute_install_batch(requests, &pm, &registry, &config, &layout)?;
        let updated = receipts.len();
        let failed = not_installed + batch_failed;
        for receipt in receipts {
            manifest.add(receipt);
        }

        if !write_backs.is_empty()
            && let Err(e) = Registry::write_asset_selections(&layout, &write_backs)
        {
            warn!("Failed to save asset selections: {e}");
        }

        if updated > 0 {
            manifest.save(&layout.manifest_path)?;
        }

        print_summary(updated, unchanged, failed);

        if failed > 0 {
            anyhow::bail!("{failed} package(s) failed to update");
        }
        Ok(())
    }
}

fn print_summary(updated: usize, up_to_date: usize, failed: usize) {
    eprintln!();
    if updated > 0 && failed == 0 {
        eprintln!("{} Updated {} package(s)", "✓".green().bold(), updated);
    } else if updated > 0 && failed > 0 {
        eprintln!("{} Updated {} package(s), {} failed", "!".yellow().bold(), updated, failed);
    } else if failed > 0 {
        eprintln!("{} All {} package(s) failed", "✗".red().bold(), failed);
    } else if up_to_date > 0 {
        eprintln!("{} All {} package(s) are up to date", "✓".green().bold(), up_to_date);
    } else {
        eprintln!("{} Nothing to update", "·".dimmed());
    }
}
