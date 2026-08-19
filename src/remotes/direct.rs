use reqwest::Url;

use super::{CandidateResult, DirectRemoteDef, InstallCandidate, MatchKind};
use crate::platform::PlatformInfo;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Direct remote has no versions configured")]
    NoVersions,

    #[error(
        "Direct remote defines multiple versions; specify an exact version with '<package>@<version>'"
    )]
    VersionRequired,

    #[error("Version '{0}' is not defined by the direct remote")]
    VersionNotFound(String),

    #[error("Version '{version}' has no direct URL for platform '{platform}'")]
    PlatformUnavailable { version: String, platform: String },

    #[error("Invalid direct URL for version '{version}': {url}")]
    InvalidUrl { version: String, url: String },

    #[error("Direct URL for version '{version}' must use http or https: {url}")]
    UnsupportedUrlScheme { version: String, url: String },

    #[error("Direct URL for version '{version}' has no asset filename: {url}")]
    MissingAssetName { version: String, url: String },
}

pub struct DirectProvider;

impl DirectProvider {
    pub fn find_candidates(
        &self,
        remote: &DirectRemoteDef,
        requested_version: Option<&str>,
    ) -> Result<CandidateResult, Error> {
        let version = match requested_version {
            Some(version) => version,
            None if remote.versions.is_empty() => return Err(Error::NoVersions),
            None if remote.versions.len() > 1 => return Err(Error::VersionRequired),
            None => remote.versions.keys().next().expect("single direct version must exist"),
        };
        let urls = remote
            .versions
            .get(version)
            .ok_or_else(|| Error::VersionNotFound(version.to_string()))?;
        let platform = PlatformInfo::current().key();
        let download_url = urls
            .get(&platform)
            .ok_or_else(|| Error::PlatformUnavailable {
                version: version.to_string(),
                platform: platform.clone(),
            })?
            .clone();
        let parsed = Url::parse(&download_url).map_err(|_| Error::InvalidUrl {
            version: version.to_string(),
            url: download_url.clone(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Error::UnsupportedUrlScheme {
                version: version.to_string(),
                url: download_url,
            });
        }
        let asset_name = parsed
            .path_segments()
            .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
            .filter(|name| *name != "." && *name != "..")
            .ok_or_else(|| Error::MissingAssetName {
                version: version.to_string(),
                url: download_url.clone(),
            })?
            .to_string();

        Ok(CandidateResult {
            candidates: vec![InstallCandidate {
                version: version.to_string(),
                asset_name: asset_name.clone(),
                download_url,
                size: 0,
            }],
            asset_names: vec![asset_name],
            match_kind: MatchKind::Explicit,
            matched_selector: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn remote(versions: &[(&str, &str, &str)]) -> DirectRemoteDef {
        let mut configured = BTreeMap::new();
        for (version, platform, url) in versions {
            configured
                .entry((*version).to_string())
                .or_insert_with(BTreeMap::new)
                .insert((*platform).to_string(), (*url).to_string());
        }
        DirectRemoteDef { versions: configured }
    }

    #[test]
    fn unversioned_request_uses_the_only_version() {
        let platform = PlatformInfo::current().key();
        let remote = remote(&[("3.53.4", &platform, "https://example.com/sqlite.zip?download=1")]);

        let result = DirectProvider.find_candidates(&remote, None).unwrap();

        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].version, "3.53.4");
        assert_eq!(result.candidates[0].asset_name, "sqlite.zip");
    }

    #[test]
    fn unversioned_request_rejects_multiple_versions() {
        let platform = PlatformInfo::current().key();
        let remote = remote(&[
            ("3.52.0", &platform, "https://example.com/sqlite-352.zip"),
            ("3.53.4", &platform, "https://example.com/sqlite-353.zip"),
        ]);

        let error = DirectProvider.find_candidates(&remote, None).unwrap_err();

        assert!(matches!(error, Error::VersionRequired));
    }

    #[test]
    fn exact_version_selects_its_url() {
        let platform = PlatformInfo::current().key();
        let remote = remote(&[
            ("3.52.0", &platform, "https://example.com/sqlite-352.zip"),
            ("3.53.4", &platform, "https://example.com/sqlite-353.zip"),
        ]);

        let result = DirectProvider.find_candidates(&remote, Some("3.52.0")).unwrap();

        assert_eq!(result.candidates[0].version, "3.52.0");
        assert_eq!(result.candidates[0].download_url, "https://example.com/sqlite-352.zip");
    }

    #[test]
    fn exact_version_must_exist() {
        let platform = PlatformInfo::current().key();
        let remote = remote(&[("3.53.4", &platform, "https://example.com/sqlite.zip")]);

        let error = DirectProvider.find_candidates(&remote, Some("3.52.0")).unwrap_err();

        assert!(matches!(error, Error::VersionNotFound(version) if version == "3.52.0"));
    }

    #[test]
    fn current_platform_must_have_a_url() {
        let remote =
            remote(&[("3.53.4", "unsupported-platform", "https://example.com/sqlite.zip")]);

        let error = DirectProvider.find_candidates(&remote, None).unwrap_err();

        assert!(matches!(error, Error::PlatformUnavailable { .. }));
    }
}
