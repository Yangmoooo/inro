use std::collections::HashMap;
use std::env;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{AssetSelector, CandidateResult, InstallCandidate, MatchKind, RemoteType};
use crate::package::PkgDef;
use crate::platform::PlatformInfo;
use crate::remotes::VersionInfo;
use crate::utils::{asset_matches_selector, is_ignored_format, is_supported_format};
use crate::{client, detail};

/// 100 is GitHub's per-page maximum. We never paginate beyond the first
/// page; for the 99% of repos with fewer than 100 releases this is the
/// whole list, and for the rest a clear "tag not found in latest 100
/// releases" surfaces faster than walking dozens of pages.
const GITHUB_RELEASES_PER_PAGE: u32 = 100;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to build HTTP client")]
    HttpClientBuild(#[from] reqwest::Error),

    #[error("Failed to start tokio runtime for synchronous GitHub call")]
    RuntimeBuild(#[source] std::io::Error),

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
        "In release tag '{tag}' for repo '{repo}', no asset was found matching the selector '{selector}'"
    )]
    NoMatchingAsset { repo: String, tag: String, selector: String },

    #[error("GitHub API rate limit exceeded for '{repo}'. {hint}")]
    RateLimited { repo: String, hint: String },
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
    size: u64,
    browser_download_url: String,
}

impl Release {
    /// Check if the release is suitable (non-draft, non-prerelease, with
    /// assets).
    fn is_suitable(&self) -> bool { !self.draft && !self.prerelease && !self.assets.is_empty() }

    /// Check if the release is available (non-draft, with assets).
    fn is_available(&self) -> bool { !self.draft && !self.assets.is_empty() }

    /// Find assets matching the given asset map or platform information.
    /// Returns the matched assets and how they were matched.
    fn find_assets(
        &self,
        asset_map: &HashMap<String, AssetSelector>,
    ) -> Result<(Vec<&Asset>, MatchKind)> {
        let platform = PlatformInfo::current();
        self.find_assets_for_platform(asset_map, &platform)
    }

    fn find_assets_for_platform(
        &self,
        asset_map: &HashMap<String, AssetSelector>,
        platform: &PlatformInfo,
    ) -> Result<(Vec<&Asset>, MatchKind)> {
        let platform_key = platform.key();

        // if the platform-specific asset is configured, use its selector
        if let Some(selector) = asset_map.get(&platform_key) {
            detail!("Using explicit configuration for platform '{platform_key}': '{selector}'");
            let mut matching_assets: Vec<(&Asset, i32)> = self
                .assets
                .iter()
                .filter(|asset| {
                    asset_matches_selector(&asset.name, selector)
                        && !is_ignored_format(&asset.name.to_lowercase())
                })
                .map(|asset| {
                    let score = calculate_heuristic_score(asset, platform);
                    (asset, score)
                })
                .collect();
            if matching_assets.is_empty() {
                detail!(
                    "No GitHub assets matched explicit selector '{selector}'. Available assets: {}",
                    format_asset_names(&self.assets)
                );
                return Err(Error::NoMatchingAsset {
                    repo: self.repo.clone(),
                    tag: self.tag_name.clone(),
                    selector: selector.to_string(),
                });
            }
            matching_assets.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
            let matching_assets: Vec<&Asset> =
                matching_assets.into_iter().map(|(asset, _)| asset).collect();
            detail!(
                "Matched {} GitHub asset(s) by explicit configuration: {}",
                matching_assets.len(),
                format_asset_refs(&matching_assets)
            );
            return Ok((matching_assets, MatchKind::Explicit));
        }

        // if not configured, use the os and arch to match the asset name
        let os_aliases = platform.os_aliases();
        let arch_aliases = platform.arch_aliases();
        detail!(
            "Selecting GitHub asset for platform '{platform_key}' using OS aliases [{}] and arch aliases [{}]",
            os_aliases.join(", "),
            arch_aliases.join(", ")
        );

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
                let score = calculate_heuristic_score(asset, platform);
                (asset, score)
            })
            .collect();

        if candidates.is_empty() {
            detail!(
                "No platform-specific GitHub assets matched. Falling back to supported assets from: {}",
                format_asset_names(&self.assets)
            );
            let fallback_assets: Vec<&Asset> = self
                .assets
                .iter()
                .filter(|asset| {
                    let name_lower = asset.name.to_lowercase();
                    is_supported_format(&name_lower) && !is_ignored_format(&name_lower)
                })
                .collect();

            if fallback_assets.is_empty() {
                detail!("No supported GitHub assets remained after filtering ignored formats");
                return Err(Error::NoMatchingAsset {
                    repo: self.repo.clone(),
                    tag: self.tag_name.clone(),
                    selector: platform_key,
                });
            }

            detail!(
                "Found {} fallback GitHub asset(s): {}",
                fallback_assets.len(),
                format_asset_refs(&fallback_assets)
            );
            return Ok((fallback_assets, MatchKind::Fallback));
        }

        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
        detail!(
            "Found {} platform GitHub asset candidate(s): {}",
            candidates.len(),
            format_scored_asset_refs(&candidates)
        );
        let sorted_assets = candidates.into_iter().map(|(asset, _)| asset).collect();
        Ok((sorted_assets, MatchKind::PlatformHeuristic))
    }
}

