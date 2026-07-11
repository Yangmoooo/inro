use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::package::{PkgReceipt, PkgState};

const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    #[serde(default, rename = "packages")]
    pub pkgs: HashMap<String, PkgState>,
}

fn default_schema_version() -> u32 { CURRENT_SCHEMA_VERSION }

impl Default for Manifest {
    fn default() -> Self { Self { schema_version: default_schema_version(), pkgs: HashMap::new() } }
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let file = File::open(path)
            .with_context(|| format!("Failed to open manifest file: {}", path.display()))?;
        let reader = BufReader::new(file);
        let raw: serde_json::Value = serde_json::from_reader(reader)
            .with_context(|| format!("Failed to parse manifest JSON: {}", path.display()))?;
        let schema = raw.get("schema_version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        if schema != CURRENT_SCHEMA_VERSION {
            bail!(
                "Unsupported manifest schema_version {schema} at {}. inro 0.7 introduced a \
                 breaking layout change — see CHANGELOG for the cleanup-and-reinstall path.",
                path.display()
            );
        }
        let manifest: Manifest = serde_json::from_value(raw)
            .with_context(|| format!("Failed to deserialize manifest: {}", path.display()))?;
        Ok(manifest)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = path.with_extension("tmp");
        let file = File::create(&temp_path)?;
        serde_json::to_writer_pretty(file, self)?;
        fs::rename(&temp_path, path)
            .with_context(|| format!("Failed to save manifest to {}", path.display()))?;
        Ok(())
    }

    pub fn add(&mut self, receipt: PkgReceipt) {
        let pkg_name = receipt.name.clone();
        let version = receipt.version.clone();

        let state = self.pkgs.entry(pkg_name).or_default();
        state.versions.insert(version.clone(), receipt);
        state.current_version = Some(version);
    }

    /// Remove a version.
    pub fn remove_version(&mut self, name: &str, version: &str) -> Option<PkgReceipt> {
        let state = self.pkgs.get_mut(name)?;
        if state.current_version.as_deref() == Some(version) {
            state.current_version = None;
        }
        let receipt = state.versions.remove(version);
        // after removing, if there are other versions, need to use manually
        if state.versions.is_empty() {
            self.pkgs.remove(name);
        }
        receipt
    }

    /// Unlink the current version.
    pub fn unlink_package(&mut self, name: &str) -> Option<PkgReceipt> {
        let state = self.pkgs.get_mut(name)?;
        let current_ver = state.current_version.take()?;
        state.versions.get(&current_ver).cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;

    use super::*;
    use crate::package::InstalledBin;
    use crate::remotes::{GitHubAssetDef, RemoteType};

    fn make_receipt(name: &str, version: &str) -> PkgReceipt {
        PkgReceipt {
            name: name.to_string(),
            version: version.to_string(),
            remote: RemoteType::GitHub(GitHubAssetDef {
                repo: format!("test/{name}"),
                asset: HashMap::new(),
            }),
            installed_at: Utc::now(),
            install_subdir: PathBuf::from(name).join(version),
            binaries: vec![InstalledBin {
                name: name.to_string(),
                bin_subpath: PathBuf::from(name),
            }],
        }
    }

    #[test]
    fn manifest_default_values() {
        let manifest = Manifest::default();
        assert_eq!(manifest.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(manifest.pkgs.is_empty());
    }

    #[test]
    fn manifest_add_single_package() {
        let mut manifest = Manifest::default();
        let receipt = make_receipt("ripgrep", "15.0.0");

        manifest.add(receipt);

        assert!(manifest.pkgs.contains_key("ripgrep"));
        let state = &manifest.pkgs["ripgrep"];
        assert_eq!(state.current_version, Some("15.0.0".to_string()));
        assert!(state.versions.contains_key("15.0.0"));
    }

    #[test]
    fn manifest_add_multiple_versions() {
        let mut manifest = Manifest::default();

        manifest.add(make_receipt("fd", "9.0.0"));
        manifest.add(make_receipt("fd", "10.0.0"));

        let state = &manifest.pkgs["fd"];
        assert_eq!(state.current_version, Some("10.0.0".to_string()));
        assert_eq!(state.versions.len(), 2);
        assert!(state.versions.contains_key("9.0.0"));
        assert!(state.versions.contains_key("10.0.0"));
    }

    #[test]
    fn manifest_remove_version_clears_current() {
        let mut manifest = Manifest::default();
        manifest.add(make_receipt("bat", "0.24.0"));
        manifest.add(make_receipt("bat", "0.25.0"));

        // Remove current version
        let removed = manifest.remove_version("bat", "0.25.0");

        assert!(removed.is_some());
        let state = &manifest.pkgs["bat"];
        assert_eq!(state.current_version, None);
        assert!(!state.versions.contains_key("0.25.0"));
        assert!(state.versions.contains_key("0.24.0"));
    }

    #[test]
    fn manifest_remove_last_version_removes_package() {
        let mut manifest = Manifest::default();
        manifest.add(make_receipt("tokei", "12.0.0"));

        manifest.remove_version("tokei", "12.0.0");

        assert!(!manifest.pkgs.contains_key("tokei"));
    }

    #[test]
    fn manifest_unlink_package() {
        let mut manifest = Manifest::default();
        manifest.add(make_receipt("just", "1.0.0"));

        let receipt = manifest.unlink_package("just");

        assert!(receipt.is_some());
        let state = &manifest.pkgs["just"];
        assert_eq!(state.current_version, None);
        // Package still exists with version
        assert!(state.versions.contains_key("1.0.0"));
    }

    #[test]
    fn manifest_json_roundtrip() {
        let mut manifest = Manifest::default();
        manifest.add(make_receipt("rg", "15.0.0"));
        manifest.add(make_receipt("fd", "10.0.0"));

        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.schema_version, manifest.schema_version);
        assert_eq!(parsed.pkgs.len(), manifest.pkgs.len());
        assert!(parsed.pkgs.contains_key("rg"));
        assert!(parsed.pkgs.contains_key("fd"));
    }

    #[test]
    fn load_default_when_path_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Manifest::load(&tmp.path().join("absent.json")).unwrap();
        assert_eq!(m.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(m.pkgs.is_empty());
    }

    #[test]
    fn load_rejects_v1_schema_with_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("old.json");
        fs::write(&path, r#"{"schema_version":1,"packages":{}}"#).unwrap();
        let err = Manifest::load(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Unsupported manifest schema_version 1"), "msg: {msg}");
        assert!(msg.contains("CHANGELOG"), "msg: {msg}");
    }

    #[test]
    fn load_accepts_v2_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("new.json");
        fs::write(&path, r#"{"schema_version":2,"packages":{}}"#).unwrap();
        let m = Manifest::load(&path).unwrap();
        assert_eq!(m.schema_version, 2);
    }
}
