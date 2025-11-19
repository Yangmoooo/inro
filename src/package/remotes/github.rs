use std::collections::HashMap;
use std::env;

use reqwest::blocking::Client;
use serde::Deserialize;

use super::{InstallCandidate, RemoteProvider, Result as RemoteResult};
use crate::package::{PackageInfo, Remote};
use crate::platform::PlatformInfo;
use crate::report;

const GITHUB_RELEASES_PER_PAGE: u32 = 20;
const GITHUB_ASSETS_VALID_TYPES: &[&str] = &[
    "application/octet-stream",
    "application/x-msdownload",
    "application/x-gtar",
    "application/gzip",
    "application/zip",
];

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

    #[error(
        "In release tag '{tag}' for repo '{repo}', no asset was found matching the keyword '{keyword}'"
    )]
    NoMatchingAsset {
        repo: String,
        tag: String,
        keyword: String,
    },
}

#[derive(Deserialize, Debug, Clone)]
pub struct Release {
    #[serde(default)]
    pub repo: String,
    pub tag_name: String,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub content_type: String,
    pub browser_download_url: String,
}

impl Release {
    pub fn find_assets(&self, asset_map: &HashMap<String, String>) -> Result<Vec<&Asset>> {
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
                    asset.name.contains(keyword)
                        && GITHUB_ASSETS_VALID_TYPES.contains(&asset.content_type.as_str())
                })
                .collect();
            if matching_assets.is_empty() {
                return Err(Error::NoMatchingAsset {
                    repo: self.repo.clone(),
                    tag: self.tag_name.clone(),
                    keyword: keyword.to_string(),
                });
            }
            return Ok(matching_assets);
        }

        // if not configured, use the os and arch to match the asset name
        let os_aliases = platform.os_aliases();
        let arch_aliases = platform.arch_aliases();

        let matching_assets: Vec<&Asset> = self
            .assets
            .iter()
            .filter(|asset| {
                if !GITHUB_ASSETS_VALID_TYPES.contains(&asset.content_type.as_str()) {
                    return false;
                }
                let name_lower = asset.name.to_lowercase();
                let os_match = os_aliases.iter().any(|&alias| name_lower.contains(alias));
                let arch_match = arch_aliases.iter().any(|&alias| name_lower.contains(alias));
                os_match && arch_match
            })
            .collect();

        if matching_assets.is_empty() {
            // TODO: hint to use `inro add`
            return Err(Error::NoMatchingAsset {
                repo: self.repo.clone(),
                tag: self.tag_name.clone(),
                keyword: platform_key,
            });
        }

        Ok(matching_assets)
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(transparent)]
pub struct Releases(Vec<Release>);

impl Releases {
    pub fn first_available(&self) -> Result<&Release> {
        let repo = &self
            .0
            .first()
            .map(|r| r.repo.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        self.0
            .iter()
            .find(|r| !r.draft && !r.prerelease && !r.assets.is_empty())
            .ok_or(Error::NoAvailableRelease(repo.to_string()))
    }
}

impl From<Vec<Release>> for Releases {
    fn from(releases: Vec<Release>) -> Self {
        Self(releases)
    }
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

pub struct GitHubProvider {
    client: Client,
}

impl GitHubProvider {
    pub fn new() -> RemoteResult<Self> {
        let client = Client::builder()
            .user_agent(format!("inro/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(Error::HttpClientBuild)?;
        Ok(Self { client })
    }

    pub fn fetch_releases(&self, repo: &str) -> Result<Releases> {
        let api_url = format!("https://api.github.com/repos/{}/releases", repo);

        let mut request_builder = self
            .client
            .get(&api_url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(&[("per_page", GITHUB_RELEASES_PER_PAGE)]);

        if let Ok(token) = env::var("INRO_GITHUB_TOKEN") {
            report!(
                MsgType::Detail,
                "Using INRO_GITHUB_TOKEN for authentication."
            );
            request_builder = request_builder.bearer_auth(token);
        } else if let Ok(token) = env::var("GITHUB_TOKEN") {
            report!(MsgType::Detail, "Using GITHUB_TOKEN for authentication.");
            request_builder = request_builder.bearer_auth(token);
        } else {
            report!(
                MsgType::Warning,
                "Unauthenticated requests are rate-limited. Consider setting INRO_GITHUB_TOKEN or GITHUB_TOKEN environment variable."
            );
        }

        report!(
            MsgType::Detail,
            "Fetching releases from GitHub repository '{repo}'..."
        );

        let response = request_builder.send().map_err(|e| Error::RequestFailed {
            repo: repo.to_string(),
            source: e,
        })?;
        let response = response
            .error_for_status()
            .map_err(|e| Error::RequestFailed {
                repo: repo.to_string(),
                source: e,
            })?;

        let mut release_vec: Vec<Release> = response.json().map_err(|e| Error::JsonParse {
            repo: repo.to_string(),
            source: e,
        })?;
        for release in &mut release_vec {
            release.repo = repo.to_string();
        }

        Ok(Releases(release_vec))
    }
}

impl RemoteProvider for GitHubProvider {
    fn find_candidates(&self, pkg: &PackageInfo) -> RemoteResult<Vec<InstallCandidate>> {
        let repo = match &pkg.remote {
            Remote::GitHub(source) => &source.repo,
        };

        let releases = self.fetch_releases(repo)?;
        let available_release = releases.first_available()?;
        let Remote::GitHub(source) = &pkg.remote;
        let assets = available_release.find_assets(&source.asset)?;

        let candidates: Vec<InstallCandidate> = assets
            .into_iter()
            .map(|asset| InstallCandidate {
                version: available_release.tag_name.clone(),
                asset_name: asset.name.clone(),
                download_url: asset.browser_download_url.clone(),
            })
            .collect();
        Ok(candidates)
    }
}
