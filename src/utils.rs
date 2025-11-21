use std::fs::{self, File};
use std::io::{self, Read, copy, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

pub fn download_file(url: &str, dest_dir: &Path) -> Result<PathBuf> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download from URL: {}", url))?;

    let file_name = Path::new(url)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("inro-download.tmp");

    let dest_path = dest_dir.join(file_name);
    let mut dest_file = File::create(&dest_path)
        .with_context(|| format!("Failed to create destination file: {}", dest_path.display()))?;

    let content = response.bytes()
        .context("Failed to read response body bytes")?;
    copy(&mut content.as_ref(), &mut dest_file)
        .context("Failed to write downloaded content to disk")?;

    Ok(dest_path)
}

#[derive(Debug)]
pub enum FileType {
    // archive
    TarGz,
    TarXz,
    TarBz2,
    Zip,
    // binary
    Pe,
    Elf,
}

impl FileType {
    pub fn detect(path: &Path) -> Result<Self> {
        if let Some(ft) = Self::from_extension(path) {
            return Ok(ft);
        }
        if let Ok(Some(ft)) = Self::from_magic_bytes(path) {
            return Ok(ft);
        }
        bail!("Unable to determine file type for '{}'.", path.display());
    }

    fn from_extension(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();

        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            return Some(Self::TarGz);
        }
        if name.ends_with(".tar.xz") || name.ends_with(".txz") {
            return Some(Self::TarXz);
        }
        if name.ends_with(".tar.bz2") || name.ends_with(".tbz") {
            return Some(Self::TarBz2);
        }
        if name.ends_with(".zip") {
            return Some(Self::Zip);
        }

        if name.ends_with(".exe") {
            return Some(Self::Pe);
        }

        None
    }

    fn from_magic_bytes(path: &Path) -> io::Result<Option<Self>> {
        let mut file = File::open(path)?;
        let mut buffer = [0u8; 8];
        let bytes_read = file.read(&mut buffer)?;
        let data = &buffer[..bytes_read];

        if data.starts_with(&[0x7F, 0x45, 0x4C, 0x46]) {
            return Ok(Some(Self::Elf));
        }
        if data.starts_with(&[0x4D, 0x5A]) {
            return Ok(Some(Self::Pe));
        }

        Ok(None)
    }
}

pub fn extract_file(file_path: &Path, dest_dir: &Path) -> Result<()> {
    let file_type = FileType::detect(file_path)?;
    let file = File::open(file_path).context("Failed to open asset file")?;
    let reader = BufReader::new(file);

    match file_type {
        FileType::TarGz => {
            let tar = flate2::read::GzDecoder::new(reader);
            let mut archive = tar::Archive::new(tar);
            archive.unpack(dest_dir).context("Failed to extract tar.gz archive")?;
        }
        FileType::TarXz => {
            let tar = xz2::read::XzDecoder::new(reader);
            let mut archive = tar::Archive::new(tar);
            archive.unpack(dest_dir).context("Failed to extract tar.xz archive")?;
        }
        FileType::TarBz2 => {
            let tar = bzip2::read::BzDecoder::new(reader);
            let mut archive = tar::Archive::new(tar);
            archive.unpack(dest_dir).context("Failed to extract tar.bz2 archive")?;
        }
        FileType::Zip => {
            let mut archive = zip::ZipArchive::new(reader)?;
            archive.extract(dest_dir).context("Failed to extract zip archive")?;
        }
        FileType::Pe | FileType::Elf => {
            let file_name = file_path.file_name().ok_or(anyhow!("Binary file name invalid"))?;
            let dest_file_path = dest_dir.join(file_name);
            fs::copy(file_path, &dest_file_path)?;

            #[cfg(target_family = "unix")]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&dest_file_path)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&dest_file_path, perms)?;
            }
        }
    }

    Ok(())
}
