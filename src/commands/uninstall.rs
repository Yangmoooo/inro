use std::fs;

use anyhow::{Context, Result};
use colored::Colorize;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::PkgReceipt;
use crate::report;
use crate::utils::unique;

pub struct UninstallCommand {
    pub names: Vec<String>,
}

struct UninstallReceipt {
    name: String,
    version: String,
    fully_removed: bool,
}

impl CommandHandler for UninstallCommand {
    fn handle(&self) -> Result<()> {
        let names = unique(&self.names);
        report!(MsgType::Info, "Starting uninstallation of {} package(s)...", names.len());

        let layout = InroLayout::new()?;
        let manifest_path = &layout.manifest_path;
        let mut manifest = Manifest::load(manifest_path)?;

        if manifest.pkgs.is_empty() {
            report!(MsgType::Warning, "No packages are currently installed");
            return Ok(());
        }

        let mut successes = Vec::new();
        let mut failures = Vec::new();

        for name in &names {
            match do_uninstall(name, &mut manifest) {
                Ok(Some(receipt)) => successes.push(receipt),
                Ok(None) => {
                    // package is not installed
                    report!(MsgType::Warning, "Package '{name}' is not installed");
                    failures.push((name.clone(), "Package not installed".to_string()));
                }
                Err(e) => {
                    report!(MsgType::Error, "Failed to uninstall '{name}': {e:?}");
                    failures.push((name.clone(), e.to_string()));
                }
            }
        }

        if !successes.is_empty() {
            manifest.save(manifest_path)?;
            report!(MsgType::Detail, "Manifest updated");
        }

        // summary
        eprintln!();
        let has_success = !successes.is_empty();
        let has_failure = !failures.is_empty();

        if has_success {
            report!(MsgType::Success, "Successfully uninstalled {} package(s):", successes.len());

            let max_name_len = successes.iter().map(|r| r.name.len()).max().unwrap_or(0);

            for receipt in &successes {
                let status_note = if receipt.fully_removed { "(fully removed)" } else { "" };

                eprintln!(
                    "  {} {:<width$} {} {}",
                    "-".green(),
                    receipt.name.bold(),
                    receipt.version.italic(),
                    status_note.dimmed(),
                    width = max_name_len
                );
            }
        }

        if has_failure {
            if has_success {
                eprintln!();
            }

            report!(MsgType::Error, "Failed to uninstall {} package(s):", failures.len());

            let max_name_len = failures.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
            for (name, reason) in &failures {
                eprintln!(
                    "  {} {:<width$} : {}",
                    "•".red(),
                    name.bold(),
                    reason,
                    width = max_name_len
                );
            }
        }

        if !has_success && !has_failure {
            report!(MsgType::Warning, "Nothing to do");
            return Ok(());
        }

        if has_failure {
            std::process::exit(1);
        }

        Ok(())
    }
}

fn do_uninstall(name: &str, manifest: &mut Manifest) -> Result<Option<UninstallReceipt>> {
    // check if installed
    let Some(state) = manifest.pkgs.get(name) else {
        return Ok(None);
    };

    // determine version
    let version = match &state.current_version {
        Some(v) => v.clone(),
        None => {
            // MARK: installed but no active version
            anyhow::bail!("No active version to uninstall");
        }
    };

    report!(MsgType::Step, "Processing package '{name}' ({version}) ...");

    // remove from manifest
    if let Some(receipt) = manifest.remove_version(name, &version) {
        // remove files
        cleanup_files(&receipt)?;

        let fully_removed = !manifest.pkgs.contains_key(name);
        Ok(Some(UninstallReceipt { name: name.to_string(), version, fully_removed }))
    } else {
        anyhow::bail!("Version not found in manifest");
    }
}

fn cleanup_files(receipt: &PkgReceipt) -> Result<()> {
    // remove symbolic link
    for bin in &receipt.binaries {
        if bin.link_path.exists() || bin.link_path.is_symlink() {
            fs::remove_file(&bin.link_path).with_context(|| {
                format!("Failed to remove symlink: {}", bin.link_path.display())
            })?;
            report!(MsgType::Detail, "Removed link: {}", bin.link_path.display());
        }
    }

    // remove data install dir
    if receipt.install_dir.exists() {
        fs::remove_dir_all(&receipt.install_dir).with_context(|| {
            format!("Failed to remove data dir: {}", receipt.install_dir.display())
        })?;
        report!(MsgType::Detail, "Removed data: {}", receipt.install_dir.display());
    }

    // if package dir is empty, remove it
    if let Some(parent) = receipt.install_dir.parent() {
        let _ = fs::remove_dir(parent);
    }

    Ok(())
}
