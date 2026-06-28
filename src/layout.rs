use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub struct InroLayout {
    pub home_dir: PathBuf,      // user home (tilde expansion only)
    pub inro_dir: PathBuf,      // $INRO_HOME, default ~/.inro
    pub config_path: PathBuf,   // inro_dir/config.toml
    pub manifest_path: PathBuf, // inro_dir/manifest.json
    pub pkgs_dir: PathBuf,      // inro_dir/pkgs
    pub managed_registry_dir: PathBuf, /* inro_dir/registry  (inro-maintained: upstream cache +
                                 * auto.toml) */
    pub user_registry_dir: PathBuf, // inro_dir/registry.d (hand-written overrides)
}

impl InroLayout {
    pub fn new() -> Result<Self> {
        let home_dir =
            dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
        let inro_dir = resolve_inro_dir(&home_dir);

        let config_path = inro_dir.join("config.toml");
        let manifest_path = inro_dir.join("manifest.json");
        let pkgs_dir = inro_dir.join("pkgs");
        let managed_registry_dir = inro_dir.join("registry");
        let user_registry_dir = inro_dir.join("registry.d");

        Ok(Self {
            home_dir,
            inro_dir,
            config_path,
            manifest_path,
            pkgs_dir,
            managed_registry_dir,
            user_registry_dir,
        })
    }
}

fn resolve_inro_dir(home_dir: &Path) -> PathBuf {
    match std::env::var("INRO_HOME") {
        Ok(value) if !value.trim().is_empty() => expand_tilde(value.trim(), home_dir),
        _ => home_dir.join(".inro"),
    }
}

fn expand_tilde(raw: &str, home_dir: &Path) -> PathBuf {
    if raw == "~" {
        return home_dir.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return home_dir.join(rest);
    }
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex, MutexGuard};

    use super::*;

    // Tests in this module mutate the INRO_HOME env var. Serialize them.
    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct EnvGuard<'a> {
        _guard: MutexGuard<'a, ()>,
        previous: Option<String>,
    }

    impl EnvGuard<'_> {
        fn set(value: Option<&str>) -> Self {
            let guard = ENV_LOCK.lock().unwrap();
            let previous = std::env::var("INRO_HOME").ok();
            match value {
                // SAFETY: tests are serialized by ENV_LOCK above; no other thread reads
                // INRO_HOME while a guard is alive.
                Some(v) => unsafe { std::env::set_var("INRO_HOME", v) },
                None => unsafe { std::env::remove_var("INRO_HOME") },
            }
            Self { _guard: guard, previous }
        }
    }

    impl Drop for EnvGuard<'_> {
        fn drop(&mut self) {
            // SAFETY: see EnvGuard::set.
            match &self.previous {
                Some(v) => unsafe { std::env::set_var("INRO_HOME", v) },
                None => unsafe { std::env::remove_var("INRO_HOME") },
            }
        }
    }

    #[test]
    fn resolve_inro_dir_defaults_to_home_dot_inro() {
        let _env = EnvGuard::set(None);
        let home = Path::new("/home/user");
        assert_eq!(resolve_inro_dir(home), PathBuf::from("/home/user/.inro"));
    }

    #[test]
    fn resolve_inro_dir_uses_env_var_when_set() {
        let _env = EnvGuard::set(Some("/opt/inro"));
        let home = Path::new("/home/user");
        assert_eq!(resolve_inro_dir(home), PathBuf::from("/opt/inro"));
    }

    #[test]
    fn resolve_inro_dir_expands_tilde_in_env_var() {
        let _env = EnvGuard::set(Some("~/tools/inro"));
        let home = Path::new("/home/user");
        assert_eq!(resolve_inro_dir(home), PathBuf::from("/home/user/tools/inro"));
    }

    #[test]
    fn resolve_inro_dir_treats_empty_env_var_as_unset() {
        let _env = EnvGuard::set(Some("   "));
        let home = Path::new("/home/user");
        assert_eq!(resolve_inro_dir(home), PathBuf::from("/home/user/.inro"));
    }

    #[test]
    fn resolve_inro_dir_handles_bare_tilde() {
        let _env = EnvGuard::set(Some("~"));
        let home = Path::new("/home/user");
        assert_eq!(resolve_inro_dir(home), PathBuf::from("/home/user"));
    }
}
