use std::collections::HashMap;
use std::env;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{InstallCandidate, RemoteType};
use crate::package::PkgDef;
use crate::platform::PlatformInfo;
use crate::remotes::VersionInfo;
use crate::utils::{is_ignored_format, is_supported_format};
use crate::{client, report};

const GITHUB_RELEASES_PER_PAGE: u32 = 20;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to build HTTP client")]
    HttpClientBuild(#[from] reqwest::Error),

    #[error("API request to GitHub for repo '{repo}' failed")]
    RequestFailed {
        repo: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Failed to parse JSON response from GitHub API for repo '{repo}'")]
    JsonParse {
        repo: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("No available release found for '{0}' (non-draft, non-prerelease, with assets)")]
    NoAvailableRelease(String),

    #[error("Release with tag '{tag}' not found in repo '{repo}'")]
    NoReleaseFound { repo: String, tag: String },

    #[error(
        "In release tag '{tag}' for repo '{repo}', no asset was found matching the keyword '{keyword}'"
    )]
    NoMatchingAsset { repo: String, tag: String, keyword: String },
}

#[derive(Deserialize, Debug, Clone)]
struct Release {
    #[serde(default)]
    repo: String,
    tag_name: String,
    html_url: String,
    prerelease: bool,
    draft: bool,
    created_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
    assets: Vec<Asset>,
}

#[derive(Deserialize, Debug, Clone)]
struct Asset {
    name: String,
    // pub content_type: String,
    browser_download_url: String,
}

impl Release {
    fn is_suitable(&self) -> bool { !self.draft && !self.prerelease && !self.assets.is_empty() }

    fn is_available(&self) -> bool { !self.draft && !self.assets.is_empty() }

    fn find_assets(&self, asset_map: &HashMap<String, String>) -> Result<Vec<&Asset>> {
        let platform = PlatformInfo::current();
        let platform_key = platform.key();

        // if the platform-specific asset is configured, use its name
        if let Some(keyword) = asset_map.get(&platform_key) {
            report!(
                MsgType::Detail,
                "Using explicit configuration for platform '{platform_key}': '{keyword}'"
            );
            let matching_assets: Vec<&Asset> = self
                .assets
                .iter()
                .filter(|asset| {
                    asset.name.contains(keyword) && !is_ignored_format(&asset.name.to_lowercase())
                })
                .collect();
            if matching_assets.is_empty() {
                return Err(Error::NoMatchingAsset {
                    repo: self.repo.clone(),
                    tag: self.tag_name.clone(),
                    keyword: keyword.clone(),
                });
            }
            return Ok(matching_assets);
        }

        // if not configured, use the os and arch to match the asset name
        let os_aliases = platform.os_aliases();
        let arch_aliases = platform.arch_aliases();

        let mut candidates: Vec<(&Asset, i32)> = self
            .assets
            .iter()
            .filter(|asset| {
                let name_lower = asset.name.to_lowercase();
                let os_match = os_aliases.iter().any(|&alias| name_lower.contains(alias));
                let arch_match = arch_aliases.iter().any(|&alias| name_lower.contains(alias));
                os_match && arch_match && is_supported_format(&name_lower)
            })
            .map(|asset| {
                let score = calculate_heuristic_score(asset, &platform);
                (asset, score)
            })
            .collect();

        if candidates.is_empty() {
            return Err(Error::NoMatchingAsset {
                repo: self.repo.clone(),
                tag: self.tag_name.clone(),
                keyword: platform_key,
            });
        }

        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        let sorted_assets = candidates.into_iter().map(|(asset, _)| asset).collect();
        Ok(sorted_assets)
    }
}

fn calculate_heuristic_score(asset: &Asset, platform: &PlatformInfo) -> i32 {
    let name = asset.name.to_lowercase();
    let mut score = 0;

    if platform.os == "windows" {
        if name.contains("msvc") {
            score += 10;
        } else if name.contains("gnu") {
            score += 0;
        }

        if name.ends_with(".7z") {
            score += 5;
        } else if name.ends_with(".zip") {
            score += 3;
        } else if name.ends_with(".exe") {
            score += 2;
        }
    }

    if platform.os == "linux" {
        if name.contains("musl") {
            score += 5;
        } else if name.contains("gnu") {
            score += 0;
        }

        if [".tar.gz", ".tgz", ".tar.xz", ".txz"].iter().any(|ext| name.ends_with(ext)) {
            score += 2;
        }
    }

    // generic
    if name.contains("setup") || name.contains("install") {
        score -= 100;
    }

    score
}

#[derive(Deserialize, Debug, Clone)]
#[serde(transparent)]
pub(crate) struct Releases(Vec<Release>);

impl Releases {
    fn list_suitable(&self) -> Vec<&Release> { self.0.iter().filter(|r| r.is_suitable()).collect() }

    fn latest_suitable(&self) -> Result<&Release> {
        let suitable = self.list_suitable();
        suitable.first().copied().ok_or_else(|| {
            let repo = self.0.first().map_or_else(|| "Unknown".to_string(), |r| r.repo.clone());
            Error::NoAvailableRelease(repo)
        })
    }

    fn get_by_tag(&self, tag: &str) -> Result<&Release> {
        self.0.iter().find(|r| r.tag_name == tag).ok_or_else(|| {
            let repo = self.0.first().map_or_else(|| "Unknown".to_string(), |r| r.repo.clone());
            Error::NoReleaseFound { repo, tag: tag.to_string() }
        })
    }
}

