use std::collections::HashSet;
use std::fs;

use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::dan::DanState;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::report;
use crate::utils::sanitize_version;

pub struct CleanCommand {
    pub dry_run: bool,
    pub layout: InroLayout,
}

impl CommandHandler for CleanCommand {
    fn handle(&self) -> Result<()> {
        let mut manifest = Manifest::load(&self.layout.manifest_path)?;
        let pkgs_root = &self.layout.dans_dir;

        if !pkgs_root.exists() {
            report!(MsgType::Warning, "No packages directory found.");
            return Ok(());
        }

        // build a keep list of (name, version)
        // 1. current active versions
        // 2. if no active, the latest (installed) version
        let mut keep_set = HashSet::new();
        for (name, state) in &manifest.dans {
            // note the version in manifest not equals in filesystem (sanitized)
            if let Some(ver) = &state.current_version {
                keep_set.insert((name.clone(), sanitize_version(ver)));
            } else if let Some(latest_ver) = get_latest_version(state) {
                keep_set.insert((name.clone(), sanitize_version(&latest_ver)));
            }
        }

        let mut candidates_to_remove = Vec::new(); // (name, version, path, size)
        let mut recovered_space: u64 = 0; // total bytes recovered

        // traverse first-level pkgs_dir for pkg_names
        for entry in fs::read_dir(pkgs_root)? {
            let entry = entry?;
            let pkg_name = entry.file_name().to_string_lossy().to_string();
            let pkg_path = entry.path();

            if !pkg_path.is_dir() {
                continue;
            }

            // traverse second-level for versions
            for ver_entry in fs::read_dir(&pkg_path)? {
                let ver_entry = ver_entry?;
                let ver_name = ver_entry.file_name().to_string_lossy().to_string();
                let ver_path = ver_entry.path();

                if !ver_path.is_dir() {
                    continue;
                }

                if !keep_set.contains(&(pkg_name.clone(), ver_name.clone())) {
                    let size = fs_extra::dir::get_size(&ver_path).unwrap_or(0);
                    candidates_to_remove.push((pkg_name.clone(), ver_name.clone(), ver_path, size));
                }
            }
        }

        if candidates_to_remove.is_empty() {
            report!(MsgType::Success, "Everything is clean. No old versions found.");
            return Ok(());
        }

        report!(MsgType::Info, "Found {} old version(s) to remove:", candidates_to_remove.len());
        let mut any_removed = false;

        for (pkg, ver, path, size) in &candidates_to_remove {
            let size_str = humansize::format_size(*size, humansize::DECIMAL);

            if self.dry_run {
                println!("  - {} {} ({})", pkg, ver.dimmed(), size_str);
            } else {
                report!(MsgType::Detail, "Removing {pkg}/{ver}...");
                if let Err(e) = fs::remove_dir_all(path) {
                    report!(MsgType::Warning, "Failed to remove {}: {}", path.display(), e);
                } else {
                    if let Some(state) = manifest.dans.get_mut(pkg) {
                        state.versions.remove(ver);
                    }
                    recovered_space += size;
                    any_removed = true;
                }
            }
        }

        if self.dry_run {
            report!(MsgType::Info, "Dry run complete. Use without --dry-run to perform cleanup.");
        } else if any_removed {
            manifest.save(&self.layout.manifest_path)?;

            let total_size_str = humansize::format_size(recovered_space, humansize::DECIMAL);
            report!(
                MsgType::Success,
                "Cleaned up {} old versions. Freed {}.",
                candidates_to_remove.len(),
                total_size_str
            );
        }

        Ok(())
    }
}

/// Get the latest **installed** version from DanState
fn get_latest_version(state: &DanState) -> Option<String> {
    state
        .versions
        .iter()
        .max_by_key(|(_ver, receipt)| receipt.installed_at)
        .map(|(ver, _receipt)| ver.clone())
}
