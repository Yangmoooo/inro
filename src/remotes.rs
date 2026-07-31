mod asset;
pub mod github;

use std::collections::HashMap;
use std::fmt;

pub use asset::{
    AssetSelector, asset_matches_selector, derive_asset_selector,
    derive_asset_selector_from_assets, is_ignored_format, is_supported_format,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteType {
    GitHub(GitHubAssetDef),
}

impl Default for RemoteType {
    fn default() -> Self { RemoteType::GitHub(GitHubAssetDef::default()) }
}

impl fmt::Display for RemoteType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteType::GitHub(def) => write!(f, "github:{}", def.repo),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GitHubAssetDef {
    pub repo: String,
    #[serde(default)]
    pub asset: HashMap<String, AssetSelector>,
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
    pub asset_names: Vec<String>,
    pub match_kind: MatchKind,
    pub matched_selector: Option<String>,
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