fn format_asset_names(assets: &[Asset]) -> String {
    assets.iter().map(|asset| asset.name.as_str()).collect::<Vec<_>>().join(", ")
}

fn format_asset_refs(assets: &[&Asset]) -> String {
    assets.iter().map(|asset| asset.name.as_str()).collect::<Vec<_>>().join(", ")
}

fn format_scored_asset_refs(assets: &[(&Asset, i32)]) -> String {
    assets
        .iter()
        .map(|(asset, score)| format!("{} (score {score})", asset.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Convert a 403/429 response with GitHub rate-limit headers into a
/// dedicated `Error::RateLimited`. Other 403s (bad token, repo-level
/// permission denial) fall through and are surfaced via
/// `error_for_status` with their original cause intact.
fn check_rate_limit(response: &reqwest::Response, repo: &str) -> Option<Error> {
    let status = response.status().as_u16();
    if !matches!(status, 403 | 429) {
        return None;
    }

    let headers = response.headers();
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let remaining: Option<u64> = header("x-ratelimit-remaining").and_then(|s| s.parse().ok());

    // A 403 only counts as rate limiting when the remaining counter is 0.
    // Without that hint it is more likely a bad-token / forbidden response.
    if status == 403 && remaining != Some(0) {
        return None;
    }

    let reset_secs: Option<i64> = header("x-ratelimit-reset").and_then(|s| s.parse().ok());
    let retry_after_secs: Option<i64> = header("retry-after").and_then(|s| s.parse().ok());
    let has_token = env::var("INRO_GITHUB_TOKEN").is_ok() || env::var("GITHUB_TOKEN").is_ok();

    let resets_in_secs =
        retry_after_secs.or_else(|| reset_secs.map(|ts| (ts - Utc::now().timestamp()).max(0)));
    Some(Error::RateLimited {
        repo: repo.to_string(),
        hint: format_rate_hint(resets_in_secs, has_token),
    })
}

/// Build a human-readable hint that explains when the limit resets and
/// what to do about it.
fn format_rate_hint(resets_in_secs: Option<i64>, has_token: bool) -> String {
    let token_hint = if has_token {
        "The token may be invalid or you have hit the per-token limit."
    } else {
        "Set INRO_GITHUB_TOKEN or GITHUB_TOKEN to raise the limit (60->5000 req/h)."
    };
    match resets_in_secs {
        Some(secs) => {
            let dt = Utc::now() + chrono::Duration::seconds(secs);
            let when = chrono_humanize::HumanTime::from(dt);
            format!("Resets {when}. {token_hint}")
        }
        None => token_hint.to_string(),
    }
}

/// Calculate a heuristic score for how well an asset matches the platform.
fn calculate_heuristic_score(asset: &Asset, platform: &PlatformInfo) -> i32 {
    let name = asset.name.to_lowercase();
    let mut score = 0;

    if platform.os == "windows" {
        if name.contains("msvc") {
            score += 10;
        } else if name.contains("gnu") {
            score += 0;
        }

        if name.ends_with(".zip") {
            score += 5;
        } else if name.ends_with(".7z") {
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

    if platform.os == "macos" {
        if [".tar.gz", ".tgz", ".tar.xz", ".txz"].iter().any(|ext| name.ends_with(ext)) {
            score += 5;
        } else if name.ends_with(".zip") {
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
    /// List all suitable releases (non-draft, non-prerelease, with assets).
    fn list_suitable(&self) -> Vec<&Release> { self.0.iter().filter(|r| r.is_suitable()).collect() }

    /// Get the latest suitable release.
    fn latest_suitable(&self) -> Result<&Release> {
        let suitable = self.list_suitable();
        suitable.first().copied().ok_or_else(|| {
            let repo = self.0.first().map_or_else(|| "Unknown".to_string(), |r| r.repo.clone());
            Error::NoAvailableRelease(repo)
        })
    }

    /// Get a release by its tag name.
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
            detail!("Using INRO_GITHUB_TOKEN for authentication");
            request_builder = request_builder.bearer_auth(token);
        } else if let Ok(token) = env::var("GITHUB_TOKEN") {
            detail!("Using GITHUB_TOKEN for authentication");
            request_builder = request_builder.bearer_auth(token);
        } else {
            detail!(
                "Unauthenticated requests are rate-limited. Consider setting INRO_GITHUB_TOKEN or GITHUB_TOKEN environment variable"
            );
        }

        detail!("Fetching releases from GitHub repository '{repo}'...");

        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::RequestFailed { repo: repo.to_string(), source: e })?;
        if let Some(rate_err) = check_rate_limit(&response, repo) {
            return Err(rate_err);
        }
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
    ) -> super::Result<CandidateResult> {
        let repo = match &pkg.remote {
            RemoteType::GitHub(asset_def) => &asset_def.repo,
        };

        let releases = self.fetch_releases_async(repo).await?;
        let release =
            if let Some(v) = ver { releases.get_by_tag(v)? } else { releases.latest_suitable()? };

        let RemoteType::GitHub(asset_def) = &pkg.remote;
        let (assets, match_kind) = release.find_assets(&asset_def.asset)?;

        let asset_names = release.assets.iter().map(|asset| asset.name.clone()).collect();
        let candidates: Vec<InstallCandidate> = assets
            .into_iter()
            .map(|asset| InstallCandidate {
                version: release.tag_name.clone(),
                asset_name: asset.name.clone(),
                download_url: asset.browser_download_url.clone(),
                size: asset.size,
            })
            .collect();
        let matched_selector = match (&pkg.remote, match_kind) {
            (RemoteType::GitHub(asset_def), MatchKind::Explicit) => {
                let platform_key = PlatformInfo::current().key();
                asset_def.asset.get(&platform_key).map(ToString::to_string)
            }
            _ => None,
        };
        Ok(CandidateResult { candidates, asset_names, match_kind, matched_selector })
    }

    // ==================== Sync facade (for info/source) ====================

    pub fn list_versions(&self, pkg: &PkgDef) -> super::Result<Vec<VersionInfo>> {
        let repo = match &pkg.remote {
            RemoteType::GitHub(asset_def) => &asset_def.repo,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Error::RuntimeBuild)?;
        let releases = runtime.block_on(self.fetch_releases_async(repo))?;

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

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex, MutexGuard};

    use super::*;
    use crate::remotes::MatchKind;

    /// Serialize tests that manipulate `GITHUB_TOKEN` / `INRO_GITHUB_TOKEN`
    /// env vars so they don't race with each other or with `format_rate_hint`.
    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct TokenEnvGuard {
        _guard: MutexGuard<'static, ()>,
        prev_inro: Option<String>,
        prev_github: Option<String>,
    }

    impl TokenEnvGuard {
        fn clear() -> Self {
            let guard = ENV_LOCK.lock().unwrap();
            let prev_inro = env::var("INRO_GITHUB_TOKEN").ok();
            let prev_github = env::var("GITHUB_TOKEN").ok();
            // SAFETY: serialized by ENV_LOCK.
            unsafe {
                env::remove_var("INRO_GITHUB_TOKEN");
                env::remove_var("GITHUB_TOKEN");
            }
            Self { _guard: guard, prev_inro, prev_github }
        }
    }

    impl Drop for TokenEnvGuard {
        fn drop(&mut self) {
            // SAFETY: serialized by ENV_LOCK.
            match &self.prev_inro {
                Some(v) => unsafe { env::set_var("INRO_GITHUB_TOKEN", v) },
                None => unsafe { env::remove_var("INRO_GITHUB_TOKEN") },
            }
            match &self.prev_github {
                Some(v) => unsafe { env::set_var("GITHUB_TOKEN", v) },
                None => unsafe { env::remove_var("GITHUB_TOKEN") },
            }
        }
    }

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            size: 1024,
            browser_download_url: format!("https://example.com/{name}"),
        }
    }

    fn release_with_assets(assets: Vec<Asset>) -> Release {
        Release {
            repo: "owner/tool".to_string(),
            tag_name: "v1.0.0".to_string(),
            html_url: "https://example.com/release".to_string(),
            prerelease: false,
            draft: false,
            created_at: Utc::now(),
            published_at: Some(Utc::now()),
            assets,
        }
    }

    #[test]
    fn falls_back_to_supported_assets_when_platform_matching_finds_none() {
        let release = release_with_assets(vec![
            asset("tool.tar.gz"),
            asset("tool.sha256"),
            asset("tool.dmg"),
            asset("README.md"),
        ]);

        let (assets, match_kind) = release.find_assets(&HashMap::new()).unwrap();

        assert_eq!(match_kind, MatchKind::Fallback);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].name, "tool.tar.gz");
    }

    #[test]
    fn macos_candidates_prefer_tar_archives_and_tie_break_by_name() {
        let release = release_with_assets(vec![
            asset("tool-b-aarch64-apple-darwin.zip"),
            asset("tool-z-aarch64-apple-darwin.tar.gz"),
            asset("tool-a-aarch64-apple-darwin.tar.gz"),
        ]);
        let platform = PlatformInfo { os: "macos", arch: "aarch64" };

        let (assets, match_kind) =
            release.find_assets_for_platform(&HashMap::new(), &platform).unwrap();

        let names: Vec<_> = assets.into_iter().map(|asset| asset.name.as_str()).collect();
        assert_eq!(match_kind, MatchKind::PlatformHeuristic);
        assert_eq!(
            names,
            vec![
                "tool-a-aarch64-apple-darwin.tar.gz",
                "tool-z-aarch64-apple-darwin.tar.gz",
                "tool-b-aarch64-apple-darwin.zip",
            ]
        );
    }

    #[test]
    fn macos_candidates_include_extensionless_binary_assets() {
        let release = release_with_assets(vec![asset("chsrc-aarch64-macos")]);
        let platform = PlatformInfo { os: "macos", arch: "aarch64" };

        let (assets, match_kind) =
            release.find_assets_for_platform(&HashMap::new(), &platform).unwrap();

        assert_eq!(match_kind, MatchKind::PlatformHeuristic);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].name, "chsrc-aarch64-macos");
    }

    // ==================== Rate-limit hint ====================

    #[test]
    fn rate_hint_with_reset_and_no_token_recommends_setting_one() {
        let _env = TokenEnvGuard::clear();

        let hint = format_rate_hint(Some(60 * 23), false);

        assert!(hint.contains("Resets in"), "missing reset clause: {hint}");
        assert!(hint.contains("INRO_GITHUB_TOKEN"), "missing token suggestion: {hint}");
    }

    #[test]
    fn rate_hint_with_token_blames_the_token() {
        let hint = format_rate_hint(Some(60), true);

        assert!(hint.contains("token may be invalid"), "expected token-specific hint: {hint}");
        assert!(!hint.contains("Set INRO_GITHUB_TOKEN"), "should not suggest setting a token");
    }

    #[test]
    fn rate_hint_without_reset_still_returns_token_advice() {
        let hint = format_rate_hint(None, false);

        assert!(!hint.contains("Resets"), "should not claim a reset time we do not know: {hint}");
        assert!(hint.contains("INRO_GITHUB_TOKEN"), "must still suggest token: {hint}");
    }
}
