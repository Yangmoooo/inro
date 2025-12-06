use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::remotes::RemoteType;
use crate::utils::create_symlink;

#[derive(Clone, Debug, Deserialize)]
pub struct DanDef {
    #[serde(default)]
    pub ver: Option<String>,
    pub remote: RemoteType,
    #[serde(default)]
    pub bin: Vec<BinDef>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BinDef {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug)]
pub struct ResolvedDan {
    #[allow(dead_code)]
    pub ver: Option<String>,
    pub remote: RemoteType,
    pub bin: Vec<ResolvedBin>,
}

#[derive(Debug)]
pub struct ResolvedBin {
    pub name: String,
    pub link: String,
}

impl DanDef {
    /// Resolves the configuration into a definitive set of installation parameters.
    pub fn resolve(self, dan_name: &str) -> ResolvedDan {
        let normalize_name = |name: String| -> String {
            if cfg!(windows) && !name.to_lowercase().ends_with(".exe") {
                format!("{}.exe", name)
            } else {
                name
            }
        };

        let bin = if self.bin.is_empty() {
            // binary default name is the dan name
            let name = normalize_name(dan_name.to_string());
            vec![ResolvedBin {
                name: name.clone(),
                link: name,
            }]
        } else {
            // process each configured binary
            self.bin
                .into_iter()
                .map(|b| {
                    let raw_name = b.name.unwrap_or_else(|| dan_name.to_string());
                    let name = normalize_name(raw_name);
                    let raw_link = b.link.unwrap_or_else(|| name.clone());
                    let link = normalize_name(raw_link);
                    ResolvedBin { name, link }
                })
                .collect()
        };

        ResolvedDan {
            ver: self.ver,
            remote: self.remote,
            bin,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DanState {
    // none means installed but not linked
    pub current_version: Option<String>,

    // key: version
    pub versions: HashMap<String, DanReceipt>,
}

impl DanState {
    pub fn default() -> Self {
        Self {
            current_version: None,
            versions: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DanReceipt {
    /// Package name, e.g. 'ripgrep'
    pub name: String,

    /// Package version, actually the tag name
    pub version: String,

    /// Remote info
    pub remote: RemoteType,

    /// Installation time
    pub installed_at: DateTime<Utc>,

    /// Installation directory, actually where the binary is extracted to
    pub install_dir: PathBuf,

    /// Binaries installed details
    pub binaries: Vec<InstalledBinary>,
}

impl DanReceipt {
    pub fn save_to_install_dir(&self) -> Result<()> {
        let receipt_path = self.install_dir.join("inro-receipt.json");
        let receipt_file = File::create(&receipt_path)
            .with_context(|| format!("Failed to create receipt backup: {:?}", receipt_path))?;
        serde_json::to_writer_pretty(receipt_file, self)?;
        Ok(())
    }

    pub fn relink(&mut self, target_dir: &Path) -> Result<()> {
        if !target_dir.exists() {
            let _ = fs::create_dir_all(target_dir)
                .with_context(|| format!("Failed to create bin dir: {target_dir:?}"));
        }

        for bin in &mut self.binaries {
            // clean up
            // if the old entry is still at there and its parent dir is not the target_dir
            // thats say the config bin_dir is changed, remove the old link
            if let Some(parent) = bin.link_path.parent()
                && parent != target_dir
                && bin.link_path.exists()
            {
                let _ = fs::remove_file(&bin.link_path);
            }

            // create new and update
            let target = target_dir.join(&bin.name);
            create_symlink(&bin.bin_path, &target)?;
            bin.link_path = target;
        }
        Ok(())
    }

    pub fn unlink(&self) -> Result<()> {
        for bin in &self.binaries {
            if bin.link_path.exists() || bin.link_path.is_symlink() {
                let _ = fs::remove_file(&bin.link_path)
                    .with_context(|| format!("Failed to remove link: {:?}", bin.link_path));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstalledBinary {
    /// Binary file name, e.g. 'rg' or 'rg.exe'
    pub name: String,

    /// Binary file path, e.g. '.../packages/ripgrep/13.0.0/rg'
    pub bin_path: PathBuf,

    /// Binary symlink path, e.g. '~/.local/bin/rg'
    pub link_path: PathBuf,
}

#[derive(thiserror::Error, Debug)]
pub enum DanError {
    #[error("Package '{0}' not found in registry")]
    NotFound(String),

    #[error("Failed to fetch from the upstream: '{0}'")]
    Remote(#[from] crate::remotes::Error),

    #[error("Download failed: '{0}'")]
    Download(#[from] anyhow::Error),

    #[error("Checksum validation failed for downloaded file")]
    #[allow(dead_code)]
    ChecksumMismatch,

    #[error("Failed to extract archive '{filename}'")]
    Extraction {
        filename: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Could not find the binary '{0}' inside the extracted archive")]
    BinaryNotFoundInArchive(String),

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}
