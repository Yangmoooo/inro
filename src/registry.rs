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
use crate::debug;
use crate::layout::InroLayout;
use crate::package::PkgDef;
use crate::remotes::AssetSelector;

/// Information needed for writing an asset selection back to a local registry.
pub struct AssetSelectionWriteBack {
    pub pkg_name: String,
    pub platform_key: String,
    pub selector: AssetSelector,
}

#[derive(Debug, Default, Deserialize)]
pub struct Registry {
    #[serde(flatten)]
    pub pkgs: HashMap<String, PkgDef>,
}

impl Registry {
    pub fn load(layout: &InroLayout) -> Result<Self> {
        let config = Config::load(layout)?;
        let mut figment = Figment::new();

        // Managed registry: upstream-cached sources (priority-name.toml) filtered by
        // config.upstreams.enabled, then auto.toml (always loaded if present).
        let managed_registry_dir = &layout.managed_registry_dir;
        let enabled_names: HashSet<String> = config
            .upstreams
            .iter()
            .filter(|u| u.enabled)
            .map(|u| format!("{:02}-{}.toml", u.priority, u.name))
            .collect();
        let mut upstream_files = collect_toml_files(managed_registry_dir)?;
        upstream_files.retain(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| enabled_names.contains(n))
                .unwrap_or(false)
        });
        for file_path in upstream_files {
            debug!("Loading managed registry: {}", file_path.display());
            figment = figment.merge(Toml::file(file_path));
        }
        let auto_path = managed_registry_dir.join("auto.toml");
        if auto_path.exists() {
            debug!("Loading automatic registry overrides: {}", auto_path.display());
            figment = figment.merge(Toml::file(auto_path));
        }

        // User registry: hand-written overrides (highest precedence).
        let user_registry_dir = &layout.user_registry_dir;
        let files = collect_toml_files(user_registry_dir)?;
        for file_path in files {
            debug!("Loading user registry override: {}", file_path.display());
            figment = figment.merge(Toml::file(file_path));
        }

        let registry: Registry = figment.extract()?;
        if registry.pkgs.is_empty() {
            return Err(anyhow!("No package definitions found in registry"));
        }

        Ok(registry)
    }

    /// Write auto-detected asset selections to the managed registry.
    ///
    /// Creates or updates `auto.toml` in `managed_registry_dir`, setting
    /// `[pkg_name.remote.github.asset].<platform_key> = selector` for each
    /// entry. User-written files under `user_registry_dir` keep precedence.
    pub fn write_asset_selections(
        layout: &InroLayout,
        selections: &[AssetSelectionWriteBack],
    ) -> Result<()> {
        if selections.is_empty() {
            return Ok(());
        }

        let file_path = layout.managed_registry_dir.join("auto.toml");
        let content =
            if file_path.exists() { fs::read_to_string(&file_path)? } else { String::new() };
        let mut doc: DocumentMut =
            content.parse().map_err(|e| anyhow!("Failed to parse {}: {e}", file_path.display()))?;

        for sel in selections {
            let pkg_table = get_or_create_table(doc.as_table_mut(), &sel.pkg_name, true)?;
            let remote_table = get_or_create_table(pkg_table, "remote", true)?;
            let github_table = get_or_create_table(remote_table, "github", true)?;
            let asset_table = get_or_create_table(github_table, "asset", false)?;

            let value = match &sel.selector {
                AssetSelector::Glob(pattern) => toml_edit::value(pattern),
                AssetSelector::Tokens(tokens) => {
                    let mut array = toml_edit::Array::new();
                    for token in tokens {
                        array.push(token.as_str());
                    }
                    toml_edit::Item::Value(toml_edit::Value::Array(array))
                }
            };
            asset_table.insert(&sel.platform_key, value);
        }

        fs::create_dir_all(&layout.managed_registry_dir)?;
        let temp_path = file_path.with_extension("tmp");
        fs::write(&temp_path, doc.to_string())?;
        // Atomic replace: `fs::rename` overwrites an existing file on Linux,
        // macOS, and modern Windows, so the registry never appears to be
        // missing or partially written from another process's view.
        fs::rename(&temp_path, &file_path)?;

        Ok(())
    }
}

