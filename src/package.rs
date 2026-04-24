use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::platform::PlatformInfo;
use crate::remotes::RemoteType;
use crate::utils::create_symlink;

/// Package definition as specified in the registry.
#[derive(Clone, Debug, Deserialize)]
pub struct PkgDef {
    #[serde(default)]
    pub ver: Option<String>,
    pub remote: RemoteType,
    #[serde(default)]
    pub bin: Vec<BinDef>,
}

/// A value that can be either a plain string or a platform-specific mapping.
/// Using #[serde(untagged)] for backward compatibility.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum PlatformAwareString {
    /// A plain string value that applies to all platforms
    Literal(String),
    /// A platform-specific mapping, e.g., {"linux-x86_64": "codex",
    /// "macos-aarch64": "codex", "windows-x86_64": "codex.exe"}
    ByPlatform(HashMap<String, String>),
}

impl PlatformAwareString {
    /// Resolve the value for the current platform.
    /// If it's a plain string, returns that.
    /// If it's a map, tries to find a matching platform key or returns None.
    fn resolve_for_platform(&self) -> Option<String> {
        match self {
            PlatformAwareString::Literal(s) => Some(s.clone()),
            PlatformAwareString::ByPlatform(map) => {
                let platform = PlatformInfo::current();
                let platform_key = platform.key();

                // Try exact match first
                if let Some(value) = map.get(&platform_key) {
                    return Some(value.clone());
                }

                // Try to find a matching key using platform and arch aliases
                let os_aliases = platform.os_aliases();
                let arch_aliases = platform.arch_aliases();

                for (key, value) in map.iter() {
                    // Check if the key matches any os-arch combination
                    let key_lower = key.to_lowercase();
                    let os_match = os_aliases.iter().any(|&alias| key_lower.contains(alias));
                    let arch_match = arch_aliases.iter().any(|&alias| key_lower.contains(alias));

                    if os_match && arch_match {
                        return Some(value.clone());
                    }
                }

                None
            }
        }
    }
}

/// Binary definition within a package.
#[derive(Clone, Debug, Deserialize)]
pub struct BinDef {
    #[serde(default)]
    pub name: Option<PlatformAwareString>,
    #[serde(default)]
    pub link: Option<PlatformAwareString>,
}

/// Resolved package definition with finalized parameters.
#[derive(Debug)]
pub struct ResolvedPkg {
    #[allow(dead_code)]
    pub ver: Option<String>,
    pub remote: RemoteType,
    pub bin: Vec<ResolvedBin>,
}

/// Resolved binary definition with concrete names and links.
#[derive(Debug)]
pub struct ResolvedBin {
    pub name: String,
    pub link: String,
}

impl PkgDef {
    /// Resolves the configuration into a definitive set of installation
    /// parameters.
    pub fn resolve(self, pkg_name: &str) -> ResolvedPkg {
        let normalize_name = |name: String| -> String {
            if cfg!(windows) && !name.to_lowercase().ends_with(".exe") {
                format!("{name}.exe")
            } else {
                name
            }
        };

        let bin = if self.bin.is_empty() {
            // binary default name is the package name
            let name = normalize_name(pkg_name.to_string());
            vec![ResolvedBin { name: name.clone(), link: name }]
        } else {
            // process each configured binary
            self.bin
                .into_iter()
                .filter_map(|b| {
                    // Resolve name from PlatformAwareString; if it doesn't match the current
                    // platform, skip this binary instead of falling back to the package name.
                    let raw_name = match b.name {
                        Some(s) => s.resolve_for_platform(),
                        None => Some(pkg_name.to_string()),
                    }?;
                    let name = normalize_name(raw_name);

                    // Resolve link from PlatformAwareString, defaulting to the resolved name
                    let raw_link = b
                        .link
                        .as_ref()
                        .and_then(|s| s.resolve_for_platform())
                        .unwrap_or_else(|| name.clone());
                    let link = normalize_name(raw_link);

                    Some(ResolvedBin { name, link })
                })
                .collect()
        };

        ResolvedPkg { ver: self.ver, remote: self.remote, bin }
    }
}