impl From<Vec<Release>> for Releases {
    fn from(releases: Vec<Release>) -> Self { Self(releases) }
}

impl FromIterator<Release> for Releases {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = Release>,
    {
        let items: Vec<Release> = iter.into_iter().collect();
        Releases(items)
    }
}

pub struct GitHubProvider;

impl GitHubProvider {
    pub fn new() -> Result<Self> { Ok(Self) }

    // ==================== Async versions (for install/update) ====================

    pub(crate) async fn fetch_releases_async(&self, repo: &str) -> Result<Releases> {
        let api_url = format!("https://api.github.com/repos/{repo}/releases");
        let client = client::get();

        let mut request_builder = client
            .get(&api_url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(&[("per_page", GITHUB_RELEASES_PER_PAGE)]);

        if let Ok(token) = env::var("INRO_GITHUB_TOKEN") {
            report!(MsgType::Detail, "Using INRO_GITHUB_TOKEN for authentication");
            request_builder = request_builder.bearer_auth(token);
        } else if let Ok(token) = env::var("GITHUB_TOKEN") {
            report!(MsgType::Detail, "Using GITHUB_TOKEN for authentication");
            request_builder = request_builder.bearer_auth(token);
        } else {
            report!(
                MsgType::Warning,
                "Unauthenticated requests are rate-limited. Consider setting INRO_GITHUB_TOKEN or GITHUB_TOKEN environment variable"
            );
        }

        report!(MsgType::Detail, "Fetching releases from GitHub repository '{repo}'...");

        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::RequestFailed { repo: repo.to_string(), source: e })?;
        let response = response
            .error_for_status()
            .map_err(|e| Error::RequestFailed { repo: repo.to_string(), source: e })?;

        let mut release_vec: Vec<Release> = response
            .json()
            .await
            .map_err(|e| Error::JsonParse { repo: repo.to_string(), source: e })?;
        for release in &mut release_vec {
            release.repo = repo.to_string();
        }

        Ok(Releases(release_vec))
    }

    pub async fn find_candidates_async(
        &self,
        pkg: &PkgDef,
        ver: Option<&str>,
    ) -> super::Result<Vec<InstallCandidate>> {
        let repo = match &pkg.remote {
            RemoteType::GitHub(asset_def) => &asset_def.repo,
        };

        let releases = self.fetch_releases_async(repo).await?;
        let release =
            if let Some(v) = ver { releases.get_by_tag(v)? } else { releases.latest_suitable()? };

        let RemoteType::GitHub(asset_def) = &pkg.remote;
        let assets = release.find_assets(&asset_def.asset)?;

        let candidates: Vec<InstallCandidate> = assets
            .into_iter()
            .map(|asset| InstallCandidate {
                version: release.tag_name.clone(),
                asset_name: asset.name.clone(),
                download_url: asset.browser_download_url.clone(),
            })
            .collect();
        Ok(candidates)
    }

    // ==================== Sync versions (for info/source) ====================

    fn fetch_releases_sync(&self, repo: &str) -> Result<Releases> {
        let api_url = format!("https://api.github.com/repos/{repo}/releases");

        let client = reqwest::blocking::Client::builder()
            .user_agent(format!("inro/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(Error::HttpClientBuild)?;

        let mut request_builder = client
            .get(&api_url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(&[("per_page", GITHUB_RELEASES_PER_PAGE)]);

        if let Ok(token) = env::var("INRO_GITHUB_TOKEN") {
            report!(MsgType::Detail, "Using INRO_GITHUB_TOKEN for authentication");
            request_builder = request_builder.bearer_auth(token);
        } else if let Ok(token) = env::var("GITHUB_TOKEN") {
            report!(MsgType::Detail, "Using GITHUB_TOKEN for authentication");
            request_builder = request_builder.bearer_auth(token);
        } else {
            report!(
                MsgType::Warning,
                "Unauthenticated requests are rate-limited. Consider setting INRO_GITHUB_TOKEN or GITHUB_TOKEN environment variable"
            );
        }

        report!(MsgType::Detail, "Fetching releases from GitHub repository '{repo}'...");

        let response = request_builder
            .send()
            .map_err(|e| Error::RequestFailed { repo: repo.to_string(), source: e })?;
        let response = response
            .error_for_status()
            .map_err(|e| Error::RequestFailed { repo: repo.to_string(), source: e })?;

        let mut release_vec: Vec<Release> =
            response.json().map_err(|e| Error::JsonParse { repo: repo.to_string(), source: e })?;
        for release in &mut release_vec {
            release.repo = repo.to_string();
        }

        Ok(Releases(release_vec))
    }

    pub fn list_versions(&self, pkg: &PkgDef) -> super::Result<Vec<VersionInfo>> {
        let repo = match &pkg.remote {
            RemoteType::GitHub(asset_def) => &asset_def.repo,
        };

        let releases = self.fetch_releases_sync(repo)?;
        let versions = releases
            .0
            .iter()
            .filter(|r| r.is_available())
            .map(|r| {
                let date = r.published_at.unwrap_or(r.created_at);
                VersionInfo {
                    tag: r.tag_name.clone(),
                    url: r.html_url.clone(),
                    published_at: date,
                    prerelease: r.prerelease,
                }
            })
            .collect();

        Ok(versions)
    }
}
