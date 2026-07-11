use anyhow::Result;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::{done, step, warn};

pub struct UnlinkCommand {
    pub name: String,
}

impl CommandHandler for UnlinkCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let config = Config::load(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        let Some(state) = manifest.pkgs.get(&self.name) else {
            anyhow::bail!("Package '{}' is not installed", self.name);
        };
        let Some(current_version) = state.current_version.as_ref() else {
            warn!("Package '{}' is already unlinked (no active version)", self.name);
            return Ok(());
        };
        let receipt = state.versions.get(current_version).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "Active version '{}' is missing from package '{}'",
                current_version,
                self.name
            )
        })?;

        step!("Unlinking '{}' ({}) ...", self.name, receipt.version);

        if let Err(unlink_error) = receipt.unlink(&config.bin_dir, &layout.pkgs_dir) {
            if let Err(rollback_error) = receipt.relink(&config.bin_dir, &layout.pkgs_dir) {
                return Err(anyhow::anyhow!(
                    "{unlink_error}; additionally failed to restore links: {rollback_error}"
                ));
            }
            return Err(unlink_error);
        }

        manifest.unlink_package(&self.name);
        if let Err(save_error) = manifest.save(&layout.manifest_path) {
            if let Err(rollback_error) = receipt.relink(&config.bin_dir, &layout.pkgs_dir) {
                return Err(anyhow::anyhow!(
                    "{save_error}; additionally failed to restore links: {rollback_error}"
                ));
            }
            return Err(save_error);
        }

        done!("Unlinked '{}'. Package remains installed", self.name);

        Ok(())
    }
}
