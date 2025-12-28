pub mod github;

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::package::PkgDef;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("An error occurred while fetching from GitHub")]
    GitHub(#[from] github::Error),

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteType {
    GitHub(GitHubAssetDef),
    // Direct(DirectDef),
}

impl Default for RemoteType {
    fn default() -> Self { RemoteType::GitHub(GitHubAssetDef::default()) }
}

impl fmt::Display for RemoteType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteType::GitHub(def) => write!(f, "github:{}", def.repo),
            // RemoteType::Direct(def) => write!(f, "direct:{}", def.url),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub tag: String,
    pub url: String,
    pub published_at: DateTime<Utc>,
    pub prerelease: bool,
}

pub trait RemoteProvider {
    fn find_candidates(&self, pkg: &PkgDef) -> Result<Vec<InstallCandidate>>;
    fn list_versions(&self, pkg: &PkgDef) -> Result<Vec<VersionInfo>>;
}

pub fn create_provider(remote: &RemoteType) -> Result<Box<dyn RemoteProvider>> {
    match remote {
        RemoteType::GitHub(_) => {
            let gh_provider = github::GitHubProvider::new().map_err(Error::GitHub)?;
            Ok(Box::new(gh_provider))
        } // RemoteType::Direct(_) => { ... }
    }
}
