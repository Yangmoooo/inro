use std::collections::{HashMap, HashSet};
use std::fs::read_dir;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use figment::Figment;
use figment::providers::{Format, Toml};
use serde::Deserialize;

use crate::config::Config;
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
