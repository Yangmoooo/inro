use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{CommandHandler, InstallCommand};
use crate::package_set;

pub struct ImportCommand {
    pub path: PathBuf,
}

impl CommandHandler for ImportCommand {
    fn handle(&self) -> Result<()> {
        let contents = fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read package set: {}", self.path.display()))?;
        let names = package_set::parse(&contents)
            .with_context(|| format!("Invalid package set: {}", self.path.display()))?;

        InstallCommand { names }.handle()
    }
}
