pub mod remotes;

use std::collections::HashMap;

use serde::Deserialize;

use remotes::RemoteType;

#[derive(Debug, Default, Deserialize)]
pub struct PackageConfig {
    #[serde(flatten)]
    pub pkgs: HashMap<String, PackageInfo>,
}

#[derive(Debug, Deserialize)]
pub struct PackageInfo {
    #[serde(default)]
    pub ver: Option<String>,
    pub remote: RemoteType,
    #[serde(default)]
    pub bin: Vec<BinInfo>,
}

#[derive(Debug, Deserialize)]
pub struct BinInfo {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug)]
pub struct ResolvedPackage {
    pub ver: Option<String>,
    pub remote: RemoteType,
    pub bin: Vec<ResolvedBin>,
}

#[derive(Debug)]
pub struct ResolvedBin {
    pub path: String,
    pub link: String,
}

impl PackageInfo {
    /// Resolves the configuration into a definitive set of installation parameters.
    /// Handles all defaults:
    /// - bin: [] -> [{ path: name, link: name }]
    /// - BinInfo: { path: None } -> { path: name }
    pub fn resolve(&self, pkgname: &str) -> ResolvedPackage {
        let bin = if self.bin.is_empty() {
            // binary default name is the package name
            vec![ResolvedBin {
                path: pkgname.to_string(),
                link: pkgname.to_string(),
            }]
        } else {
            // process each configured binary
            self.bin
                .iter()
                .map(|b| {
                    ResolvedBin {
                        // path default is the package name
                        path: b.path.clone().unwrap_or_else(|| pkgname.to_string()),
                        // link default is the same as the path, or package name if path was None
                        link: b.link.clone().unwrap_or_else(|| {
                            b.path.clone().unwrap_or_else(|| pkgname.to_string())
                        }),
                    }
                })
                .collect()
        };

        ResolvedPackage {
            ver: self.ver.clone(),
            remote: self.remote.clone(),
            bin,
        }
    }
}

#[derive(Debug)]
pub struct PackgeReceipt {
    pub name: String,
    pub ver: String,
    pub bin: Vec<String>,
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
        source: anyhow::Error,
    },

    #[error("Could not find the binary '{0}' inside the extracted archive")]
    BinaryNotFoundInArchive(String),

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}
