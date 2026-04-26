use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::PkgReceipt;
use crate::utils::{parse_package_version, unique};
use crate::{detail, done, fail, hint, step, warn};

pub struct UninstallCommand {
    pub names: Vec<String>,
    pub all: bool,
}

struct UninstallReceipt {
    name: String,
    version: String,
    fully_removed: bool,
}

impl CommandHandler for UninstallCommand {
    fn handle(&self) -> Result<()> {
        let names = unique(&self.names);
        hint!("Starting uninstallation of {} package(s)...", names.len());

        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let config = Config::load(&layout)?;
        let manifest_path = &layout.manifest_path;
        let mut manifest = Manifest::load(manifest_path)?;

        if manifest.pkgs.is_empty() {
            warn!("No packages are currently installed");
            return Ok(());
        }

        let mut successes = Vec::new();
        let mut failures = Vec::new();

        for name in &names {
            match do_uninstall(name, self.all, &mut manifest, &config.bin_dir, &layout.pkgs_dir) {
                Ok(Some(receipt)) => successes.push(receipt),
                Ok(None) => {
                    // package is not installed
                    warn!("Package '{name}' is not installed");
                    failures.push((name.clone(), "Package not installed".to_string()));
                }
                Err(e) => {
                    fail!("Failed to uninstall '{name}': {e:?}");
                    failures.push((name.clone(), e.to_string()));
                }
            }
        }

        if !successes.is_empty() {
            manifest.save(manifest_path)?;
            detail!("Manifest updated");
        }

        // summary
        eprintln!();
        let has_success = !successes.is_empty();
        let has_failure = !failures.is_empty();

        if has_success {
            done!("Successfully uninstalled {} package(s):", successes.len());

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

            fail!("Failed to uninstall {} package(s):", failures.len());

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
            warn!("Nothing to do");
            return Ok(());
        }

        if has_failure {
            std::process::exit(1);
        }

        Ok(())
    }
}

fn do_uninstall(
    raw_name: &str,
    for_all: bool,
    manifest: &mut Manifest,
    bin_dir: &Path,
    pkgs_dir: &Path,
) -> Result<Option<UninstallReceipt>> {
    let (name, requested_ver) = parse_package_version(raw_name);

    // check if installed
    let Some(state) = manifest.pkgs.get(name) else {
        return Ok(None);
    };

    // if uninstall --all
    if for_all {
        step!("Uninstalling ALL versions of '{name}'...");

        if let Some(receipts) = manifest.remove_package(name) {
            for receipt in receipts {
                cleanup_files(&receipt)?;
                detail!("Removed version {}", receipt.version);
            }
            return Ok(Some(UninstallReceipt {
                name: name.to_string(),
                version: "ALL".to_string(),
                fully_removed: true,
            }));
        } else {
            unreachable!()
        }
    }

    // if not --all
    let current_ver = state.current_version.clone();
    let target_ver = if let Some(ver) = requested_ver {
        // specify a version
        if !state.versions.contains_key(ver) {
            anyhow::bail!("Version '{ver}' is not installed for package '{name}'");
        }
        ver.to_string()
    } else {
        // not specify
        match &current_ver {
            Some(v) => v.clone(),
            None => {
                let one_ver = state.versions.keys().next().expect("None versions");
                // no active version, but only one version, remove it
                if state.versions.len() == 1 {
                    one_ver.clone()
                } else {
                    anyhow::bail!(
                        "Package '{name}' has no active version and multiple versions installed. Please specify a version to uninstall (e.g. '{name}@{one_ver}') or use --all"
                    );
                }
            }
        }
    };

    step!("Uninstalling package '{name}' ({target_ver})...");

    // remove from manifest
    if let Some(receipt) = manifest.remove_version(name, &target_ver) {
        // remove files
        cleanup_files(&receipt)?;

        let fully_removed = !manifest.pkgs.contains_key(name);

        // auto-switch if:
        // 1. target_ver == current_ver
        // 2. package has at least one version
        // 3. current_version is none
        if !fully_removed && Some(&target_ver) == current_ver.as_ref() {
            // reacquire state
            if let Some(state) = manifest.pkgs.get_mut(name)
                && state.current_version.is_none()
                && let Some(next_ver) = state.get_latest_version()
            {
                hint!("Auto-switching to fallback version '{next_ver}'...");

                // get receipt and relink
                if let Some(new_receipt) = state.versions.get_mut(&next_ver) {
                    if let Err(e) = new_receipt.relink(bin_dir, pkgs_dir) {
                        warn!("Failed to auto-switch symlinks: {e:?}");
                    } else {
                        state.current_version = Some(next_ver.clone());
                        detail!("Switched successfully");
                    }
                }
            }
        }
        Ok(Some(UninstallReceipt { name: name.to_string(), version: target_ver, fully_removed }))
    } else {
        anyhow::bail!("Failed to remove version from manifest");
    }
}

fn cleanup_files(receipt: &PkgReceipt) -> Result<()> {
    // remove symbolic link
    for bin in &receipt.binaries {
        let link = &bin.link_path;

        if link.is_symlink() {
            match fs::read_link(link) {
                Ok(target) => {
                    // only remove if the symlink points to the expected target
                    if target == bin.bin_path {
                        fs::remove_file(link).with_context(|| {
                            format!("Failed to remove symlink: {}", link.display())
                        })?;
                        detail!("Removed link: {}", link.display());
                    } else {
                        // symlink points to a different target, skip
                    }
                }
                Err(e) => {
                    warn!("Failed to read symlink {}: {}, skipping removal", link.display(), e);
                }
            }
        } else if link.exists() {
            // not a symlink but exists, skip
        }
    }

    // remove data install dir
    if receipt.install_dir.exists() {
        fs::remove_dir_all(&receipt.install_dir).with_context(|| {
            format!("Failed to remove data dir: {}", receipt.install_dir.display())
        })?;
        detail!("Removed data: {}", receipt.install_dir.display());
    }

    // if package dir is empty, remove it
    if let Some(parent) = receipt.install_dir.parent() {
        let _ = fs::remove_dir(parent);
    }

    Ok(())
}
