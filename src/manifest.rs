use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::package::{PkgReceipt, PkgState};

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    #[serde(default, rename = "packages")]
    pub pkgs: HashMap<String, PkgState>,
}

fn default_schema_version() -> u32 { 1 }

impl Default for Manifest {
    fn default() -> Self { Self { schema_version: default_schema_version(), pkgs: HashMap::new() } }
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let file = File::open(path)
            .with_context(|| format!("Failed to open manifest file: {}", path.display()))?;
        let reader = BufReader::new(file);
        let manifest = serde_json::from_reader(reader)
            .with_context(|| format!("Failed to parse manifest JSON: {}", path.display()))?;
        Ok(manifest)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = path.with_extension("tmp");
        let file = File::create(&temp_path)?;
        serde_json::to_writer_pretty(file, self)?;
        fs::rename(&temp_path, path)
            .with_context(|| format!("Failed to save manifest to {}", path.display()))?;
        Ok(())
    }

    pub fn add(&mut self, receipt: PkgReceipt) {
        let pkg_name = receipt.name.clone();
        let version = receipt.version.clone();

        let state = self.pkgs.entry(pkg_name).or_default();
        state.versions.insert(version.clone(), receipt);
        state.current_version = Some(version);
    }

    /// Remove a version
    pub fn remove_version(&mut self, name: &str, version: &str) -> Option<PkgReceipt> {
        let state = self.pkgs.get_mut(name)?;
        if state.current_version.as_deref() == Some(version) {
            state.current_version = None;
        }
        let receipt = state.versions.remove(version);
        // after removing, if there are other versions, need to use manually
        if state.versions.is_empty() {
            self.pkgs.remove(name);
        }
        receipt
    }

    pub fn unlink_package(&mut self, name: &str) -> Option<PkgReceipt> {
        let state = self.pkgs.get_mut(name)?;
        let current_ver = state.current_version.take()?;
        state.versions.get(&current_ver).cloned()
    }

    /// Remove all versions of the package
    pub fn remove_package(&mut self, name: &str) -> Option<Vec<PkgReceipt>> {
        let state = self.pkgs.remove(name)?;
        let receipts = state.versions.into_values().collect();
        Some(receipts)
    }
}
