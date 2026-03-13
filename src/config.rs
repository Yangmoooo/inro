use std::path::{Path, PathBuf};

use anyhow::Result;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

use crate::layout::InroLayout;

const INRO_DEFAULT_REGISTRY: &str =
    "https://raw.githubusercontent.com/Yangmoooo/inro-registry/main/default.toml";
const PARALLEL_DOWNLOADS_MIN: usize = 1;
const PARALLEL_DOWNLOADS_MAX: usize = 32;
const PARALLEL_DOWNLOADS_DEFAULT_MIN: usize = 4;
const PARALLEL_DOWNLOADS_DEFAULT_MAX: usize = 16;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    // install
    pub bin_dir: PathBuf,

    // sources
    pub upstreams: Vec<UpstreamDef>,

    // behavior
    pub parallel_downloads: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UpstreamDef {
    pub name: String,
    pub priority: u8,
    pub url: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool { true }

impl Default for Config {
    fn default() -> Self {
        Self {
            // install defaults
            bin_dir: dirs::home_dir()
                // handled in layout::InroLayout::new
                .expect("Layout violated contract: home directory could not be found")
                .join(".local")
                .join("bin"),

            // sources defaults
            upstreams: vec![UpstreamDef {
                name: "default".to_string(),
                priority: 0,
                url: INRO_DEFAULT_REGISTRY.to_string(),
                enabled: true,
            }],

            // behavior defaults
            parallel_downloads: Self::default_parallel_downloads(),
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

        // layer 3: inro standard env vars
        // such as:
        // INRO_BIN_DIR -> bin_dir
        // INRO_PARALLEL_DOWNLOADS -> parallel_downloads
        figment = figment.merge(Env::prefixed("INRO_"));

        let mut config: Config = figment.extract()?;
        config.expand_paths(&layout.home_dir);
        config.normalize();

        Ok(config)
    }

    fn expand_paths(&mut self, home: &Path) {
        self.bin_dir = Self::expand_path(&self.bin_dir, home);
    }

    fn normalize(&mut self) {
        self.parallel_downloads =
            self.parallel_downloads.clamp(PARALLEL_DOWNLOADS_MIN, PARALLEL_DOWNLOADS_MAX);
    }

    fn default_parallel_downloads() -> usize {
        let raw = std::thread::available_parallelism()
            .map(|n| n.get().saturating_mul(2))
            .unwrap_or(PARALLEL_DOWNLOADS_DEFAULT_MIN);
        raw.clamp(PARALLEL_DOWNLOADS_DEFAULT_MIN, PARALLEL_DOWNLOADS_DEFAULT_MAX)
    }

    fn expand_path(path: &Path, home: &Path) -> PathBuf {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_path_tilde_only() {
        let home = Path::new("/home/user");
        let result = Config::expand_path(Path::new("~"), home);
        assert_eq!(result, PathBuf::from("/home/user"));
    }

    #[test]
    fn expand_path_tilde_slash() {
        let home = Path::new("/home/user");
        let result = Config::expand_path(Path::new("~/bin"), home);
        assert_eq!(result, PathBuf::from("/home/user/bin"));
    }

    #[test]
    fn expand_path_tilde_nested() {
        let home = Path::new("/home/user");
        let result = Config::expand_path(Path::new("~/.local/bin"), home);
        assert_eq!(result, PathBuf::from("/home/user/.local/bin"));
    }

    #[test]
    fn expand_path_absolute_unchanged() {
        let home = Path::new("/home/user");
        let result = Config::expand_path(Path::new("/usr/local/bin"), home);
        assert_eq!(result, PathBuf::from("/usr/local/bin"));
    }

    #[test]
    fn expand_path_relative_unchanged() {
        let home = Path::new("/home/user");
        let result = Config::expand_path(Path::new("./bin"), home);
        assert_eq!(result, PathBuf::from("./bin"));
    }

    #[test]
    fn expand_path_tilde_not_at_start() {
        // Tilde not at start should not be expanded
        let home = Path::new("/home/user");
        let result = Config::expand_path(Path::new("/path/to/~"), home);
        assert_eq!(result, PathBuf::from("/path/to/~"));
    }

    #[test]
    fn normalize_parallel_downloads_enforces_lower_bound() {
        let mut config = Config { parallel_downloads: 0, ..Config::default() };
        config.normalize();
        assert_eq!(config.parallel_downloads, PARALLEL_DOWNLOADS_MIN);
    }

    #[test]
    fn normalize_parallel_downloads_enforces_upper_bound() {
        let mut config =
            Config { parallel_downloads: PARALLEL_DOWNLOADS_MAX + 100, ..Config::default() };
        config.normalize();
        assert_eq!(config.parallel_downloads, PARALLEL_DOWNLOADS_MAX);
    }

    #[test]
    fn default_parallel_downloads_within_expected_range() {
        let value = Config::default_parallel_downloads();
        assert!((PARALLEL_DOWNLOADS_DEFAULT_MIN..=PARALLEL_DOWNLOADS_DEFAULT_MAX).contains(&value));
    }
}