/// Current state of a package installation.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PkgState {
    // none means installed but not linked
    pub current_version: Option<String>,

    // key: version
    pub versions: HashMap<String, PkgReceipt>,

    // if true, this package will be skipped during updates
    #[serde(default)]
    pub pinned: bool,
}

impl PkgState {
    /// Get the latest **installed** version from PkgState.
    pub fn get_latest_version(&self) -> Option<String> {
        self.versions
            .iter()
            .max_by_key(|(_ver, receipt)| receipt.installed_at)
            .map(|(ver, _receipt)| ver.clone())
    }
}

/// Receipt information for an installed package version.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PkgReceipt {
    /// Package name, e.g. 'ripgrep'.
    pub name: String,

    /// Package version, actually the tag name.
    pub version: String,

    /// Remote info.
    pub remote: RemoteType,

    /// Installation time.
    pub installed_at: DateTime<Utc>,

    /// Installation directory, actually where the binary is extracted to.
    pub install_dir: PathBuf,

    /// Binaries installed details.
    pub binaries: Vec<InstalledBin>,
}

impl PkgReceipt {
    /// Save the receipt to the installation directory.
    pub fn save_to_install_dir(&self) -> Result<()> {
        let receipt_path = self.install_dir.join("inro-receipt.json");
        let receipt_file = File::create(&receipt_path).with_context(|| {
            format!("Failed to create receipt backup: {}", receipt_path.display())
        })?;
        serde_json::to_writer_pretty(receipt_file, self)?;
        Ok(())
    }

    /// Relink the binaries to the target directory.
    pub fn relink(&mut self, target_dir: &Path) -> Result<()> {
        if !target_dir.exists() {
            let _ = fs::create_dir_all(target_dir)
                .with_context(|| format!("Failed to create bin dir: {}", target_dir.display()));
        }

        for bin in &mut self.binaries {
            // Clean up
            // If the old entry is still at there and its parent dir is not the target_dir
            // Thats say the config bin_dir is changed, remove the old link
            if let Some(parent) = bin.link_path.parent()
                && parent != target_dir
                && bin.link_path.exists()
            {
                let _ = fs::remove_file(&bin.link_path);
            }

            // Create new and update
            let target = target_dir.join(&bin.name);
            create_symlink(&bin.bin_path, &target)?;
            bin.link_path = target;
        }
        Ok(())
    }

    /// Unlink the binaries from the target directory.
    pub fn unlink(&self) -> Result<()> {
        for bin in &self.binaries {
            if bin.link_path.exists() || bin.link_path.is_symlink() {
                fs::remove_file(&bin.link_path).with_context(|| {
                    format!("Failed to remove link: {}", bin.link_path.display())
                })?;
            }
        }
        Ok(())
    }
}

/// Information about an installed binary.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstalledBin {
    /// Binary file name, e.g. 'rg' or 'rg.exe'
    pub name: String,

    /// Binary file path, e.g. '.../packages/ripgrep/13.0.0/rg'
    pub bin_path: PathBuf,

    /// Binary symlink path, e.g. '~/.local/bin/rg'
    pub link_path: PathBuf,
}

#[derive(thiserror::Error, Debug)]
pub enum PkgError {
    #[error("Package '{0}' not found in registry")]
    NotFound(String),

    #[error("No suitable release found for this platform")]
    NoCandidates,

