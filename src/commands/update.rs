use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::installer::{find_best_candidate, install_candidate};
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::{PkgError, PkgReceipt};
use crate::registry::Registry;
use crate::report;
use crate::utils::unique;

pub struct UpdateCommand {
    pub names: Vec<String>,
    pub layout: InroLayout,
}

struct UpdateReceipt {
    name: String,
    old_version: String,
    new_version: String,
    full_receipt: PkgReceipt,
}

enum UpdateStatus {
    Updated(Box<UpdateReceipt>),
    Skipped,
    NotInstalled,
    Failed(String, String),
}

impl CommandHandler for UpdateCommand {
    fn handle(&self) -> Result<()> {
        let config = Config::load(&self.layout)?;
        let registry = Registry::load(&self.layout)?;
        let mut manifest = Manifest::load(&self.layout.manifest_path)?;

        let names: Vec<String> = if self.names.is_empty() {
            manifest.pkgs.keys().cloned().collect()
        } else {
            unique(&self.names)
        };

        let mut results = Vec::new();
        let mut any_updated = false;

        for name in &names {
            let res = check_and_update(name, &manifest, &registry, &config, &self.layout);
            match res {
                Ok(UpdateStatus::Updated(ref receipt)) => {
                    manifest.add(receipt.full_receipt.clone());
                    if let Err(e) = receipt.full_receipt.save_to_install_dir() {
                        report!(
                            MsgType::Warning,
                            "Failed to save backup receipt for '{name}': {e}"
                        );
                    }
                    any_updated = true;
                    results.push(res.unwrap()); // res is ok here
                }
                Ok(_) => (),
                Err(e) => {
                    report!(MsgType::Error, "Failed to update '{name}': {e}");
                    results.push(UpdateStatus::Failed(name.to_string(), e.to_string()));
                }
            }
        }

        if any_updated {
            manifest.save(&self.layout.manifest_path)?;
            report!(MsgType::Detail, "Manifest updated");
        }

        print_summary(&results);

        Ok(())
    }
}

fn check_and_update(
    name: &str,
    manifest: &Manifest,
    registry: &Registry,
    config: &Config,
    layout: &InroLayout,
) -> Result<UpdateStatus> {
    let state = match manifest.pkgs.get(name) {
        Some(s) => s,
        None => {
            report!(MsgType::Warning, "'{name}' not installed, skipping");
            return Ok(UpdateStatus::NotInstalled);
        }
    };

    let current_ver = state.current_version.as_deref().unwrap_or_default();

    let pkg_def = registry.pkgs.get(name).ok_or(PkgError::NotFound(name.to_string()))?;

    let candidate = find_best_candidate(pkg_def)?;

    if candidate.version == current_ver {
        report!(MsgType::Info, "'{name}' is up to date ({current_ver})");
        return Ok(UpdateStatus::Skipped);
    }

    report!(MsgType::Step, "Updating '{name}': {current_ver} -> {}", candidate.version);

    let pkg = pkg_def.clone().resolve(name);
    let receipt = install_candidate(name, &candidate, &pkg, config, layout)?;

    Ok(UpdateStatus::Updated(Box::new(UpdateReceipt {
        name: name.to_string(),
        old_version: current_ver.to_string(),
        new_version: candidate.version,
        full_receipt: receipt,
    })))
}

fn print_summary(results: &[UpdateStatus]) {
    let mut updated = Vec::new();
    let mut failed = Vec::new();

    for res in results {
        match res {
            UpdateStatus::Updated(r) => updated.push(r),
            UpdateStatus::Failed(name, error) => failed.push((name, error)),
            _ => (),
        }
    }

    eprintln!();

    if !updated.is_empty() {
        report!(MsgType::Success, "Updated {} package(s):", updated.len());
        let max_len = updated.iter().map(|r| r.name.len()).max().unwrap_or(0);
        for r in &updated {
            eprintln!(
                "  {} {:<max_len$} : {} -> {}",
                "+".green(),
                r.name.bold(),
                r.old_version.dimmed(),
                r.new_version,
            );
        }
    }

    if !failed.is_empty() {
        if !updated.is_empty() {
            eprintln!();
        }

        report!(MsgType::Error, "Failed to update {} package(s):", failed.len());
        let max_len = failed.iter().map(|t| t.0.len()).max().unwrap_or(0);
        for (name, err) in failed {
            eprintln!("  {} {:<max_len$} : {}", "•".red(), name.bold(), err);
        }
        std::process::exit(1);
    }

    if updated.is_empty() && failed.is_empty() {
        report!(MsgType::Success, "All packages are up to date.");
    }
}
