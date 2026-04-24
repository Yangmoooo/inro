pub mod github;

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed fetching from GitHub")]
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
    pub size: u64,
}

/// Result of candidate discovery, carrying both candidates and how they were
/// matched.
#[derive(Debug)]
pub struct CandidateResult {
    pub candidates: Vec<InstallCandidate>,
    pub match_kind: MatchKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Explicit,
    PlatformHeuristic,
    Fallback,
}

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub tag: String,
    pub url: String,
    pub published_at: DateTime<Utc>,
    pub prerelease: bool,
}

pub fn create_provider(remote: &RemoteType) -> Result<github::GitHubProvider> {
    match remote {
        RemoteType::GitHub(_) => {
            let gh_provider = github::GitHubProvider::new().map_err(Error::GitHub)?;
            Ok(gh_provider)
        } // RemoteType::Direct(_) => { ... }
    }
}
