use std::fs::{self, File};
use std::io::{self, BufReader, Read, copy};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local, Utc};
use chrono_humanize::HumanTime;
use supports_hyperlinks::supports_hyperlinks;
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::{client, report};

pub fn unique(strs: &[String]) -> Vec<String> {
    let mut vec = strs.to_owned();
    vec.sort_unstable();
    vec.dedup();
    vec
}

/// Async version of download_file (for install/update with parallel downloads)
pub async fn download_file_async(url: &str, dest_dir: &Path) -> Result<PathBuf> {
    report!(MsgType::Detail, "Downloading from {url}...",);

    let client = client::get();
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to download from URL: {url}"))?;

    let response =
        response.error_for_status().with_context(|| format!("HTTP error for URL: {url}"))?;

    let file_name =
        Path::new(url).file_name().and_then(|s| s.to_str()).unwrap_or("inro-download.tmp");

    let dest_path = dest_dir.join(file_name);
    let content = response.bytes().await.context("Failed to read response body bytes")?;

    let mut dest_file = tokio::fs::File::create(&dest_path)
        .await
        .with_context(|| format!("Failed to create destination file: {}", dest_path.display()))?;

    dest_file.write_all(&content).await.context("Failed to write downloaded content to disk")?;

    Ok(dest_path)
}

/// Sync version of download_file (for source update)
pub fn download_file(url: &str, dest_dir: &Path) -> Result<PathBuf> {
    report!(MsgType::Detail, "Downloading from {url}...",);

    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download from URL: {url}"))?;

    let file_name =
        Path::new(url).file_name().and_then(|s| s.to_str()).unwrap_or("inro-download.tmp");

    let dest_path = dest_dir.join(file_name);
    let mut dest_file = File::create(&dest_path)
        .with_context(|| format!("Failed to create destination file: {}", dest_path.display()))?;

    let content = response.bytes().context("Failed to read response body bytes")?;
    copy(&mut content.as_ref(), &mut dest_file)
        .context("Failed to write downloaded content to disk")?;

    Ok(dest_path)
}

const BLOCK_EXTENSIONS: &[&str] = &[
    ".sha256", // checksum
    ".sha256sum",
    ".md5",
    ".asc",
    ".sig",
    ".txt", // plain
    ".md",
    ".xml", // data
    ".json",
    ".yml",
    ".yaml",
    ".toml",
    ".deb", // installer
    ".rpm",
    ".msi",
    ".pkg",
    ".dmg",
];
const ALLOW_EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tgz", //
    ".tar.xz", ".txz", //
    ".tar.bz2", ".tbz", //
    ".7z",  //
    ".zip", //
    ".exe", //
];

pub fn is_ignored_format(name: &str) -> bool {
    BLOCK_EXTENSIONS.iter().any(|ext| name.ends_with(ext))
}
pub fn is_supported_format(name: &str) -> bool {
    if ALLOW_EXTENSIONS.iter().any(|ext| name.ends_with(ext)) {
        return true;
    }
    if !name.contains('.') {
        return true;
    }
    false // a elf like xxx-v0.1.0-linux-x86_64 need to be specified in registry
}

