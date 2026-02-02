use std::collections::HashSet;
use std::fs;

use anyhow::Result;
use colored::Colorize;
use humansize::{DECIMAL, format_size};

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::utils::sanitize_version;
use crate::{detail, done, hint, warn};

pub struct CleanCommand {
    pub dry_run: bool,
}

impl CommandHandler for CleanCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;
        let pkgs_root = layout.pkgs_dir;

        if !pkgs_root.exists() {
            warn!("No packages directory found.");
            return Ok(());
        }

        // build a keep list of (name, version)
        // 1. current active versions
        // 2. if no active, the latest (installed) version
        let mut keep_set = HashSet::new();
        for (name, state) in &manifest.pkgs {
            // note the version in manifest not equals in filesystem (sanitized)
            if let Some(ver) = &state.current_version {
                keep_set.insert((name.clone(), sanitize_version(ver)));
            } else if let Some(latest_ver) = state.get_latest_version() {
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
            done!("Everything is clean. No old versions found.");
            return Ok(());
        }

        hint!("Found {} old version(s) to remove:", candidates_to_remove.len());
        let mut any_removed = false;

        for (pkg, ver, path, size) in &candidates_to_remove {
            let size_str = format_size(*size, DECIMAL);

            if self.dry_run {
                println!("  - {} {} ({})", pkg, ver.dimmed(), size_str);
            } else {
                detail!("Removing {pkg}/{ver}...");
                if let Err(e) = fs::remove_dir_all(path) {
                    warn!("Failed to remove {}: {}", path.display(), e);
                } else {
                    if let Some(state) = manifest.pkgs.get_mut(pkg) {
                        state.versions.remove(ver);
                    }
                    recovered_space += size;
                    any_removed = true;
                }
            }
        }

        if self.dry_run {
            hint!("Dry run complete. Use without --dry-run to perform cleanup.");
        } else if any_removed {
            manifest.save(&layout.manifest_path)?;

            let total_size_str = format_size(recovered_space, DECIMAL);
            done!(
                "Cleaned up {} old versions. Freed {}.",
                candidates_to_remove.len(),
                total_size_str
            );
        }

        Ok(())
    }
}
