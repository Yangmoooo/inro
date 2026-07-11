use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::platform::PlatformInfo;
use crate::remotes::RemoteType;
use crate::utils::{
    create_symlink, ensure_link_replaceable, is_inro_managed_symlink, symlink_points_to,
    validate_path_component,
};
use crate::warn;

/// Package definition as specified in the registry.
#[derive(Clone, Debug, Deserialize)]
pub struct PkgDef {
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
    pub fn resolve(&self, pkg_name: &str) -> ResolvedPkg {
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
                .iter()
                .filter_map(|b| {
                    // Resolve name from PlatformAwareString; if it doesn't match the current
                    // platform, skip this binary instead of falling back to the package name.
                    let raw_name = match &b.name {
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

        ResolvedPkg { remote: self.remote.clone(), bin }
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
///
/// Paths are stored as portable subpaths and resolved against the current
/// `pkgs_dir` / `bin_dir` at runtime, so the manifest survives layout
/// changes (e.g. relocating `$INRO_HOME`).
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

    /// Install subdirectory under `pkgs_dir`, e.g. "ripgrep/15.0.0".
    pub install_subdir: PathBuf,

    /// Binaries installed details.
    pub binaries: Vec<InstalledBin>,
}

impl PkgReceipt {
    /// Absolute install directory under the given `pkgs_dir`.
    pub fn install_dir(&self, pkgs_dir: &Path) -> PathBuf { pkgs_dir.join(&self.install_subdir) }

    /// Absolute path to a binary on disk.
    pub fn bin_path(&self, bin: &InstalledBin, pkgs_dir: &Path) -> PathBuf {
        self.install_dir(pkgs_dir).join(&bin.bin_subpath)
    }

    /// Absolute symlink path for a binary under the given `bin_dir`.
    pub fn link_path(&self, bin: &InstalledBin, bin_dir: &Path) -> PathBuf {
        bin_dir.join(&bin.name)
    }

    /// Save the receipt to the installation directory.
    pub fn save_to_install_dir(&self, pkgs_dir: &Path) -> Result<()> {
        let receipt_path = self.install_dir(pkgs_dir).join("inro-receipt.json");
        let receipt_file = File::create(&receipt_path).with_context(|| {
            format!("Failed to create receipt backup: {}", receipt_path.display())
        })?;
        serde_json::to_writer_pretty(receipt_file, self)?;
        Ok(())
    }

    /// Link the binaries into `bin_dir`. Overwrites only links inro itself
    /// already manages (i.e. pointing inside `pkgs_dir`); foreign files are
    /// refused by the underlying `create_symlink`.
    pub fn relink(&self, bin_dir: &Path, pkgs_dir: &Path) -> Result<()> {
        self.relink_with(bin_dir, pkgs_dir, create_symlink)
    }

    fn relink_with<F>(&self, bin_dir: &Path, pkgs_dir: &Path, mut link_file: F) -> Result<()>
    where
        F: FnMut(&Path, &Path, &Path) -> Result<()>,
    {
        fs::create_dir_all(bin_dir)
            .with_context(|| format!("Failed to create bin dir: {}", bin_dir.display()))?;

        let mut previous_links = Vec::with_capacity(self.binaries.len());
        for bin in &self.binaries {
            validate_path_component(&bin.name, "link name")?;
            let link = self.link_path(bin, bin_dir);
            ensure_link_replaceable(&link, pkgs_dir)?;
            let previous_target = if link.is_symlink() {
                Some(fs::read_link(&link).with_context(|| {
                    format!("Failed to read existing symlink: {}", link.display())
                })?)
            } else {
                None
            };
            previous_links.push((link, previous_target));
        }

        for (index, bin) in self.binaries.iter().enumerate() {
            let target = &previous_links[index].0;
            let original = self.bin_path(bin, pkgs_dir);
            if let Err(link_error) = link_file(&original, target, pkgs_dir) {
                if let Err(rollback_error) =
                    rollback_links(&previous_links[..=index], pkgs_dir, &mut link_file)
                {
                    return Err(anyhow::anyhow!(
                        "{link_error}; additionally failed to roll back links: {rollback_error}"
                    ));
                }
                return Err(link_error);
            }
        }
        Ok(())
    }

    /// Remove the binaries' symlinks from `bin_dir`. Only entries that are
    /// still inro-managed symlinks (pointing inside `pkgs_dir`) are removed;
    /// anything the user replaced by hand is left alone.
    pub fn unlink(&self, bin_dir: &Path, pkgs_dir: &Path) -> Result<()> {
        for bin in &self.binaries {
            validate_path_component(&bin.name, "link name")?;
            let link = self.link_path(bin, bin_dir);
            if !link.exists() && !link.is_symlink() {
                continue;
            }
            if !is_inro_managed_symlink(&link, pkgs_dir) {
                warn!("Skipping '{}': it is not a symlink managed by inro", link.display());
                continue;
            }
            if !symlink_points_to(&link, &self.bin_path(bin, pkgs_dir)) {
                continue;
            }
            fs::remove_file(&link)
                .with_context(|| format!("Failed to remove link: {}", link.display()))?;
        }
        Ok(())
    }
}

fn rollback_links<F>(
    links: &[(PathBuf, Option<PathBuf>)],
    pkgs_dir: &Path,
    link_file: &mut F,
) -> Result<()>
where
    F: FnMut(&Path, &Path, &Path) -> Result<()>,
{
    for (link, previous_target) in links.iter().rev() {
        if link.is_symlink() {
            if !is_inro_managed_symlink(link, pkgs_dir) {
                anyhow::bail!("Refusing to roll back foreign symlink '{}'", link.display());
            }
            fs::remove_file(link).with_context(|| {
                format!("Failed to remove link during rollback: {}", link.display())
            })?;
        } else if link.exists() {
            anyhow::bail!("Refusing to roll back foreign file '{}'", link.display());
        }

        if let Some(previous_target) = previous_target {
            link_file(previous_target, link, pkgs_dir)?;
        }
    }
    Ok(())
}

/// Information about an installed binary.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstalledBin {
    /// Final link name, e.g. 'rg' or 'rg.exe'.
    pub name: String,

    /// Binary subpath under `install_dir`, e.g. "rg" or "bin/rg".
    pub bin_subpath: PathBuf,
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
                selector: "macos-aarch64".to_string(),
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
                install_subdir: PathBuf::from("test/1.0.0"),
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
                install_subdir: PathBuf::from("test/1.0.0"),
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
                install_subdir: PathBuf::from("test/2.0.0"),
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

    // ==================== PkgReceipt::relink() / unlink() ====================

    #[cfg(unix)]
    fn make_receipt(install_subdir: &str, bin_name: &str) -> PkgReceipt {
        PkgReceipt {
            name: "tool".to_string(),
            version: "v1.0.0".to_string(),
            remote: RemoteType::GitHub(GitHubAssetDef {
                repo: "test/tool".to_string(),
                asset: HashMap::new(),
            }),
            installed_at: Utc::now(),
            install_subdir: PathBuf::from(install_subdir),
            binaries: vec![InstalledBin {
                name: bin_name.to_string(),
                bin_subpath: PathBuf::from(bin_name),
            }],
        }
    }

    #[cfg(unix)]
    #[test]
    fn relink_propagates_target_dir_creation_failure() {
        // Pick a target path whose parent is not a directory, so create_dir_all
        // is guaranteed to fail. relink() must surface that error instead of
        // silently swallowing it and failing later in create_symlink.
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("not-a-dir");
        fs::write(&file_path, b"file content").unwrap();
        let unreachable_dir = file_path.join("bin");

        let pkgs_dir = tmp.path().join("pkgs");
        let install_dir = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(install_dir.join("tool"), b"\x7fELF").unwrap();

        let receipt = make_receipt("tool/v1.0.0", "tool");
        let err = receipt.relink(&unreachable_dir, &pkgs_dir).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Failed to create bin dir"), "unexpected error chain: {msg}");
    }

    #[cfg(unix)]
    #[test]
    fn relink_refuses_to_overwrite_foreign_file() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&pkgs_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();

        // Pretend the user has a pre-existing tool from another package manager
        // sitting at the destination.
        let foreign_path = bin_dir.join("tool");
        fs::write(&foreign_path, b"\x7fELF foreign binary").unwrap();

        // inro's own binary lives under pkgs_dir.
        let install_dir = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(install_dir.join("tool"), b"\x7fELF").unwrap();

        let receipt = make_receipt("tool/v1.0.0", "tool");
        let err = receipt.relink(&bin_dir, &pkgs_dir).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Refusing to overwrite"), "unexpected error: {msg}");
        // Foreign file must remain intact.
        assert_eq!(fs::read(&foreign_path).unwrap(), b"\x7fELF foreign binary");
    }

    #[cfg(unix)]
    #[test]
    fn relink_replaces_existing_inro_owned_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&pkgs_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();

        // An older inro install: link points at v0.9.0 inside pkgs_dir.
        let old_install = pkgs_dir.join("tool/v0.9.0");
        fs::create_dir_all(&old_install).unwrap();
        let old_target = old_install.join("tool");
        fs::write(&old_target, b"\x7fELF old").unwrap();
        let link_path = bin_dir.join("tool");
        std::os::unix::fs::symlink(&old_target, &link_path).unwrap();

        // Now relink to a fresh v1.0.0.
        let new_install = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&new_install).unwrap();
        let new_target = new_install.join("tool");
        fs::write(&new_target, b"\x7fELF new").unwrap();

        let receipt = make_receipt("tool/v1.0.0", "tool");
        receipt.relink(&bin_dir, &pkgs_dir).unwrap();

        let resolved = std::fs::read_link(&link_path).unwrap();
        assert_eq!(resolved, new_target);
    }

