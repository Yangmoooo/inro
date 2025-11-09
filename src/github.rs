use std::env;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Release {
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

const GITHUB_RELEASES_PER_PAGE: u32 = 20;

pub fn fetch_releases(repo: &str) -> Result<Releases> {
    let api_url = format!("https://api.github.com/repos/{}/releases", repo);

    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("inro/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Failed to build HTTP client")?;

    let mut request_builder = client
        .get(&api_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .query(&[("per_page", GITHUB_RELEASES_PER_PAGE)]);

    if let Ok(token) = env::var("INRO_GITHUB_TOKEN") {
        println!("-> Using INRO_GITHUB_TOKEN for authentication.");
        request_builder = request_builder.bearer_auth(token);
    } else if let Ok(token) = env::var("GITHUB_TOKEN") {
        println!("-> Using GITHUB_TOKEN for authentication.");
        request_builder = request_builder.bearer_auth(token);
    } else {
        println!("-> No GITHUB_TOKEN found. Making unauthenticated request.");
    }

    println!("-> Fetching all releases for '{repo}'...");

    let response = request_builder
        .send()
        .with_context(|| format!("Failed to send request to {api_url}"))?;

    let response = response
        .error_for_status()
        .context("API request failed. Check repo name, token, or network.")?;

    let releases: Releases = response
        .json()
        .context("Failed to parse JSON response (expected an array of releases)")?;

    Ok(releases)
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
            bail!(
                "Could not find any asset matching the keyword '{}' in release '{}'",
                keyword,
                self.tag_name
            );
        }
        Ok(matching_assets)
    }
}

impl Releases {
    pub fn first_publish(&self) -> Result<&Release> {
        self.0
            .iter()
            .find(|r| !r.draft && !r.prerelease && !r.assets.is_empty())
            .context("No published (non-draft, non-prerelease, with assets) release found.")
    }
}

impl From<Vec<Release>> for Releases {
    fn from(releases: Vec<Release>) -> Self {
        Self(releases)
    }
}
