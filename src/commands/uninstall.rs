use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::PkgReceipt;
use crate::utils::{ensure_unique_package_args, parse_package_version, unique};
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
        ensure_unique_package_args(&self.names)?;
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

        let pkgs_dir = &layout.pkgs_dir;
        let bin_dir = &config.bin_dir;

        for name in &names {
            match do_uninstall(name, self.all, &mut manifest, bin_dir, pkgs_dir) {
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

        manifest.save(manifest_path)?;
        detail!("Manifest updated");

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
            anyhow::bail!("some packages failed to uninstall");
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

    let Some(state) = manifest.pkgs.get(name) else {
        return Ok(None);
    };

    if for_all {
        step!("Uninstalling ALL versions of '{name}'...");

        let receipts: Vec<_> = state.versions.values().cloned().collect();
        if receipts.is_empty() {
            anyhow::bail!("Package '{name}' has no installed versions");
        }
        for receipt in receipts {
            cleanup_files(&receipt, bin_dir, pkgs_dir)?;
            manifest.remove_version(name, &receipt.version);
            detail!("Removed version {}", receipt.version);
        }
        return Ok(Some(UninstallReceipt {
            name: name.to_string(),
            version: "ALL".to_string(),
            fully_removed: true,
        }));
    }

    let current_ver = state.current_version.clone();
    let target_ver = if let Some(ver) = requested_ver {
        if !state.versions.contains_key(ver) {
            anyhow::bail!("Version '{ver}' is not installed for package '{name}'");
        }
        ver.to_string()
    } else {
        match &current_ver {
            Some(v) => v.clone(),
            None => {
                if let Some(one_ver) = state.versions.keys().next()
                    && state.versions.len() == 1
                {
                    one_ver.clone()
                } else {
                    anyhow::bail!(
                        "Package '{name}' has no active version. Specify a version to uninstall \
                         or use --all"
                    );
                }
            }
        }
    };
    let receipt =
        state.versions.get(&target_ver).cloned().ok_or_else(|| {
            anyhow::anyhow!("Version '{target_ver}' is not installed for '{name}'")
        })?;

    step!("Uninstalling package '{name}' ({target_ver})...");

    cleanup_files(&receipt, bin_dir, pkgs_dir)?;
    if manifest.remove_version(name, &target_ver).is_some() {
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
                if let Some(new_receipt) = state.versions.get(&next_ver) {
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

fn cleanup_files(receipt: &PkgReceipt, bin_dir: &Path, pkgs_dir: &Path) -> Result<()> {
    // Remove only links that still point to this receipt's binaries.
    receipt.unlink(bin_dir, pkgs_dir)?;
    for bin in &receipt.binaries {
        let link = receipt.link_path(bin, bin_dir);
        if !link.exists() && !link.is_symlink() {
            detail!("Removed link: {}", link.display());
        }
    }

    let install_dir = receipt.install_dir(pkgs_dir);
    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)
            .with_context(|| format!("Failed to remove data dir: {}", install_dir.display()))?;
        detail!("Removed data: {}", install_dir.display());
    }

    // if the per-package parent dir is empty, remove it
    if let Some(parent) = install_dir.parent() {
        let _ = fs::remove_dir(parent);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use chrono::Utc;

    use super::*;
    use crate::package::InstalledBin;
    use crate::remotes::{GitHubAssetDef, RemoteType};

    #[test]
    fn cleanup_failure_keeps_version_in_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&pkgs_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();

        let receipt = PkgReceipt {
            name: "tool".to_string(),
            version: "v1.0.0".to_string(),
            remote: RemoteType::GitHub(GitHubAssetDef {
                repo: "test/tool".to_string(),
                asset: HashMap::new(),
            }),
            installed_at: Utc::now(),
            install_subdir: PathBuf::from("tool/v1.0.0"),
            binaries: vec![InstalledBin {
                name: "tool".to_string(),
                bin_subpath: PathBuf::from("tool"),
            }],
        };
        let mut manifest = Manifest::default();
        manifest.add(receipt);

        let install_path = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(install_path.parent().unwrap()).unwrap();
        fs::write(&install_path, b"not a directory").unwrap();

        assert!(do_uninstall("tool", false, &mut manifest, &bin_dir, &pkgs_dir).is_err());

        let state = manifest.pkgs.get("tool").expect("package must remain in manifest");
        assert_eq!(state.current_version.as_deref(), Some("v1.0.0"));
        assert!(state.versions.contains_key("v1.0.0"));
    }
}
