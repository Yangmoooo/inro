use std::collections::HashMap;
use std::fs::read_dir;

use anyhow::{Result, anyhow};
use figment::Figment;
use figment::providers::{Format, Toml};
use serde::Deserialize;

use crate::layout::InroLayout;
use crate::package::PkgDef;

#[derive(Debug, Default, Deserialize)]
pub struct Registry {
    #[serde(flatten)]
    pub pkgs: HashMap<String, PkgDef>,
}

impl Registry {
    pub fn load(layout: &InroLayout) -> Result<Self> {
        let mut figment = Figment::new();

        // load upstream registry
        let upstream_registry_dir = &layout.upstream_registry_dir;
        if upstream_registry_dir.exists() {
            let mut files = vec![];
            for entry in read_dir(upstream_registry_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                    files.push(path);
                }
            }

            files.sort();
            for file_path in files {
                figment = figment.merge(Toml::file(file_path));
            }
        }

        // load local registry
        let local_registry_dir = &layout.local_registry_dir;
        if local_registry_dir.exists() {
            let mut files = vec![];
            for entry in read_dir(local_registry_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                    files.push(path);
                }
            }

            files.sort();
            for file_path in files {
                figment = figment.merge(Toml::file(file_path));
            }
        }

        let registry: Registry = figment.extract()?;
        if registry.pkgs.is_empty() {
            return Err(anyhow!("No package definitions found in registry"));
        }

        Ok(registry)
    }
}
