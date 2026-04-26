use anyhow::{Result, anyhow};
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::{done, fail, hint, step};

pub struct UseCommand {
    pub name: String,
    pub version: String,
}

impl CommandHandler for UseCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;
        let config = Config::load(&layout)?;

        let state = manifest
            .pkgs
            .get_mut(&self.name)
            .ok_or_else(|| anyhow!("Package '{}' is not installed", self.name))?;

        if let Some(receipt) = state.versions.get_mut(&self.version) {
            step!("Switching '{}' to version '{}'...", self.name, self.version);

            if let Err(e) = receipt.relink(&config.bin_dir) {
                fail!("Failed to update symlinks: {e}");
                return Err(e);
            }

            state.current_version = Some(self.version.clone());

            manifest.save(&layout.manifest_path)?;

            done!("Now using {}@{}", self.name.green(), self.version.green());
        } else {
            fail!("Version '{}' is not installed for package '{}'", self.version, self.name);

            let mut available: Vec<_> = state.versions.keys().collect();
            available.sort();
            available.reverse();

            hint!(
                "Available versions: {}",
                available
                    .into_iter()
                    .map(std::string::String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            std::process::exit(1);
        }

        Ok(())
    }
}