fn get_or_create_table<'a>(
    parent: &'a mut Table,
    key: &str,
    implicit: bool,
) -> Result<&'a mut Table> {
    let item = parent.entry(key).or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(implicit);
        Item::Table(table)
    });
    item.as_table_mut().ok_or_else(|| anyhow!("'{key}' is not a table"))
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
    use crate::layout::InroLayout;
    use crate::platform::PlatformInfo;
    use crate::remotes::RemoteType;

    fn test_layout(root: &Path) -> InroLayout {
        let inro_dir = root.join("inro");
        InroLayout {
            home_dir: root.join("home"),
            config_path: inro_dir.join("config.toml"),
            manifest_path: inro_dir.join("manifest.json"),
            pkgs_dir: inro_dir.join("pkgs"),
            managed_registry_dir: inro_dir.join("registry"),
            user_registry_dir: inro_dir.join("registry.d"),
            inro_dir,
        }
    }

    #[test]
    fn direct_versions_parse_and_merge_across_registry_layers() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        fs::create_dir_all(&layout.managed_registry_dir).unwrap();
        fs::create_dir_all(&layout.user_registry_dir).unwrap();

        let platform_key = PlatformInfo::current().key();
        fs::write(
            layout.managed_registry_dir.join("00-default.toml"),
            format!(
                r#"
[sqlite]
[sqlite.remote.direct."3.52.0"]
"{platform_key}" = "https://example.com/sqlite-352.zip"
[[sqlite.bin]]
name = "sqlite3"
"#
            ),
        )
        .unwrap();
        fs::write(
            layout.user_registry_dir.join("sqlite.toml"),
            format!(
                r#"
[sqlite.remote.direct."3.53.4"]
"{platform_key}" = "https://example.com/sqlite-353.zip"
"#
            ),
        )
        .unwrap();

        let registry = Registry::load(&layout).unwrap();
        let pkg = registry.pkgs.get("sqlite").unwrap();
        let RemoteType::Direct(direct) = &pkg.remote else {
            panic!("expected direct remote");
        };

        assert_eq!(direct.versions.len(), 2);
        assert_eq!(direct.versions["3.52.0"][&platform_key], "https://example.com/sqlite-352.zip");
        assert_eq!(direct.versions["3.53.4"][&platform_key], "https://example.com/sqlite-353.zip");
        assert_eq!(pkg.bin.len(), 1);
    }

    #[test]
    fn write_asset_selection_merges_with_upstream_package_definition() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        fs::create_dir_all(&layout.managed_registry_dir).unwrap();

        fs::write(
            layout.managed_registry_dir.join("00-default.toml"),
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
            &[AssetSelectionWriteBack {
                pkg_name: "tool".to_string(),
                platform_key: platform_key.clone(),
                selector: AssetSelector::Glob("*linux-x86_64.tar.gz".to_string()),
            }],
        )
        .unwrap();

        let registry = Registry::load(&layout).unwrap();
        let pkg = registry.pkgs.get("tool").unwrap();

        let RemoteType::GitHub(github) = &pkg.remote else {
            panic!("expected GitHub remote");
        };
        assert_eq!(github.repo, "owner/tool");
        assert_eq!(
            github.asset.get(&platform_key),
            Some(&AssetSelector::Glob("*linux-x86_64.tar.gz".to_string()))
        );
        assert_eq!(pkg.bin.len(), 1);
        let resolved = pkg.resolve("tool");
        #[cfg(not(windows))]
        assert_eq!(resolved.bin[0].link, "tool");
        #[cfg(windows)]
        assert_eq!(resolved.bin[0].link, "tool.exe");
    }

    #[test]
    fn write_asset_selection_uses_compact_dotted_table() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        let platform_key = PlatformInfo::current().key();

        Registry::write_asset_selections(
            &layout,
            &[AssetSelectionWriteBack {
                pkg_name: "codex".to_string(),
                platform_key: platform_key.clone(),
                selector: AssetSelector::Glob("codex-*-aarch64-apple-darwin.tar.gz".to_string()),
            }],
        )
        .unwrap();

        let auto_toml = fs::read_to_string(layout.managed_registry_dir.join("auto.toml")).unwrap();

        assert!(auto_toml.contains("[codex.remote.github.asset]"));
        assert!(
            auto_toml
                .contains(&format!(r#"{platform_key} = "codex-*-aarch64-apple-darwin.tar.gz""#))
        );
        assert!(!auto_toml.contains("[codex]\n\n"));
        assert!(!auto_toml.contains("[codex.remote]\n\n"));
        assert!(!auto_toml.contains("[codex.remote.github]\n\n"));
    }

    #[test]
    fn write_asset_selection_updates_existing_local_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        let platform_key = PlatformInfo::current().key();

        Registry::write_asset_selections(
            &layout,
            &[AssetSelectionWriteBack {
                pkg_name: "codex".to_string(),
                platform_key: platform_key.clone(),
                selector: AssetSelector::Glob("old.tar.gz".to_string()),
            }],
        )
        .unwrap();
        Registry::write_asset_selections(
            &layout,
            &[AssetSelectionWriteBack {
                pkg_name: "codex".to_string(),
                platform_key: platform_key.clone(),
                selector: AssetSelector::Glob("new.tar.gz".to_string()),
            }],
        )
        .unwrap();

        let auto_toml = fs::read_to_string(layout.managed_registry_dir.join("auto.toml")).unwrap();
        assert!(auto_toml.contains(&format!(r#"{platform_key} = "new.tar.gz""#)));
        assert!(!auto_toml.contains("old.tar.gz"));
    }

    #[test]
    fn write_asset_selection_leaves_no_temp_file_after_success() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        let platform_key = PlatformInfo::current().key();

        // Write twice so the second invocation exercises the rename-over-existing
        // path that previously deleted the destination before renaming.
        for selector in ["first.tar.gz", "second.tar.gz"] {
            Registry::write_asset_selections(
                &layout,
                &[AssetSelectionWriteBack {
                    pkg_name: "codex".to_string(),
                    platform_key: platform_key.clone(),
                    selector: AssetSelector::Glob(selector.to_string()),
                }],
            )
            .unwrap();

            let auto_toml_exists = layout.managed_registry_dir.join("auto.toml").exists();
            let temp_left_behind = layout.managed_registry_dir.join("auto.tmp").exists();
            assert!(auto_toml_exists, "auto.toml should always exist after a successful write");
            assert!(!temp_left_behind, "auto.tmp must be renamed in one step, not left behind");
        }
    }

    #[test]
    fn write_asset_selection_parse_error_includes_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        fs::create_dir_all(&layout.managed_registry_dir).unwrap();
        let auto_path = layout.managed_registry_dir.join("auto.toml");
        fs::write(&auto_path, "not valid toml =").unwrap();

        let error = Registry::write_asset_selections(
            &layout,
            &[AssetSelectionWriteBack {
                pkg_name: "codex".to_string(),
                platform_key: PlatformInfo::current().key(),
                selector: AssetSelector::Glob("codex.tar.gz".to_string()),
            }],
        )
        .unwrap_err();

        assert!(error.to_string().contains(&auto_path.display().to_string()));
    }

    #[test]
    fn load_picks_up_auto_toml_alongside_upstream_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        fs::create_dir_all(&layout.managed_registry_dir).unwrap();

        fs::write(
            layout.managed_registry_dir.join("00-default.toml"),
            r#"
[tool]
[tool.remote.github]
repo = "owner/tool"
[[tool.bin]]
name = "tool"
"#,
        )
        .unwrap();

        let platform_key = PlatformInfo::current().key();
        fs::write(
            layout.managed_registry_dir.join("auto.toml"),
            format!("[tool.remote.github.asset]\n{platform_key} = \"tool-from-auto.tar.gz\"\n"),
        )
        .unwrap();

        let registry = Registry::load(&layout).unwrap();
        let pkg = registry.pkgs.get("tool").unwrap();
        let RemoteType::GitHub(github) = &pkg.remote else {
            panic!("expected GitHub remote");
        };
        assert_eq!(
            github.asset.get(&platform_key),
            Some(&AssetSelector::Glob("tool-from-auto.tar.gz".to_string()))
        );
    }

    #[test]
    fn user_registry_overrides_auto_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        let layout = test_layout(temp_dir.path());
        fs::create_dir_all(&layout.managed_registry_dir).unwrap();
        fs::create_dir_all(&layout.user_registry_dir).unwrap();

        fs::write(
            layout.managed_registry_dir.join("00-default.toml"),
            r#"
[tool]
[tool.remote.github]
repo = "owner/tool"
[[tool.bin]]
name = "tool"
"#,
        )
        .unwrap();

        let platform_key = PlatformInfo::current().key();
        fs::write(
            layout.managed_registry_dir.join("auto.toml"),
            format!("[tool.remote.github.asset]\n{platform_key} = \"from-auto.tar.gz\"\n"),
        )
        .unwrap();
        fs::write(
            layout.user_registry_dir.join("override.toml"),
            format!("[tool.remote.github.asset]\n{platform_key} = \"from-user.tar.gz\"\n"),
        )
        .unwrap();

        let registry = Registry::load(&layout).unwrap();
        let pkg = registry.pkgs.get("tool").unwrap();
        let RemoteType::GitHub(github) = &pkg.remote else {
            panic!("expected GitHub remote");
        };
        assert_eq!(
            github.asset.get(&platform_key),
            Some(&AssetSelector::Glob("from-user.tar.gz".to_string()))
        );
    }
}
