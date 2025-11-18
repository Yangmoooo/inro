mod remotes;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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
#[serde(rename_all = "lowercase")]
pub enum Remote {
    GitHub(GitHubSource),
    // Direct(DirectSource),
}

impl Default for Remote {
    fn default() -> Self {
        Remote::GitHub(GitHubSource::default())
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct GitHubSource {
    pub repo: String,
    #[serde(default)]
    pub asset: HashMap<String, String>,
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
    #[error("Package '{name}' not found in any sources")]
    NotFound { name: String },

    #[error("Failed to fetch from '{name}'")]
    Remote {
        name: String,
        #[source]
        source: remotes::Error,
    },

    #[error("Download failed for '{url}'")]
    Download {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Checksum validation failed for downloaded file")]
    ChecksumMismatch,

    #[error("Failed to extract archive '{filename}'")]
    Extraction {
        filename: String,
        #[source]
        source: std::io::Error, // TODO replace from archive library
    },

    #[error("Could not find the binary '{binary_name}' inside the extracted archive")]
    BinaryNotFoundInArchive { binary_name: String },

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}
