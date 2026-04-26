use anyhow::Result;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::{done, fail, step, warn};

pub struct UnlinkCommand {
    pub name: String,
}

impl CommandHandler for UnlinkCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        if let Some(receipt) = manifest.unlink_package(&self.name) {
            step!("Unlinking '{}' ({}) ...", self.name, receipt.version);

            if let Err(e) = receipt.unlink(&layout.pkgs_dir) {
                warn!("Failed to remove symlinks: {e}");
            }

            manifest.save(&layout.manifest_path)?;

            done!("Unlinked '{}'. Package remains installed", self.name);
        } else if let Some(state) = manifest.pkgs.get(&self.name) {
            if state.current_version.is_none() {
                warn!("Package '{}' is already unlinked (no active version)", self.name);
            }
        } else {
            fail!("Package '{}' is not installed", self.name);
            std::process::exit(1);
        }

        Ok(())
    }
}
