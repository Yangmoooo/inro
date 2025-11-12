use std::env;

use serde::Deserialize;

use crate::{report, reporter::MsgType};

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
#[serde(transparent)]
pub struct Releases(Vec<Release>);

#[derive(Deserialize, Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub content_type: String,
    pub browser_download_url: String,
}

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

    #[error("No available release found for '{repo}' (non-draft, non-prerelease, with assets)")]
    NoAvailableRelease { repo: String },

    #[error(
        "In release tag '{tag}' for repo '{repo}', no asset was found matching the keyword '{keyword}'"
    )]
    NoMatchingAsset {
        repo: String,
        tag: String,
        keyword: String,
    },
}

const GITHUB_RELEASES_PER_PAGE: u32 = 20;

pub fn fetch_releases(repo: &str) -> Result<Releases> {
    let api_url = format!("https://api.github.com/repos/{}/releases", repo);

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

impl Release {
    pub fn find_assets(&self, keyword: &str) -> Result<Vec<&Asset>> {
        const VALID_TYPES: &[&str] = &[
            "application/octet-stream",
            "application/x-gtar",
            "application/gzip",
            "application/zip",
        ];
        let matching_assets: Vec<&Asset> = self
            .assets
            .iter()
            .filter(|asset| {
                asset.name.contains(keyword) && VALID_TYPES.contains(&asset.content_type.as_str())
            })
            .collect();
        if matching_assets.is_empty() {
            return Err(Error::NoMatchingAsset {
                repo: self.repo.clone(),
                tag: self.tag_name.clone(),
                keyword: keyword.to_string(),
            });
        }
        Ok(matching_assets)
    }
}

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
            .ok_or(Error::NoAvailableRelease {
                repo: repo.to_string(),
            })
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
