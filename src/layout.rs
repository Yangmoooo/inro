use std::path::PathBuf;

use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub struct InroLayout {
    pub home_dir: PathBuf,              // user home
    pub config_path: PathBuf,           // config.toml
    pub manifest_path: PathBuf,         // inro.json
    pub dans_dir: PathBuf,              // packages actually installed
    pub upstream_registry_dir: PathBuf, // upstream registry, inro managed
    pub local_registry_dir: PathBuf,    // local registry, user defined
}

impl InroLayout {
    pub fn new() -> Result<Self> {
        let home_dir =
            dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
        let config_dir =
            dirs::config_dir().ok_or_else(|| anyhow!("Could not determine config directory"))?;
        let data_local_dir = dirs::data_local_dir()
            .ok_or_else(|| anyhow!("Could not determine data local directory"))?;

        let config_path = config_dir.join("inro").join("config.toml");
        let manifest_path = data_local_dir.join("inro").join("inro-manifest.json");

        let dans_dir = data_local_dir.join("inro").join("packages");
        let upstream_registry_dir = data_local_dir.join("inro").join("sources.list.d");
        let local_registry_dir = config_dir.join("inro").join("sources.list.d");

        Ok(Self {
            home_dir,
            config_path,
            manifest_path,
            dans_dir,
            upstream_registry_dir,
            local_registry_dir,
        })
    }
}
