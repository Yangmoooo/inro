use anyhow::Result;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::report;

pub struct UnlinkCommand {
    pub name: String,
    pub layout: InroLayout,
}

impl CommandHandler for UnlinkCommand {
    fn handle(&self) -> Result<()> {
        let mut manifest = Manifest::load(&self.layout.manifest_path)?;

        if let Some(receipt) = manifest.unlink_dan(&self.name) {
            report!(MsgType::Step, "Unlinking '{}' ({}) ...", self.name, receipt.version);

            if let Err(e) = receipt.unlink() {
                report!(MsgType::Warning, "Failed to remove symlinks: {e}");
            }

            manifest.save(&self.layout.manifest_path)?;

            report!(MsgType::Success, "Unlinked '{}'. Package remains installed", self.name);
        } else if let Some(state) = manifest.dans.get(&self.name) {
            if state.current_version.is_none() {
                report!(
                    MsgType::Warning,
                    "Package '{}' is already unlinked (no active version)",
                    self.name
                );
            }
        } else {
            report!(MsgType::Error, "Package '{}' is not installed", self.name);
            std::process::exit(1);
        }

        Ok(())
    }
}