#[derive(Debug)]
pub enum FileType {
    // archive
    TarGz,
    TarXz,
    TarBz2,
    SevenZ,
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
        if name.ends_with(".7z") {
            return Some(Self::SevenZ);
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

pub fn extract_file(file_path: &Path, dest_dir: &Path) -> Result<FileType> {
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
        FileType::SevenZ => {
            sevenz_rust2::decompress_file(file_path, dest_dir)?;
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

    Ok(file_type)
}

pub fn flatten_single_directory(root_dir: &Path) -> Result<()> {
    let entries: Vec<_> = fs::read_dir(root_dir)?.filter_map(Result::ok).collect();

    if entries.len() != 1 {
        return Ok(());
    }

    let entry = &entries[0];
    let entry_path = entry.path();

    if !entry_path.is_dir() {
        return Ok(());
    }

    let sub_entries: Vec<_> = fs::read_dir(&entry_path)?.filter_map(Result::ok).collect();

    for sub_entry in sub_entries {
        let sub_path = sub_entry.path();
        let file_name = sub_entry.file_name();
        let target_path = root_dir.join(file_name);

        fs::rename(&sub_path, &target_path).with_context(|| {
            format!("Failed to move {} to {}", sub_path.display(), target_path.display())
        })?;
    }

    fs::remove_dir(&entry_path)?;

    Ok(())
}

pub fn rename_single_file(root_dir: &Path, target_name: &str) -> Result<()> {
    let entries: Vec<_> = fs::read_dir(root_dir)?.filter_map(Result::ok).collect();

    if entries.len() != 1 {
        return Ok(());
    }

    let entry = &entries[0];
    let entry_path = entry.path();

    if !entry_path.is_file() {
        return Ok(());
    }

    let target_path = root_dir.join(target_name);
    fs::rename(&entry_path, &target_path).with_context(|| {
        format!("Failed to move {} to {}", entry_path.display(), target_path.display())
    })?;

    Ok(())
}

pub fn find_binary_in_dir(root: &Path, bin_name: &str) -> Option<PathBuf> {
    let walker = WalkDir::new(root).into_iter();

    for entry in walker.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file()
            && let Some(fname) = path.file_name().and_then(|s| s.to_str())
            && fname.to_lowercase() == bin_name.to_lowercase()
        {
            return Some(path.to_path_buf());
        }
    }
    None
}

pub fn create_symlink(original: &Path, link: &Path) -> Result<()> {
    if link.exists() || link.is_symlink() {
        if link.is_dir() {
            fs::remove_dir_all(link)?;
        } else {
            fs::remove_file(link)?;
        }
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)?;
    }

    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(original, link) {
            Ok(()) => {}
            Err(e) => {
                // error code 1314: a required privilege is not held by the client
                if let Some(os_err) = e.raw_os_error()
                    && os_err == 1314
                {
                    return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "Creating symlinks on Windows requires Developer Mode or running as Administrator."
                        ).into());
                }
                return Err(e.into());
            }
        }
    }

    Ok(())
}

pub fn sanitize_version(raw_version: &str) -> String { raw_version.replace(['/', '\\', ':'], "-") }

pub fn format_date(dt: &DateTime<Utc>) -> String {
    let local_dt: DateTime<Local> = DateTime::from(*dt);
    let abs_time = local_dt.format("%Y-%m-%d").to_string();
    let rel_time = HumanTime::from(*dt).to_string();

    // "202x-xx-xx (x days ago)"
    format!("{abs_time}, {rel_time}")
}

pub fn terminal_link(text: &str, url: &str) -> String {
    if supports_hyperlinks() {
        format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
    } else {
        text.to_string()
    }
}

