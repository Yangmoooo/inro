use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::Result;
use colored::Colorize;
use humansize::{DECIMAL, format_size};
use walkdir::WalkDir;

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
        let _lock = if self.dry_run { None } else { Some(crate::lock::acquire(&layout)?) };
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
        for entry in fs::read_dir(&pkgs_root)? {
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
                    let size = directory_size(&ver_path).unwrap_or(0);
                    candidates_to_remove.push((pkg_name.clone(), ver_name.clone(), ver_path, size));
                }
            }
        }

        if candidates_to_remove.is_empty() {
            done!("Everything is clean. No old versions found.");
            return Ok(());
        }

        hint!("Found {} old version(s) to remove:", candidates_to_remove.len());
        let mut removed_count = 0usize;

        for (pkg, ver, path, size) in &candidates_to_remove {
            let size_str = format_size(*size, DECIMAL);

            if self.dry_run {
                println!("  - {} {} ({})", pkg, ver.dimmed(), size_str);
                continue;
            }

            detail!("Removing {pkg}/{ver}...");
            if let Err(e) = fs::remove_dir_all(path) {
                warn!("Failed to remove {}: {}", path.display(), e);
            } else {
                if let Some(state) = manifest.pkgs.get_mut(pkg)
                    && let Some(raw_ver) =
                        state.versions.keys().find(|v| sanitize_version(v) == *ver).cloned()
                {
                    state.versions.remove(&raw_ver);
                }
                // If the package dir is empty after removal, remove it as well
                let pkg_dir = pkgs_root.join(pkg);
                if pkg_dir.read_dir().map(|mut d| d.next().is_none()).unwrap_or(false) {
                    let _ = fs::remove_dir(&pkg_dir);
                }
                recovered_space += size;
                removed_count += 1;
            }
        }

        if self.dry_run {
            hint!("Dry run complete. Use without --dry-run to perform cleanup.");
        } else if removed_count > 0 {
            manifest.save(&layout.manifest_path)?;

            let total_size_str = format_size(recovered_space, DECIMAL);
            done!("Cleaned up {} old versions. Freed {}.", removed_count, total_size_str);
        }

        Ok(())
    }
}

fn directory_size(path: &Path) -> Result<u64> {
    WalkDir::new(path).follow_links(false).follow_root_links(false).into_iter().try_fold(
        0,
        |size, entry| -> Result<u64> {
            let entry = entry?;
            if entry.depth() == 0 || entry.file_type().is_dir() {
                return Ok(size);
            }
            Ok(size.saturating_add(entry.metadata()?.len()))
        },
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn directory_size_counts_nested_files() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(temp.path().join("root.bin"), [0; 3]).unwrap();
        fs::write(nested.join("nested.bin"), [0; 5]).unwrap();

        assert_eq!(directory_size(temp.path()).unwrap(), 8);
    }

    #[test]
    fn directory_size_rejects_a_missing_root() {
        let temp = tempfile::tempdir().unwrap();

        assert!(directory_size(&temp.path().join("missing")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn directory_size_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("target");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("large.bin"), [0; 64]).unwrap();

        let link = root.join("target-link");
        symlink(&target, &link).unwrap();
        let link_size = fs::symlink_metadata(&link).unwrap().len();

        assert_eq!(directory_size(&root).unwrap(), link_size);
    }
}
