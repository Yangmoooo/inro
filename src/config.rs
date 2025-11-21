use std::path::{Path, PathBuf};

use anyhow::Result;
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

const INRO_REMOTE_SOURCES: &str = "https://github.com/Yangmoooo/inro-sources.git";

// config about package at package.rs, not here

#[derive(Debug, Deserialize, Serialize)]
pub struct UserConfig {
    // Install
    pub bin_dir: PathBuf,

    // Sources
    pub remotes: Vec<RemoteSources>,

    // Network
    pub github_token: Option<String>,
    pub proxy: Option<String>,
    pub timeout_secs: u64,

    // Behavior
    pub parallel_downloads: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RemoteSources {
    pub name: String,
    pub url: String,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            // Install defaults
            bin_dir: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/bin"),

            // Sources defaults
            remotes: vec![RemoteSources {
                name: "default".to_string(),
                url: INRO_REMOTE_SOURCES.to_string(),
            }],

            // Network defaults
            github_token: None,
            proxy: None,
            timeout_secs: 30,

            // Behavior defaults
            parallel_downloads: 4,
        }
    }
}

impl UserConfig {
    pub fn load(config_path: &Path) -> Result<Self> {
        let mut figment = Figment::new()
            // layer 1: hard-coded defaults
            .merge(Serialized::defaults(UserConfig::default()))
            // layer 2: user-defined config file
            .merge(Toml::file(config_path));

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

        let mut config: UserConfig = figment.extract()?;
        config.expand_paths();

        Ok(config)
    }

    fn expand_paths(&mut self) {
        self.bin_dir = expand_path(&self.bin_dir);
    }
}

fn expand_path(path: &Path) -> PathBuf {
    if !path.starts_with("~") {
        return path.to_path_buf();
    }
    let path_str = path.to_string_lossy();
    let home = dirs::home_dir().expect("Could not determine home directory");

    // ~
    if path_str == "~" {
        return home;
    }
    // ~/foo or ~\foo
    if path_str.starts_with("~/") || path_str.starts_with("~\\") {
        return home.join(&path_str[2..]);
    }

    path.to_path_buf()
}