    #[error("Remote error: {0}")]
    Remote(#[from] crate::remotes::Error),

    #[error("Download failed: '{0}'")]
    Download(#[from] anyhow::Error),

    #[error("Failed to extract archive '{filename}'")]
    Extraction {
        filename: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Could not find the binary '{0}' inside the extracted archive")]
    BinaryNotFoundInArchive(String),

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remotes::{self, GitHubAssetDef};

    fn make_pkg_def(bins: Vec<BinDef>) -> PkgDef {
        PkgDef {
            ver: Some("v1.0.0".to_string()),
            remote: RemoteType::GitHub(GitHubAssetDef {
                repo: "test/repo".to_string(),
                asset: HashMap::new(),
            }),
            bin: bins,
        }
    }

    #[test]
    fn remote_error_display_includes_specific_cause() {
        let error =
            PkgError::Remote(remotes::Error::GitHub(remotes::github::Error::NoMatchingAsset {
                repo: "owner/tool".to_string(),
                tag: "v1.0.0".to_string(),
                keyword: "macos-aarch64".to_string(),
            }));

        let message = error.to_string();

        assert!(message.contains("owner/tool"));
        assert!(message.contains("macos-aarch64"));
        assert!(!message.contains("Failed to fetch from the upstream"));
        assert!(!message.contains("Failed fetching from GitHub"));
    }

    // ==================== PkgDef::resolve() ====================

    #[test]
    fn resolve_empty_bins_uses_package_name() {
        let pkg_def = make_pkg_def(vec![]);
        let resolved = pkg_def.resolve("ripgrep");

        assert_eq!(resolved.bin.len(), 1);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "ripgrep");
            assert_eq!(resolved.bin[0].link, "ripgrep");
        }
        #[cfg(windows)]
        {
            assert_eq!(resolved.bin[0].name, "ripgrep.exe");
            assert_eq!(resolved.bin[0].link, "ripgrep.exe");
        }
    }

    #[test]
    fn resolve_custom_bin_name() {
        let pkg_def = make_pkg_def(vec![BinDef {
            name: Some(PlatformAwareString::Literal("rg".to_string())),
            link: None,
        }]);
        let resolved = pkg_def.resolve("ripgrep");

        assert_eq!(resolved.bin.len(), 1);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "rg");
            assert_eq!(resolved.bin[0].link, "rg");
        }
    }

    #[test]
    fn resolve_custom_bin_name_and_link() {
        let pkg_def = make_pkg_def(vec![BinDef {
            name: Some(PlatformAwareString::Literal("rg".to_string())),
            link: Some(PlatformAwareString::Literal("ripgrep".to_string())),
        }]);
        let resolved = pkg_def.resolve("ripgrep");

        assert_eq!(resolved.bin.len(), 1);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "rg");
            assert_eq!(resolved.bin[0].link, "ripgrep");
        }
    }

    #[test]
    fn resolve_multiple_binaries() {
        let pkg_def = make_pkg_def(vec![
            BinDef { name: Some(PlatformAwareString::Literal("uv".to_string())), link: None },
            BinDef { name: Some(PlatformAwareString::Literal("uvx".to_string())), link: None },
        ]);
        let resolved = pkg_def.resolve("uv");

        assert_eq!(resolved.bin.len(), 2);
    }

    #[test]
    fn resolve_preserves_version() {
        let pkg_def = make_pkg_def(vec![]);
        let resolved = pkg_def.resolve("test");

        assert_eq!(resolved.ver, Some("v1.0.0".to_string()));
    }

    // ==================== PkgState::get_latest_version() ====================

    #[test]
    fn pkg_state_get_latest_version_empty() {
        let state = PkgState::default();
        assert!(state.get_latest_version().is_none());
    }

    #[test]
    fn pkg_state_get_latest_version_single() {
        let mut state = PkgState::default();
        state.versions.insert(
            "1.0.0".to_string(),
            PkgReceipt {
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                remote: RemoteType::default(),
                installed_at: Utc::now(),
                install_dir: PathBuf::from("/tmp"),
                binaries: vec![],
            },
        );

        assert_eq!(state.get_latest_version(), Some("1.0.0".to_string()));
    }

    #[test]
    fn pkg_state_get_latest_version_by_install_time() {
        use chrono::Duration;

        let mut state = PkgState::default();
        let now = Utc::now();

        // Older version installed first
        state.versions.insert(
            "1.0.0".to_string(),
            PkgReceipt {
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                remote: RemoteType::default(),
                installed_at: now - Duration::hours(1),
                install_dir: PathBuf::from("/tmp/1.0.0"),
                binaries: vec![],
            },
        );

        // Newer version installed later
        state.versions.insert(
            "2.0.0".to_string(),
            PkgReceipt {
                name: "test".to_string(),
                version: "2.0.0".to_string(),
                remote: RemoteType::default(),
                installed_at: now,
                install_dir: PathBuf::from("/tmp/2.0.0"),
                binaries: vec![],
            },
        );

        // Should return the one with latest installed_at
        assert_eq!(state.get_latest_version(), Some("2.0.0".to_string()));
    }

    // ==================== Platform-specific binary names ====================

    #[test]
    fn resolve_platform_specific_name() {
        let platform = PlatformInfo::current();
        let platform_key = platform.key();

        let mut name_map = HashMap::new();
        name_map.insert(platform_key.clone(), "platform-specific-bin".to_string());
        name_map.insert("other-platform".to_string(), "other-bin".to_string());

        let pkg_def = make_pkg_def(vec![BinDef {
            name: Some(PlatformAwareString::ByPlatform(name_map)),
            link: None,
        }]);
        let resolved = pkg_def.resolve("test");

        assert_eq!(resolved.bin.len(), 1);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "platform-specific-bin");
            assert_eq!(resolved.bin[0].link, "platform-specific-bin");
        }
        #[cfg(windows)]
        {
            assert_eq!(resolved.bin[0].name, "platform-specific-bin.exe");
            assert_eq!(resolved.bin[0].link, "platform-specific-bin.exe");
        }
    }

    #[test]
    fn resolve_platform_specific_name_and_link() {
        let platform = PlatformInfo::current();
        let platform_key = platform.key();

        let mut name_map = HashMap::new();
        name_map.insert(platform_key.clone(), "codex-x86_64-pc-windows-msvc".to_string());

        let mut link_map = HashMap::new();
        link_map.insert(platform_key.clone(), "codex".to_string());

        let pkg_def = make_pkg_def(vec![BinDef {
            name: Some(PlatformAwareString::ByPlatform(name_map)),
            link: Some(PlatformAwareString::ByPlatform(link_map)),
        }]);
        let resolved = pkg_def.resolve("test");

        assert_eq!(resolved.bin.len(), 1);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "codex-x86_64-pc-windows-msvc");
            assert_eq!(resolved.bin[0].link, "codex");
        }
        #[cfg(windows)]
        {
            assert_eq!(resolved.bin[0].name, "codex-x86_64-pc-windows-msvc.exe");
            assert_eq!(resolved.bin[0].link, "codex.exe");
        }
    }

    #[test]
    fn resolve_multiple_platform_specific_binaries() {
        let platform = PlatformInfo::current();
        let platform_key = platform.key();

        let mut name_map1 = HashMap::new();
        name_map1.insert(platform_key.clone(), "bin1".to_string());

        let mut link_map1 = HashMap::new();
        link_map1.insert(platform_key.clone(), "link1".to_string());

        let mut name_map2 = HashMap::new();
        name_map2.insert(platform_key.clone(), "bin2".to_string());

        let pkg_def = make_pkg_def(vec![
            BinDef {
                name: Some(PlatformAwareString::ByPlatform(name_map1)),
                link: Some(PlatformAwareString::ByPlatform(link_map1)),
            },
            BinDef { name: Some(PlatformAwareString::ByPlatform(name_map2)), link: None },
        ]);
        let resolved = pkg_def.resolve("test");

        assert_eq!(resolved.bin.len(), 2);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "bin1");
            assert_eq!(resolved.bin[0].link, "link1");
            assert_eq!(resolved.bin[1].name, "bin2");
            assert_eq!(resolved.bin[1].link, "bin2");
        }
    }

    #[test]
    fn resolve_platform_specific_with_aliases() {
        // Test that platform aliases work
        let platform = PlatformInfo::current();

        let mut name_map = HashMap::new();
        // Use a fuzzy key that should match using aliases
        if platform.os == "linux" {
            name_map.insert("linux-x86_64".to_string(), "matched-bin".to_string());
        } else if platform.os == "windows" {
            name_map.insert("windows-x86_64".to_string(), "matched-bin".to_string());
        } else if platform.os == "macos" {
            name_map.insert("darwin-aarch64".to_string(), "matched-bin".to_string());
        }

        let pkg_def = make_pkg_def(vec![BinDef {
            name: Some(PlatformAwareString::ByPlatform(name_map)),
            link: None,
        }]);
        let resolved = pkg_def.resolve("test");

        // Only test if we can match
        if !resolved.bin.is_empty() {
            #[cfg(not(windows))]
            {
                assert_eq!(resolved.bin[0].name, "matched-bin");
            }
            #[cfg(windows)]
            {
                assert_eq!(resolved.bin[0].name, "matched-bin.exe");
            }
        }
    }

    #[test]
    fn resolve_mixed_string_and_map_binaries() {
        let platform = PlatformInfo::current();
        let platform_key = platform.key();

        let mut name_map = HashMap::new();
        name_map.insert(platform_key.clone(), "platform-bin".to_string());

        let pkg_def = make_pkg_def(vec![
            BinDef {
                name: Some(PlatformAwareString::Literal("simple-bin".to_string())),
                link: None,
            },
            BinDef { name: Some(PlatformAwareString::ByPlatform(name_map)), link: None },
        ]);
        let resolved = pkg_def.resolve("test");

        assert_eq!(resolved.bin.len(), 2);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "simple-bin");
            assert_eq!(resolved.bin[1].name, "platform-bin");
        }
    }

    #[test]
    fn resolve_platform_specific_without_match_is_skipped() {
        // Test that when platform doesn't match, the binary definition is skipped
        let mut name_map = HashMap::new();
        name_map.insert("nonexistent-platform-xyz".to_string(), "nonexistent-bin".to_string());

        let pkg_def = make_pkg_def(vec![BinDef {
            name: Some(PlatformAwareString::ByPlatform(name_map)),
            link: None,
        }]);
        let resolved = pkg_def.resolve("fallback-test");

        assert!(resolved.bin.is_empty());
    }

    // ==================== PlatformAwareString deserialization ====================

    #[test]
    fn deserialize_platform_aware_string_literal() {
        let toml = r#"
            name = "test-bin"
        "#;

        let result: Result<BinDef, _> = toml::from_str(toml);
        assert!(result.is_ok());
        let bin_def = result.unwrap();
        assert!(bin_def.name.is_some());

        if let Some(PlatformAwareString::Literal(s)) = bin_def.name {
            assert_eq!(s, "test-bin");
        } else {
            panic!("Expected PlatformAwareString::Literal");
        }
    }

    #[test]
    fn deserialize_platform_aware_string_by_platform() {
        let toml = r#"
            [name]
            "linux-x86_64" = "linux-bin"
            "macos-aarch64" = "macos-bin"
            "windows-x86_64" = "windows-bin.exe"
        "#;

        let result: Result<BinDef, _> = toml::from_str(toml);
        assert!(result.is_ok());
        let bin_def = result.unwrap();
        assert!(bin_def.name.is_some());

        if let Some(PlatformAwareString::ByPlatform(map)) = bin_def.name {
            assert_eq!(map.get("linux-x86_64"), Some(&"linux-bin".to_string()));
            assert_eq!(map.get("macos-aarch64"), Some(&"macos-bin".to_string()));
            assert_eq!(map.get("windows-x86_64"), Some(&"windows-bin.exe".to_string()));
        } else {
            panic!("Expected PlatformAwareString::ByPlatform");
        }
    }

    #[test]
    fn deserialize_full_bin_def_with_platform_specific() {
        let toml = r#"
            [name]
            "linux-x86_64" = "codex-x86_64-unknown-linux-musl"
            "macos-aarch64" = "codex-aarch64-apple-darwin"
            "windows-x86_64" = "codex-x86_64-pc-windows-msvc.exe"

            [link]
            "linux-x86_64" = "codex"
            "macos-aarch64" = "codex"
            "windows-x86_64" = "codex"
        "#;

        let result: Result<BinDef, _> = toml::from_str(toml);
        assert!(result.is_ok());
        let bin_def = result.unwrap();

        assert!(bin_def.name.is_some());
        assert!(bin_def.link.is_some());
    }

    #[test]
    fn deserialize_complete_pkg_def_with_platform_specific() {
        let toml = r#"
ver = "v1.0.0"

[remote.github]
repo = "example/codex"

[[bin]]
[bin.name]
"linux-x86_64" = "codex-x86_64-unknown-linux-musl"
"macos-aarch64" = "codex-aarch64-apple-darwin"
"windows-x86_64" = "codex-x86_64-pc-windows-msvc.exe"

[bin.link]
"linux-x86_64" = "codex"
"macos-aarch64" = "codex"
"windows-x86_64" = "codex"

[[bin]]
[bin.name]
"windows-x86_64" = "codex-windows-sandbox-setup.exe"

[[bin]]
[bin.name]
"windows-x86_64" = "codex-command-runner.exe"
        "#;

        let result: Result<PkgDef, _> = toml::from_str(toml);
        assert!(result.is_ok(), "Failed to deserialize: {:?}", result.err());

        let pkg_def = result.unwrap();
        assert_eq!(pkg_def.ver, Some("v1.0.0".to_string()));
        assert_eq!(pkg_def.bin.len(), 3);

        // Resolve and check that it works for current platform
        let resolved = pkg_def.resolve("codex");

        #[cfg(target_os = "linux")]
        {
            // Only the linux-specific binary should be kept
            assert_eq!(resolved.bin.len(), 1);
            assert_eq!(resolved.bin[0].name, "codex-x86_64-unknown-linux-musl");
            assert_eq!(resolved.bin[0].link, "codex");
        }

        #[cfg(target_os = "macos")]
        {
            // Only the macOS-specific binary should be kept
            assert_eq!(resolved.bin.len(), 1);
            assert_eq!(resolved.bin[0].name, "codex-aarch64-apple-darwin");
            assert_eq!(resolved.bin[0].link, "codex");
        }

        #[cfg(target_os = "windows")]
        {
            assert_eq!(resolved.bin.len(), 3);
            // All three binaries should match
            assert_eq!(resolved.bin.len(), 3);
            assert_eq!(resolved.bin[0].name, "codex-x86_64-pc-windows-msvc.exe");
            assert_eq!(resolved.bin[0].link, "codex.exe");
            assert_eq!(resolved.bin[1].name, "codex-windows-sandbox-setup.exe");
            assert_eq!(resolved.bin[2].name, "codex-command-runner.exe");
        }
    }

    #[test]
    fn backward_compatibility_with_string_binaries() {
        // Ensure old-style string binaries still work
        let toml = r#"
ver = "v1.0.0"

[remote.github]
repo = "example/simple"

[[bin]]
name = "simple-bin"
link = "simple-link"
        "#;

        let result: Result<PkgDef, _> = toml::from_str(toml);
        assert!(result.is_ok());

        let pkg_def = result.unwrap();
        let resolved = pkg_def.resolve("simple");

        assert_eq!(resolved.bin.len(), 1);
        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "simple-bin");
            assert_eq!(resolved.bin[0].link, "simple-link");
        }
        #[cfg(windows)]
        {
            assert_eq!(resolved.bin[0].name, "simple-bin.exe");
            assert_eq!(resolved.bin[0].link, "simple-link.exe");
        }
    }
}
