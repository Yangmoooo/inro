use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use tempfile::NamedTempFile;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::{InstalledBin, PkgReceipt, ResolvedPkg, rollback_links};
use crate::registry::Registry;
use crate::utils::{
    create_symlink, find_binary_in_dir, is_inro_managed_symlink, symlink_points_to,
    validate_path_component,
};
use crate::{done, step};

pub struct RelinkCommand {
    pub name: String,
}

impl CommandHandler for RelinkCommand {
    fn handle(&self) -> Result<()> {
        validate_path_component(&self.name, "package name")?;
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let config = Config::load(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;
        let state = manifest
            .pkgs
            .get(&self.name)
            .ok_or_else(|| anyhow!("Package '{}' is not installed", self.name))?;
        let version = state.current_version.as_ref().ok_or_else(|| {
            anyhow!(
                "Package '{}' has no active version; run 'inro use {} <VERSION>' first",
                self.name,
                self.name,
            )
        })?;
        let previous = state.versions.get(version).cloned().ok_or_else(|| {
            anyhow!("Active version '{}' is missing from package '{}'", version, self.name,)
        })?;
        let registry = Registry::load(&layout)?;
        let definition = registry
            .pkgs
            .get(&self.name)
            .ok_or_else(|| anyhow!("Package '{}' not found in registry", self.name))?;
        let updated =
            resolve_receipt(&previous, &definition.resolve(&self.name), &layout.pkgs_dir)?;

        step!("Relinking '{}' ({}) from registry definitions...", self.name, previous.version);
        apply_relink(&previous, updated, &mut manifest, &config, &layout)?;
        done!("Updated links for '{}@{}'", self.name, previous.version);
        Ok(())
    }
}

fn resolve_receipt(
    previous: &PkgReceipt,
    package: &ResolvedPkg,
    pkgs_dir: &Path,
) -> Result<PkgReceipt> {
    if package.bin.is_empty() {
        bail!("Package '{}' has no binary defined for the current platform", previous.name);
    }
    let install_dir = previous.install_dir(pkgs_dir);
    let mut names = HashSet::new();
    let mut binaries = Vec::with_capacity(package.bin.len());
    for bin in &package.bin {
        validate_path_component(&bin.name, "binary name")?;
        validate_path_component(&bin.link, "link name")?;
        let key = if cfg!(windows) { bin.link.to_lowercase() } else { bin.link.clone() };
        if !names.insert(key) {
            bail!("Duplicate link name '{}' in package '{}'", bin.link, previous.name);
        }
        let path = find_binary_in_dir(&install_dir, &bin.name).ok_or_else(|| anyhow!(
            "Binary '{}' not found in installed package '{}@{}'; install a version that includes it",
            bin.name, previous.name, previous.version,
        ))?;
        binaries.push(InstalledBin {
            name: bin.link.clone(),
            bin_subpath: path.strip_prefix(&install_dir)?.to_path_buf(),
        });
    }
    Ok(PkgReceipt { binaries, ..previous.clone() })
}

fn apply_relink(
    previous: &PkgReceipt,
    updated: PkgReceipt,
    manifest: &mut Manifest,
    config: &Config,
    layout: &InroLayout,
) -> Result<()> {
    let mut obsolete = previous.clone();
    for bin in &obsolete.binaries {
        validate_path_component(&bin.name, "link name")?;
    }
    obsolete.binaries.retain(|bin| {
        let retained = updated.binaries.iter().any(|new| {
            if cfg!(windows) {
                new.name.eq_ignore_ascii_case(&bin.name)
            } else {
                new.name == bin.name
            }
        });
        let link = previous.link_path(bin, &config.bin_dir);
        !retained
            && is_inro_managed_symlink(&link, &layout.pkgs_dir)
            && symlink_points_to(&link, &previous.bin_path(bin, &layout.pkgs_dir))
    });
    let mut affected = updated.clone();
    affected.binaries.extend(obsolete.binaries.iter().cloned());
    let previous_links =
        affected.snapshot_binary_links(&config.bin_dir, &layout.pkgs_dir.join(&previous.name))?;

    // Stage both receipt versions before touching links so rollback needs no
    // new file.
    let install_dir = previous.install_dir(&layout.pkgs_dir);
    let receipt_path = install_dir.join("inro-receipt.json");
    let backup = match fs::symlink_metadata(&receipt_path) {
        Ok(metadata) if metadata.is_file() => {
            let file = stage_file(&install_dir, &fs::read(&receipt_path)?)?;
            file.as_file().set_permissions(metadata.permissions())?;
            Some(file)
        }
        Ok(_) => bail!("Install receipt '{}' is not a regular file", receipt_path.display()),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Failed to read install receipt"),
    };
    let staged = stage_file(&install_dir, &serde_json::to_vec_pretty(&updated)?)?;
    if let Some(backup) = &backup {
        staged.as_file().set_permissions(backup.as_file().metadata()?.permissions())?;
    }
    let mut receipt_written = false;
    let result = (|| -> Result<()> {
        updated.link_binaries(&config.bin_dir, &layout.pkgs_dir)?;
        obsolete.unlink_binaries(&config.bin_dir, &layout.pkgs_dir)?;
        staged.persist(&receipt_path).context("Failed to update install receipt")?;
        receipt_written = true;
        manifest.add(updated);
        manifest.save(&layout.manifest_path)?;
        Ok(())
    })();

    if let Err(mut error) = result {
        manifest.add(previous.clone());
        if receipt_written {
            let restore = match backup {
                Some(file) => file.persist(&receipt_path).map(|_| ()).map_err(|e| e.error),
                None => fs::remove_file(&receipt_path),
            };
            if let Err(restore_error) = restore {
                error = anyhow!(
                    "{error:#}; additionally failed to restore install receipt: {restore_error}"
                );
            }
        }
        if let Err(restore_error) =
            rollback_links(&previous_links, &layout.pkgs_dir, &mut create_symlink)
        {
            error = anyhow!("{error:#}; additionally failed to restore links: {restore_error:#}");
        }
        return Err(error);
    }
    Ok(())
}

fn stage_file(directory: &Path, contents: &[u8]) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new_in(directory).context("Failed to stage install receipt")?;
    file.write_all(contents).context("Failed to write staged install receipt")?;
    Ok(file)
}
