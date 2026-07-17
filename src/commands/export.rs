use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::{package_set, warn};

pub struct ExportCommand {
    pub output: Option<PathBuf>,
}

impl CommandHandler for ExportCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let manifest = Manifest::load(&layout.manifest_path)?;
        let package_set = package_set::render(&manifest);
        if let Some(path) = &self.output {
            write_atomic(path, package_set.contents.as_bytes())?;
        } else {
            io::stdout().write_all(package_set.contents.as_bytes())?;
        }
        if package_set.skipped_unlinked > 0 {
            warn!("Skipped {} unlinked package(s)", package_set.skipped_unlinked);
        }

        Ok(())
    }
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create export directory: {}", parent.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!("Failed to create temporary export file in {}", parent.display())
    })?;
    temp.write_all(contents)
        .with_context(|| format!("Failed to write package export: {}", path.display()))?;
    temp.flush().with_context(|| format!("Failed to flush package export: {}", path.display()))?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to save package export: {}", path.display()))?;
    Ok(())
}
