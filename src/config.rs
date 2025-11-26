use std::path::{Path, PathBuf};

use anyhow::Result;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

use crate::layout::InroLayout;

const INRO_DEFAULT_REGISTRY: &str = "https://github.com/Yangmoooo/inro-sources.git";

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    // install
    pub bin_dir: PathBuf,

    // sources
    pub upstreams: Vec<UpstreamDef>,

    // network
    pub github_token: Option<String>,
    pub proxy: Option<String>,
    pub use_proxy: bool,
    pub timeout_secs: u64,

    // behavior
    pub parallel_downloads: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UpstreamDef {
    pub name: String,
    pub priority: u8,
    pub url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // install defaults
            bin_dir: dirs::home_dir().unwrap_or_default(),

            // sources defaults
            upstreams: vec![UpstreamDef {
                name: "default".to_string(),
                priority: 0,
                url: INRO_DEFAULT_REGISTRY.to_string(),
            }],

            // network defaults
            github_token: None,
            proxy: None,
            use_proxy: false,
            timeout_secs: 30,

            // behavior defaults
            parallel_downloads: 4,
        }
    }
}

impl Config {
    pub fn load(layout: &InroLayout) -> Result<Self> {
        // layer 1: hard-coded defaults
        let mut figment = Figment::new().merge(Serialized::defaults(Config::default()));

        // layer 2: user-defined config file
        if layout.config_path.exists() {
            figment = figment.merge(Toml::file(&layout.config_path));
        }

        // layer 3: universal env vars
        // HTTPS_PROXY | ALL_PROXY -> network.proxy
        if let Ok(proxy) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("ALL_PROXY")) {
            figment = figment.merge(("proxy", proxy));
        }
        // GITHUB_TOKEN -> network.github_token
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            figment = figment.merge(("github_token", token));
        }

        // layer 4: inro standard env vars
        // such as:
        // INRO_BIN_DIR -> bin_dir
        // INRO_TIMEOUT_SECS -> timeout_secs
        figment = figment.merge(Env::prefixed("INRO_"));

        let mut config: Config = figment.extract()?;
        config.expand_paths(&layout.home_dir);

        Ok(config)
    }

    fn expand_paths(&mut self, home: &Path) {
        self.bin_dir = expand_path(&self.bin_dir, home);
    }
}

fn expand_path(home: &Path, path: &Path) -> PathBuf {
    if !path.starts_with("~") {
        return path.to_path_buf();
    }
    let path_str = path.to_string_lossy();

    // ~
    if path_str == "~" {
        return home.to_path_buf();
    }
    // ~/foo or ~\foo
    if path_str.starts_with("~/") || path_str.starts_with("~\\") {
        return home.join(&path_str[2..]);
    }

    path.to_path_buf()
}
