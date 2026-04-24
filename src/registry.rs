use std::collections::{HashMap, HashSet};
use std::fs::{self, read_dir};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use figment::Figment;
use figment::providers::{Format, Toml};
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table};

use crate::config::Config;
use crate::installer::WriteBackInfo;
use crate::layout::InroLayout;
use crate::package::PkgDef;

#[derive(Debug, Default, Deserialize)]
pub struct Registry {
    #[serde(flatten)]
    pub pkgs: HashMap<String, PkgDef>,
}

impl Registry {
    pub fn load(layout: &InroLayout) -> Result<Self> {
        let config = Config::load(layout)?;
        let mut figment = Figment::new();

        // load upstream registry - sorted by priority, only enabled sources
        let upstream_registry_dir = &layout.upstream_registry_dir;
        let enabled_names: HashSet<String> = config
            .upstreams
            .iter()
            .filter(|u| u.enabled)
            .map(|u| format!("{:02}-{}.toml", u.priority, u.name))
            .collect();
        let mut upstream_files = collect_toml_files(upstream_registry_dir)?;
        upstream_files.retain(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| enabled_names.contains(n))
                .unwrap_or(false)
        });
        for file_path in upstream_files {
            figment = figment.merge(Toml::file(file_path));
        }

        // load local registry
        let local_registry_dir = &layout.local_registry_dir;
        let files = collect_toml_files(local_registry_dir)?;
        for file_path in files {
            figment = figment.merge(Toml::file(file_path));
        }

        let registry: Registry = figment.extract()?;
        if registry.pkgs.is_empty() {
            return Err(anyhow!("No package definitions found in registry"));
        }

        Ok(registry)
    }

    /// Write asset selections back to the local registry file.
    ///
    /// Creates or updates `local.toml` in `local_registry_dir`, setting
    /// `[pkg_name.remote.github.asset].<platform_key> = keyword` for each
    /// entry.
    pub fn write_asset_selections(layout: &InroLayout, selections: &[WriteBackInfo]) -> Result<()> {
        if selections.is_empty() {
            return Ok(());
        }

        let file_path = layout.local_registry_dir.join("local.toml");
        let content =
            if file_path.exists() { fs::read_to_string(&file_path)? } else { String::new() };
        let mut doc: DocumentMut =
            content.parse().map_err(|e| anyhow!("Failed to parse local.toml: {e}"))?;

        for sel in selections {
            // Navigate/create: [pkg_name] -> [remote] -> [github] -> [asset]
            let pkg_table = doc.entry(&sel.pkg_name).or_insert_with(|| Item::Table(Table::new()));
            let pkg_table = pkg_table
                .as_table_mut()
                .ok_or_else(|| anyhow!("'{0}' is not a table", sel.pkg_name))?;

            let remote_table =
                pkg_table.entry("remote").or_insert_with(|| Item::Table(Table::new()));
            let remote_table =
                remote_table.as_table_mut().ok_or_else(|| anyhow!("'remote' is not a table"))?;

            let github_table =
                remote_table.entry("github").or_insert_with(|| Item::Table(Table::new()));
            let github_table =
                github_table.as_table_mut().ok_or_else(|| anyhow!("'github' is not a table"))?;

            let asset_table =
                github_table.entry("asset").or_insert_with(|| Item::Table(Table::new()));
            let asset_table =
                asset_table.as_table_mut().ok_or_else(|| anyhow!("'asset' is not a table"))?;

            asset_table.insert(&sel.platform_key, toml_edit::value(&sel.keyword));
        }

        fs::create_dir_all(&layout.local_registry_dir)?;
        let temp_path = file_path.with_extension("tmp");
        fs::write(&temp_path, doc.to_string())?;
        fs::rename(&temp_path, &file_path)?;

        Ok(())
    }
}

/// Collect all .toml files in the given directory, sorted by filename.
fn collect_toml_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut files = vec![];
    for entry in read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("toml") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::installer::WriteBackInfo;
    use crate::layout::InroLayout;
    use crate::platform::PlatformInfo;
    use crate::remotes::RemoteType;

    fn test_layout(root: &Path) -> InroLayout {
        InroLayout {
            home_dir: root.join("home"),
            config_path: root.join("config/inro/config.toml"),
            manifest_path: root.join("data/inro/inro-manifest.json"),
            pkgs_dir: root.join("data/inro/pkgs"),
            upstream_registry_dir: root.join("data/inro/sources.list.d"),
            local_registry_dir: root.join("config/inro/sources.list.d"),
        }
    }

    #[test]
    fn write_asset_selection_merges_with_upstream_package_definition() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        fs::create_dir_all(&layout.upstream_registry_dir).unwrap();

        fs::write(
            layout.upstream_registry_dir.join("00-default.toml"),
            r#"
[tool]
[tool.remote.github]
repo = "owner/tool"
[[tool.bin]]
name = "tool-bin"
link = "tool"
"#,
        )
        .unwrap();

        let platform_key = PlatformInfo::current().key();
        Registry::write_asset_selections(
            &layout,
            &[WriteBackInfo {
                pkg_name: "tool".to_string(),
                platform_key: platform_key.clone(),
                keyword: "linux-x86_64.tar.gz".to_string(),
            }],
        )
        .unwrap();

        let registry = Registry::load(&layout).unwrap();
        let pkg = registry.pkgs.get("tool").unwrap();

        let RemoteType::GitHub(github) = &pkg.remote;
        assert_eq!(github.repo, "owner/tool");
        assert_eq!(github.asset.get(&platform_key), Some(&"linux-x86_64.tar.gz".to_string()));
        assert_eq!(pkg.bin.len(), 1);
        let resolved = pkg.clone().resolve("tool");
        assert_eq!(resolved.bin[0].link, "tool");
    }
}
