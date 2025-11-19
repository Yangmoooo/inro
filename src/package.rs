pub mod remotes;

use std::collections::HashMap;

use serde::Deserialize;

use remotes::Remote;

#[derive(Debug, Default, Deserialize)]
pub struct PackageConfig {
    #[serde(flatten)]
    pub pkgs: HashMap<String, PackageInfo>,
}

#[derive(Debug, Deserialize)]
pub struct PackageInfo {
    #[serde(default)]
    pub ver: Option<String>,
    pub remote: Remote,
    #[serde(default)]
    pub bin: Vec<BinConfig>,
}

#[derive(Debug, Deserialize)]
pub struct BinConfig {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug)]
pub struct PackgeReceipt {
    pub name: String,
    pub ver: String,
    pub bins: Vec<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum PackageError {
    #[error("Package '{0}' not found in any sources")]
    NotFound(String),

    #[error("Failed to fetch from remote: '{0}'")]
    Remote(#[from] remotes::Error),

    #[error("Download failed: '{0}'")]
    Download(#[from] anyhow::Error),

    #[error("Checksum validation failed for downloaded file")]
    ChecksumMismatch,

    #[error("Failed to extract archive '{filename}'")]
    Extraction {
        filename: String,
        #[source]
        source: std::io::Error, // TODO replace from archive library
    },

    #[error("Could not find the binary '{0}' inside the extracted archive")]
    BinaryNotFoundInArchive(String),

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}
