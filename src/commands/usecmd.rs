use anyhow::{Result, anyhow};
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::report;

pub struct UseCommand {
    pub name: String,
    pub version: String,
    pub layout: InroLayout,
}

impl CommandHandler for UseCommand {
    fn handle(&self) -> Result<()> {
        let mut manifest = Manifest::load(&self.layout.manifest_path)?;
        let config = Config::load(&self.layout)?;

        let state = manifest
            .pkgs
            .get_mut(&self.name)
            .ok_or_else(|| anyhow!("Package '{}' is not installed", self.name))?;

        if let Some(receipt) = state.versions.get_mut(&self.version) {
            report!(MsgType::Step, "Switching '{}' to version '{}'...", self.name, self.version);

            if let Err(e) = receipt.relink(&config.bin_dir) {
                report!(MsgType::Error, "Failed to update symlinks: {e}");
                return Err(e);
            }

            state.current_version = Some(self.version.clone());

            manifest.save(&self.layout.manifest_path)?;

            report!(MsgType::Success, "Now using {}@{}", self.name.green(), self.version.green());
        } else {
            report!(
                MsgType::Error,
                "Version '{}' is not installed for package '{}'",
                self.version,
                self.name
            );

            let mut available: Vec<_> = state.versions.keys().collect();
            available.sort();
            available.reverse();

            report!(
                MsgType::Info,
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
