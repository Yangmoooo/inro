use std::env;
use std::io;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;
use tempfile::Builder;

use super::RemoteProvider;
use crate::package::{PackageInfo, Remote};
use crate::report;
use crate::reporter::MsgType;

const GITHUB_RELEASES_PER_PAGE: u32 = 20;
const GITHUB_ASSETS_VALID_TYPES: &[&str] = &[
    "application/octet-stream",
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

    #[error("Download failed for '{url}'")]
    Download {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
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
    pub fn find_assets(&self, keyword: &str) -> Result<Vec<&Asset>> {
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
        Ok(matching_assets)
    }
}

pub async fn fetch_releases(repo: &str) -> Result<Releases> {
    let api_url = format!("https://api.github.com/repos/{}/releases", repo);

    let client = reqwest::Client::builder()
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

    let response = request_builder
        .send()
        .await
        .map_err(|e| Error::RequestFailed {
            repo: repo.to_string(),
            source: e,
        })?;
    let response = response
        .error_for_status()
        .map_err(|e| Error::RequestFailed {
            repo: repo.to_string(),
            source: e,
        })?;

    let mut release_vec: Vec<Release> = response.json().await.map_err(|e| Error::JsonParse {
        repo: repo.to_string(),
        source: e,
    })?;
    for release in &mut release_vec {
        release.repo = repo.to_string();
    }

    Ok(Releases(release_vec))
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
            .ok_or_else(|| Error::NoAvailableRelease {
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

pub struct GitHubProvider {
    pkg: PackageInfo,
}

#[async_trait]
impl RemoteProvider for GitHubProvider {
    async fn download_asset(&self, dest_dir: &Path) -> Result<PathBuf> {
        let source = match &self.pkg.remote {
            Remote::GitHub(source) => source,
            // _ => return Err(super::Error::UnsupportedSourceType("github".to_string())),
        };

        let releases = fetch_releases(&source.repo).await?;
        let latest_release = releases.first_available()?;

        let os = env::consts::OS;
        let asset_keyword = source.asset.get(os).ok_or_else(|| Error::NoMatchingAsset {
            repo: source.repo.clone(),
            tag: latest_release.tag_name.clone(),
            keyword: os.to_string(),
        })?;

        let assets = latest_release.find_assets(asset_keyword)?;
        let asset = assets.first().unwrap(); // find_assets ensures at least one

        let url = &asset.browser_download_url;
        let response = reqwest::get(url).await.map_err(|e| Error::Download {
            url: url.clone(),
            source: e,
        })?;

        let tmp_dir = Builder::new().prefix("inro-").tempdir()?;
        let file_name = Path::new(url)
            .file_name()
            .unwrap_or_else(|| "inro-download.tmp".as_ref());
        let tmp_path = tmp_dir.path().join(file_name);

        let mut tmp_file = tokio::fs::File::create(&tmp_path).await?;
        let mut content = io::Cursor::new(response.bytes().await.map_err(|e| Error::Download {
            url: url.clone(),
            source: e,
        })?);
        tokio::io::copy(&mut content, &mut tmp_file).await?;

        let dest_path = dest_dir.join(file_name);
        tokio::fs::rename(&tmp_path, &dest_path).await?;

        Ok(dest_path)
    }
}
