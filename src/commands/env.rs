use std::path::Path;

use anyhow::Result;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;

pub struct EnvCommand {}

impl CommandHandler for EnvCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let config = Config::load(&layout).ok();

        let mut rows: Vec<(&str, &Path)> = vec![
            ("INRO_HOME", layout.inro_dir.as_path()),
            ("config_path", layout.config_path.as_path()),
            ("manifest_path", layout.manifest_path.as_path()),
            ("pkgs_dir", layout.pkgs_dir.as_path()),
            ("managed_registry_dir", layout.managed_registry_dir.as_path()),
            ("user_registry_dir", layout.user_registry_dir.as_path()),
        ];
        if let Some(cfg) = config.as_ref() {
            rows.push(("bin_dir", cfg.bin_dir.as_path()));
        }

        let key_width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        for (key, path) in rows {
            println!("{key:<key_width$}  {}", path.display());
        }

        Ok(())
    }
}
