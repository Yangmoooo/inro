use std::fs::File;
use std::io::copy;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn download_file(url: &str, dest_dir: &Path) -> Result<PathBuf> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download from URL: {}", url))?;

    let file_name = Path::new(url)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("inro-download.tmp");

    let dest_path = dest_dir.join(file_name);
    let mut dest_file = File::create(&dest_path)
        .with_context(|| format!("Failed to create destination file: {:?}", dest_path))?;

    let content = response.bytes()
        .context("Failed to read response body bytes")?;
    copy(&mut content.as_ref(), &mut dest_file)
        .context("Failed to write downloaded content to disk")?;

    Ok(dest_path)
}
