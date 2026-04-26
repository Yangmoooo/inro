use anyhow::Result;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::{done, fail};

pub struct PinCommand {
    pub name: String,
}

pub struct UnpinCommand {
    pub name: String,
}

impl CommandHandler for PinCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        match manifest.pkgs.get_mut(&self.name) {
            Some(state) => {
                state.pinned = true;
                manifest.save(&layout.manifest_path)?;
                done!("Pinned '{}'. It will be skipped during updates", self.name);
            }
            None => {
                fail!("Package '{}' is not installed", self.name);
                std::process::exit(1);
            }
        }

        Ok(())
    }
}

impl CommandHandler for UnpinCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        match manifest.pkgs.get_mut(&self.name) {
            Some(state) => {
                state.pinned = false;
                manifest.save(&layout.manifest_path)?;
                done!("Unpinned '{}'. It will be updated normally", self.name);
            }
            None => {
                fail!("Package '{}' is not installed", self.name);
                std::process::exit(1);
            }
        }

        Ok(())
    }
}