    #[cfg(unix)]
    #[test]
    fn relink_checks_all_destinations_before_changing_links() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        let old_install = pkgs_dir.join("tool/v0.9.0");
        fs::create_dir_all(&old_install).unwrap();
        let old_target = old_install.join("tool");
        fs::write(&old_target, b"old").unwrap();
        let first_link = bin_dir.join("tool");
        std::os::unix::fs::symlink(&old_target, &first_link).unwrap();

        let new_install = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&new_install).unwrap();
        fs::write(new_install.join("tool"), b"new").unwrap();
        fs::write(new_install.join("helper"), b"new helper").unwrap();
        fs::write(bin_dir.join("helper"), b"user-owned").unwrap();

        let receipt = PkgReceipt {
            name: "tool".to_string(),
            version: "v1.0.0".to_string(),
            remote: RemoteType::default(),
            installed_at: Utc::now(),
            install_subdir: PathBuf::from("tool/v1.0.0"),
            binaries: vec![
                InstalledBin { name: "tool".to_string(), bin_subpath: PathBuf::from("tool") },
                InstalledBin { name: "helper".to_string(), bin_subpath: PathBuf::from("helper") },
            ],
        };

        receipt.relink(&bin_dir, &pkgs_dir).unwrap_err();

        assert_eq!(fs::read_link(first_link).unwrap(), old_target);
        assert_eq!(fs::read(bin_dir.join("helper")).unwrap(), b"user-owned");
    }

    #[cfg(unix)]
    #[test]
    fn relink_rolls_back_when_later_link_creation_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        let old_install = pkgs_dir.join("tool/v0.9.0");
        fs::create_dir_all(&old_install).unwrap();
        let old_tool = old_install.join("tool");
        let old_helper = old_install.join("helper");
        fs::write(&old_tool, b"old tool").unwrap();
        fs::write(&old_helper, b"old helper").unwrap();
        let tool_link = bin_dir.join("tool");
        let helper_link = bin_dir.join("helper");
        std::os::unix::fs::symlink(&old_tool, &tool_link).unwrap();
        std::os::unix::fs::symlink(&old_helper, &helper_link).unwrap();

        let new_install = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&new_install).unwrap();
        fs::write(new_install.join("tool"), b"new tool").unwrap();
        fs::write(new_install.join("helper"), b"new helper").unwrap();
        let receipt = PkgReceipt {
            name: "tool".to_string(),
            version: "v1.0.0".to_string(),
            remote: RemoteType::default(),
            installed_at: Utc::now(),
            install_subdir: PathBuf::from("tool/v1.0.0"),
            binaries: vec![
                InstalledBin { name: "tool".to_string(), bin_subpath: PathBuf::from("tool") },
                InstalledBin { name: "helper".to_string(), bin_subpath: PathBuf::from("helper") },
            ],
        };

        let mut calls = 0usize;
        let error = receipt
            .relink_with(&bin_dir, &pkgs_dir, |original, link, owned_root| {
                calls += 1;
                if calls == 2 {
                    anyhow::bail!("simulated link failure");
                }
                create_symlink(original, link, owned_root)
            })
            .unwrap_err();

        assert!(error.to_string().contains("simulated link failure"));
        assert_eq!(fs::read_link(tool_link).unwrap(), old_tool);
        assert_eq!(fs::read_link(helper_link).unwrap(), old_helper);
    }

    #[cfg(unix)]
    #[test]
    fn relink_refuses_symlink_pointing_outside_pkgs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir_all(&pkgs_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();

        // A symlink at the destination, but pointing outside pkgs_dir
        // (e.g. user's own custom symlink).
        let foreign_target = elsewhere.join("tool");
        fs::write(&foreign_target, b"\x7fELF custom").unwrap();
        let link_path = bin_dir.join("tool");
        std::os::unix::fs::symlink(&foreign_target, &link_path).unwrap();

        let install_dir = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(install_dir.join("tool"), b"\x7fELF").unwrap();

        let receipt = make_receipt("tool/v1.0.0", "tool");
        let err = receipt.relink(&bin_dir, &pkgs_dir).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Refusing to replace"), "unexpected error: {msg}");
        // Original symlink must still be there.
        assert!(link_path.is_symlink());
        assert_eq!(std::fs::read_link(&link_path).unwrap(), foreign_target);
    }

    #[cfg(unix)]
    #[test]
    fn unlink_removes_inro_managed_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&pkgs_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();

        let install_dir = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&install_dir).unwrap();
        let bin_path = install_dir.join("tool");
        fs::write(&bin_path, b"\x7fELF").unwrap();

        let link_path = bin_dir.join("tool");
        std::os::unix::fs::symlink(&bin_path, &link_path).unwrap();

        let receipt = make_receipt("tool/v1.0.0", "tool");
        receipt.unlink(&bin_dir, &pkgs_dir).unwrap();

        assert!(!link_path.exists() && !link_path.is_symlink(), "managed symlink should be gone");
    }

    #[cfg(unix)]
    #[test]
    fn unlink_keeps_link_owned_by_another_version() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        let old_install = pkgs_dir.join("tool/v0.9.0");
        fs::create_dir_all(&old_install).unwrap();
        fs::write(old_install.join("tool"), b"old").unwrap();

        let current_install = pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&current_install).unwrap();
        let current_target = current_install.join("tool");
        fs::write(&current_target, b"current").unwrap();
        let link_path = bin_dir.join("tool");
        std::os::unix::fs::symlink(&current_target, &link_path).unwrap();

        let old_receipt = make_receipt("tool/v0.9.0", "tool");
        old_receipt.unlink(&bin_dir, &pkgs_dir).unwrap();

        assert_eq!(fs::read_link(link_path).unwrap(), current_target);
    }

    #[cfg(unix)]
    #[test]
    fn unlink_skips_foreign_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&pkgs_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();

        // User replaced inro's symlink with their own binary at the same path.
        let link_path = bin_dir.join("tool");
        fs::write(&link_path, b"user's own binary").unwrap();

        let receipt = make_receipt("tool/v1.0.0", "tool");
        // Must succeed (not fail), but must NOT delete the foreign file.
        receipt.unlink(&bin_dir, &pkgs_dir).unwrap();

        assert_eq!(fs::read(&link_path).unwrap(), b"user's own binary");
    }

    #[cfg(unix)]
    #[test]
    fn unlink_skips_symlink_pointing_outside_pkgs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let bin_dir = tmp.path().join("bin");
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir_all(&pkgs_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();

        // The link in bin_dir now points outside pkgs_dir (e.g. user pointed
        // it at a custom build).
        let foreign_target = elsewhere.join("tool");
        fs::write(&foreign_target, b"custom").unwrap();
        let link_path = bin_dir.join("tool");
        std::os::unix::fs::symlink(&foreign_target, &link_path).unwrap();

        let receipt = make_receipt("tool/v1.0.0", "tool");
        receipt.unlink(&bin_dir, &pkgs_dir).unwrap();

        // Foreign symlink must remain intact.
        assert!(link_path.is_symlink());
        assert_eq!(fs::read_link(&link_path).unwrap(), foreign_target);
    }
}