/// Parses "package" or "package@version"
pub fn parse_package_version(input: &str) -> (&str, Option<&str>) {
    if let Some((name, version)) = input.split_once('@') {
        (name, Some(version))
    } else {
        (input, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== unique() ====================

    #[test]
    fn unique_removes_duplicates() {
        let input = vec!["b".into(), "a".into(), "b".into(), "c".into(), "a".into()];
        let result = unique(&input);
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn unique_empty_input() {
        let input: Vec<String> = vec![];
        let result = unique(&input);
        assert!(result.is_empty());
    }

    #[test]
    fn unique_single_element() {
        let input = vec!["only".into()];
        let result = unique(&input);
        assert_eq!(result, vec!["only"]);
    }

    // ==================== is_ignored_format() ====================

    #[test]
    fn is_ignored_format_checksums() {
        assert!(is_ignored_format("file.sha256"));
        assert!(is_ignored_format("file.sha256sum"));
        assert!(is_ignored_format("file.md5"));
        assert!(is_ignored_format("file.asc"));
        assert!(is_ignored_format("file.sig"));
    }

    #[test]
    fn is_ignored_format_installers() {
        assert!(is_ignored_format("package.deb"));
        assert!(is_ignored_format("package.rpm"));
        assert!(is_ignored_format("package.msi"));
        assert!(is_ignored_format("package.pkg"));
        assert!(is_ignored_format("package.dmg"));
    }

    #[test]
    fn is_ignored_format_data_files() {
        assert!(is_ignored_format("readme.txt"));
        assert!(is_ignored_format("notes.md"));
        assert!(is_ignored_format("config.json"));
        assert!(is_ignored_format("data.xml"));
    }

    #[test]
    fn is_ignored_format_valid_archives_not_ignored() {
        assert!(!is_ignored_format("package.tar.gz"));
        assert!(!is_ignored_format("package.zip"));
        assert!(!is_ignored_format("package.7z"));
    }

    // ==================== is_supported_format() ====================

    #[test]
    fn is_supported_format_archives() {
        assert!(is_supported_format("package.tar.gz"));
        assert!(is_supported_format("package.tgz"));
        assert!(is_supported_format("package.tar.xz"));
        assert!(is_supported_format("package.txz"));
        assert!(is_supported_format("package.tar.bz2"));
        assert!(is_supported_format("package.tbz"));
        assert!(is_supported_format("package.7z"));
        assert!(is_supported_format("package.zip"));
    }

    #[test]
    fn is_supported_format_exe() {
        assert!(is_supported_format("binary.exe"));
    }

    #[test]
    fn is_supported_format_no_extension() {
        // ELF binaries often have no extension
        assert!(is_supported_format("ripgrep-linux-x86_64"));
    }

    #[test]
    fn is_supported_format_unknown_extension() {
        // Unknown extensions are not supported (need explicit config)
        assert!(!is_supported_format("file.unknown"));
        assert!(!is_supported_format("file.abc"));
    }

    // ==================== sanitize_version() ====================

    #[test]
    fn sanitize_version_replaces_slashes() {
        assert_eq!(sanitize_version("v1/2/3"), "v1-2-3");
        assert_eq!(sanitize_version("v1\\2\\3"), "v1-2-3");
    }

    #[test]
    fn sanitize_version_replaces_colons() {
        assert_eq!(sanitize_version("v1:2:3"), "v1-2-3");
    }

    #[test]
    fn sanitize_version_mixed() {
        assert_eq!(sanitize_version("v1/2:3\\4"), "v1-2-3-4");
    }

    #[test]
    fn sanitize_version_no_changes() {
        assert_eq!(sanitize_version("v1.2.3"), "v1.2.3");
        assert_eq!(sanitize_version("1.0.0-beta"), "1.0.0-beta");
    }

    // ==================== parse_package_version() ====================

    #[test]
    fn parse_package_version_with_version() {
        let (name, version) = parse_package_version("ripgrep@15.1.0");
        assert_eq!(name, "ripgrep");
        assert_eq!(version, Some("15.1.0"));
    }

    #[test]
    fn parse_package_version_without_version() {
        let (name, version) = parse_package_version("ripgrep");
        assert_eq!(name, "ripgrep");
        assert_eq!(version, None);
    }

    #[test]
    fn parse_package_version_with_v_prefix() {
        let (name, version) = parse_package_version("fd@v9.0.0");
        assert_eq!(name, "fd");
        assert_eq!(version, Some("v9.0.0"));
    }

    #[test]
    fn parse_package_version_empty_version() {
        // Edge case: trailing @
        let (name, version) = parse_package_version("pkg@");
        assert_eq!(name, "pkg");
        assert_eq!(version, Some(""));
    }

    // ==================== FileType::from_extension() ====================

    #[test]
    fn filetype_from_extension_tar_gz() {
        let path = Path::new("archive.tar.gz");
        assert!(matches!(FileType::from_extension(path), Some(FileType::TarGz)));

        let path = Path::new("archive.tgz");
        assert!(matches!(FileType::from_extension(path), Some(FileType::TarGz)));
    }

    #[test]
    fn filetype_from_extension_tar_xz() {
        let path = Path::new("archive.tar.xz");
        assert!(matches!(FileType::from_extension(path), Some(FileType::TarXz)));

        let path = Path::new("archive.txz");
        assert!(matches!(FileType::from_extension(path), Some(FileType::TarXz)));
    }

    #[test]
    fn filetype_from_extension_zip() {
        let path = Path::new("archive.zip");
        assert!(matches!(FileType::from_extension(path), Some(FileType::Zip)));
    }

    #[test]
    fn filetype_from_extension_7z() {
        let path = Path::new("archive.7z");
        assert!(matches!(FileType::from_extension(path), Some(FileType::SevenZ)));
    }

    #[test]
    fn filetype_from_extension_exe() {
        let path = Path::new("binary.exe");
        assert!(matches!(FileType::from_extension(path), Some(FileType::Pe)));
    }

    #[test]
    fn filetype_from_extension_unknown() {
        let path = Path::new("binary-linux-x86_64");
        assert!(FileType::from_extension(path).is_none());
    }
}
