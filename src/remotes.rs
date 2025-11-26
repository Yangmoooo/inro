pub mod github;

use std::collections::HashMap;

use serde::Deserialize;

use crate::dan::ResolvedDan;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("An error occurred while fetching from GitHub")]
    GitHub(#[from] github::Error),

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteType {
    GitHub(GitHubAssetDef),
    // Direct(DirectSource),
}

impl Default for RemoteType {
    fn default() -> Self {
        RemoteType::GitHub(GitHubAssetDef::default())
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GitHubAssetDef {
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
    fn find_candidates(&self, dan: &ResolvedDan) -> Result<Vec<InstallCandidate>>;
}
