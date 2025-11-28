use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::remotes::RemoteType;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DanReceipt {
    /// Package name, e.g. 'ripgrep'
    pub name: String,

    /// Package version, actually the tag name
    pub version: String,

    /// Remote info
    pub remote_type: RemoteType,

    /// Installation time
    pub installed_at: DateTime<Utc>,

    /// Installation directory, actually where the binary is extracted to
    pub install_dir: PathBuf,

    /// Binaries installed details
    pub binaries: Vec<InstalledBinary>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstalledBinary {
    /// Binary file name, e.g. 'rg' or 'rg.exe'
    pub name: String,

    /// Binary file path, e.g. '.../packages/ripgrep/13.0.0/rg'
    pub source_path: PathBuf,

    /// Binary symlink path, e.g. '~/.local/bin/rg'
    pub link_path: PathBuf,
}

#[derive(thiserror::Error, Debug)]
pub enum DanError {
    #[error("Package '{0}' not found in sources")]
    NotFound(String),

    #[error("Failed to fetch from the upstream: '{0}'")]
    Remote(#[from] crate::remotes::Error),

    #[error("Download failed: '{0}'")]
    Download(#[from] anyhow::Error),

    #[error("Checksum validation failed for downloaded file")]
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
