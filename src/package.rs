use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::remotes::RemoteType;
use crate::utils::create_symlink;

#[derive(Clone, Debug, Deserialize)]
pub struct PkgDef {
    #[serde(default)]
    pub ver: Option<String>,
    pub remote: RemoteType,
    #[serde(default)]
    pub bin: Vec<BinDef>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BinDef {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug)]
pub struct ResolvedPkg {
    #[allow(dead_code)]
    pub ver: Option<String>,
    pub remote: RemoteType,
    pub bin: Vec<ResolvedBin>,
}

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
                .map(|b| {
                    let raw_name = b.name.unwrap_or_else(|| pkg_name.to_string());
                    let name = normalize_name(raw_name);
                    let raw_link = b.link.unwrap_or_else(|| name.clone());
                    let link = normalize_name(raw_link);
                    ResolvedBin { name, link }
                })
                .collect()
        };

        ResolvedPkg { ver: self.ver, remote: self.remote, bin }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PkgState {
    // none means installed but not linked
    pub current_version: Option<String>,

    // key: version
    pub versions: HashMap<String, PkgReceipt>,
}

impl PkgState {
    /// Get the latest **installed** version from PkgState
    pub fn get_latest_version(&self) -> Option<String> {
        self.versions
            .iter()
            .max_by_key(|(_ver, receipt)| receipt.installed_at)
            .map(|(ver, _receipt)| ver.clone())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PkgReceipt {
    /// Package name, e.g. 'ripgrep'
    pub name: String,

    /// Package version, actually the tag name
    pub version: String,

    /// Remote info
    pub remote: RemoteType,

    /// Installation time
    pub installed_at: DateTime<Utc>,

    /// Installation directory, actually where the binary is extracted to
    pub install_dir: PathBuf,

    /// Binaries installed details
    pub binaries: Vec<InstalledBin>,
}

impl PkgReceipt {
    pub fn save_to_install_dir(&self) -> Result<()> {
        let receipt_path = self.install_dir.join("inro-receipt.json");
        let receipt_file = File::create(&receipt_path).with_context(|| {
            format!("Failed to create receipt backup: {}", receipt_path.display())
        })?;
        serde_json::to_writer_pretty(receipt_file, self)?;
        Ok(())
    }

    pub fn relink(&mut self, target_dir: &Path) -> Result<()> {
        if !target_dir.exists() {
            let _ = fs::create_dir_all(target_dir)
                .with_context(|| format!("Failed to create bin dir: {}", target_dir.display()));
        }

        for bin in &mut self.binaries {
            // clean up
            // if the old entry is still at there and its parent dir is not the target_dir
            // thats say the config bin_dir is changed, remove the old link
            if let Some(parent) = bin.link_path.parent()
                && parent != target_dir
                && bin.link_path.exists()
            {
                let _ = fs::remove_file(&bin.link_path);
            }

            // create new and update
            let target = target_dir.join(&bin.name);
            create_symlink(&bin.bin_path, &target)?;
            bin.link_path = target;
        }
        Ok(())
    }

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

    #[error("Failed to fetch from the upstream: '{0}'")]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remotes::GitHubAssetDef;

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
        let pkg_def = make_pkg_def(vec![BinDef { name: Some("rg".to_string()), link: None }]);
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
            name: Some("rg".to_string()),
            link: Some("ripgrep".to_string()),
        }]);
        let resolved = pkg_def.resolve("ripgrep");

        #[cfg(not(windows))]
        {
            assert_eq!(resolved.bin[0].name, "rg");
            assert_eq!(resolved.bin[0].link, "ripgrep");
        }
    }

    #[test]
    fn resolve_multiple_binaries() {
        let pkg_def = make_pkg_def(vec![
            BinDef { name: Some("uv".to_string()), link: None },
            BinDef { name: Some("uvx".to_string()), link: None },
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
}
