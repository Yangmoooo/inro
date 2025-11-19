pub mod github;

use std::collections::HashMap;

use serde::Deserialize;

use crate::package::PackageInfo;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("An error occurred while fetching from GitHub")]
    GitHub(#[from] github::Error),

    #[error("The source type '{0}' is not supported")]
    UnsupportedSourceType(String),

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

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

#[derive(Debug, Clone)]
pub struct InstallCandidate {
    pub version: String,
    pub asset_name: String,
    pub download_url: String,
}

pub trait RemoteProvider {
    fn find_candidates(&self, pkg: &PackageInfo) -> Result<Vec<InstallCandidate>>;
}
