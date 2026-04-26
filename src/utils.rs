use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write, copy};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local, Utc};
use chrono_humanize::HumanTime;
use futures::StreamExt;
use supports_hyperlinks::supports_hyperlinks;
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::progress::PkgProgress;
use crate::remotes::AssetSelector;
use crate::{client, detail, warn};

/// Buffer size for file I/O operations (1 MB).
/// Using a large buffer significantly reduces system calls for large files.
const BUFFER_SIZE: usize = 1024 * 1024;

pub fn unique(strs: &[String]) -> Vec<String> {
    let mut vec = strs.to_owned();
    vec.sort_unstable();
    vec.dedup();
    vec
}

/// Async download with progress tracking.
pub async fn download_file_with_progress(
    url: &str,
    dest_dir: &Path,
    size: u64,
    progress: &PkgProgress,
) -> Result<PathBuf> {
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
    let mut dest_file = tokio::fs::File::create(&dest_path)
        .await
        .with_context(|| format!("Failed to create destination file: {}", dest_path.display()))?;

    // Set progress bar length
    progress.set_length(size);

    // Stream the response body and update progress
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Failed to read response chunk")?;
        dest_file.write_all(&chunk).await.context("Failed to write chunk to disk")?;
        progress.inc(chunk.len() as u64);
    }

    Ok(dest_path)
}

/// Sync download (for source update).
pub fn download_file(url: &str, dest_dir: &Path) -> Result<PathBuf> {
    detail!("Downloading from {url}...");

    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download from URL: {url}"))?;
    let response =
        response.error_for_status().with_context(|| format!("HTTP error for URL: {url}"))?;

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
    // Checksum
    ".sha256",
    ".sha256sum",
    ".md5",
    ".asc",
    ".sig",
    // Plain
    ".txt",
    ".md",
    // Data
    ".xml",
    ".json",
    ".yml",
    ".yaml",
    ".toml",
    // Installer
    ".deb",
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
    false // An elf like xxx-v0.1.0-linux-x86_64 need to be specified in registry
}

#[derive(Debug)]
pub enum FileType {
    // Archive
    TarGz,
    TarXz,
    TarBz2,
    SevenZ,
    Zip,
    // Binary
    Pe,
    Elf,
    MachO,
}

impl FileType {
    /// Detect file type by extension or magic bytes.
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
        if data.starts_with(&[0xFE, 0xED, 0xFA, 0xCE])
            || data.starts_with(&[0xFE, 0xED, 0xFA, 0xCF])
            || data.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
            || data.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
            || data.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE])
            || data.starts_with(&[0xCA, 0xFE, 0xBA, 0xBF])
        {
            return Ok(Some(Self::MachO));
        }

        Ok(None)
    }
}

/// Extract file to destination directory based on its type.
pub fn extract_file(file_path: &Path, dest_dir: &Path) -> Result<FileType> {
    let file_type = FileType::detect(file_path)?;
    match file_type {
        FileType::TarGz => {
            let file = File::open(file_path).context("Failed to open asset file")?;
            let reader = BufReader::with_capacity(BUFFER_SIZE, file);
            let tar = flate2::read::GzDecoder::new(reader);
            extract_tar_buffered(tar, dest_dir).context("Failed to extract tar.gz archive")?;
        }
        FileType::TarXz => {
            let file = File::open(file_path).context("Failed to open asset file")?;
            let reader = BufReader::with_capacity(BUFFER_SIZE, file);
            let tar = xz2::read::XzDecoder::new(reader);
            extract_tar_buffered(tar, dest_dir).context("Failed to extract tar.xz archive")?;
        }
        FileType::TarBz2 => {
            let file = File::open(file_path).context("Failed to open asset file")?;
            let reader = BufReader::with_capacity(BUFFER_SIZE, file);
            let tar = bzip2::read::BzDecoder::new(reader);
            extract_tar_buffered(tar, dest_dir).context("Failed to extract tar.bz2 archive")?;
        }
        FileType::SevenZ => {
            sevenz_rust2::decompress_file(file_path, dest_dir)?;
        }
        FileType::Zip => {
            extract_zip_buffered(file_path, dest_dir).context("Failed to extract zip archive")?;
        }
        FileType::Pe | FileType::Elf | FileType::MachO => {
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

/// Resolve a path under `base` without path traversal.
///
/// Normalizes `..` and `.` components purely lexically, then checks that
/// the result stays within `base`. Returns `None` if the path escapes.
fn safe_join_path(base: &Path, untrusted: &Path) -> Option<PathBuf> {
    use std::path::Component;

    let mut resolved = base.to_path_buf();
    for component in untrusted.components() {
        match component {
            Component::Normal(c) => resolved.push(c),
            Component::CurDir => {}
            // ParentDir: go up but never above base
            Component::ParentDir => {
                if !resolved.starts_with(base) || resolved == base {
                    return None;
                }
                resolved.pop();
            }
            // RootDir / Prefix: absolute path — reject
            _ => return None,
        }
    }

    if resolved.starts_with(base) { Some(resolved) } else { None }
}

/// Extract a TAR archive using large buffers for better performance.
///
/// This function manually extracts files instead of using `Archive::unpack()`
/// to control buffer sizes. All entry paths are validated against path
/// traversal before extraction.
fn extract_tar_buffered<R: Read>(reader: R, dest_dir: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    let mut buffer = vec![0u8; BUFFER_SIZE];

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?;
        let entry_type = entry.header().entry_type();

        let out_path = match safe_join_path(dest_dir, &entry_path) {
            Some(p) => p,
            None => {
                bail!("tar entry '{}' would escape destination directory", entry_path.display())
            }
        };

        if entry_type.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else if entry_type.is_file() {
            if let Some(parent) = out_path.parent()
                && !parent.exists()
            {
                fs::create_dir_all(parent)?;
            }

            let out_file = File::create(&out_path)?;
            let mut writer = BufWriter::with_capacity(BUFFER_SIZE, out_file);

            loop {
                let bytes_read = entry.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                writer.write_all(&buffer[..bytes_read])?;
            }
            writer.flush()?;

            #[cfg(unix)]
            if let Ok(mode) = entry.header().mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))?;
            }
        } else if entry_type.is_symlink() {
            #[cfg(unix)]
            {
                if let Some(parent) = out_path.parent()
                    && !parent.exists()
                {
                    fs::create_dir_all(parent)?;
                }
                if let Some(target) = entry.link_name()? {
                    // Resolve the symlink target relative to the entry's parent
                    // *inside the archive* (a relative path), then validate it
                    // stays within `dest_dir`.
                    let entry_parent = entry_path.parent().unwrap_or(Path::new(""));
                    if target.is_absolute()
                        || safe_join_path(dest_dir, &entry_parent.join(&target)).is_none()
                    {
                        bail!(
                            "tar symlink '{}' -> '{}' would escape destination directory",
                            entry_path.display(),
                            target.display()
                        );
                    }
                    std::os::unix::fs::symlink(&target, &out_path)?;
                }
            }
        } else if entry_type.is_hard_link()
            && let Some(target) = entry.link_name()?
        {
            let target_path = match safe_join_path(dest_dir, &target) {
                Some(p) => p,
                None => bail!(
                    "tar hard link '{}' -> '{}' would escape destination directory",
                    entry_path.display(),
                    target.display()
                ),
            };
            fs::hard_link(&target_path, &out_path)?;
        }
    }

    Ok(())
}

/// Extract a ZIP archive using large buffers for better performance.
///
/// This function manually extracts files instead of using
/// `ZipArchive::extract()` to control buffer sizes.
fn extract_zip_buffered(file_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = File::open(file_path)?;
    let reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut archive = zip::ZipArchive::new(reader)?;
    let mut buffer = vec![0u8; BUFFER_SIZE];

    for i in 0..archive.len() {
        let mut zip_file = archive.by_index(i)?;
        let out_path = match zip_file.enclosed_name() {
            Some(path) => dest_dir.join(path),
            None => {
                warn!(
                    "Skipping zip entry with suspicious path: '{}'",
                    zip_file.name()
                );
                continue;
            }
        };

        if zip_file.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let out_file = File::create(&out_path)?;
            let mut writer = BufWriter::with_capacity(BUFFER_SIZE, out_file);

            loop {
                let bytes_read = zip_file.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                writer.write_all(&buffer[..bytes_read])?;
            }
            writer.flush()?;

            #[cfg(unix)]
            if let Some(mode) = zip_file.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))?;
            }
        }
    }

    Ok(())
}

/// If the root directory contains a single subdirectory, move its contents up
/// and remove it.
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

/// Rename the single file in the root directory to the target name.
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

/// Recursively search for a binary file by name (case-insensitive).
///
/// When multiple files share the same name (e.g. an executable in `usr/bin/`
/// and a bash-completion script in `usr/share/bash-completion/completions/`),
/// the function prefers files that are executable or reside under a directory
/// whose name is `bin` (e.g. `/usr/bin`, `/usr/local/bin`). Plain data files
/// with the same name are ignored.
pub fn find_binary_in_dir(root: &Path, bin_name: &str) -> Option<PathBuf> {
    let walker = WalkDir::new(root).into_iter();
    let target = bin_name.to_lowercase();

    for entry in walker.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file()
            && let Some(fname) = path.file_name().and_then(|s| s.to_str())
            && fname.to_lowercase() == target
        {
            // Prefer candidates that sit under a `bin/` directory or are
            // executable on Unix.
            if is_likely_binary(path) {
                return Some(path.to_path_buf());
            }
        }
    }
    None
}

/// Heuristic: returns `true` when the file looks like a real binary rather
/// than a data/completion file.
fn is_likely_binary(path: &Path) -> bool {
    if matches!(
        FileType::from_magic_bytes(path),
        Ok(Some(FileType::Elf | FileType::MachO | FileType::Pe))
    ) {
        return true;
    }

    // Check if any parent directory is named "bin" (skip the file itself)
    if let Some(parent) = path.parent()
        && parent.ancestors().any(|a| a.file_name().and_then(|n| n.to_str()) == Some("bin"))
    {
        return true;
    }

    // On Unix, check for the executable permission bit
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata()
            && meta.permissions().mode() & 0o111 != 0
        {
            return true;
        }
    }

    false
}

/// Create a symlink from `link` to `original`, replacing existing link/file if
/// necessary.
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

/// Sanitize version string to be filesystem-safe.
pub fn sanitize_version(raw_version: &str) -> String { raw_version.replace(['/', '\\', ':'], "-") }

/// Format a DateTime<Utc> into a string with absolute and relative time.
pub fn format_date(dt: &DateTime<Utc>) -> String {
    let local_dt: DateTime<Local> = DateTime::from(*dt);
    let abs_time = local_dt.format("%Y-%m-%d").to_string();
    let rel_time = HumanTime::from(*dt).to_string();

    // "202x-xx-xx (x days ago)"
    format!("{abs_time}, {rel_time}")
}

/// Create a terminal hyperlink if supported.
pub fn terminal_link(text: &str, url: &str) -> String {
    if supports_hyperlinks() {
        format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
    } else {
        text.to_string()
    }
}

/// Derive a version-agnostic asset selector from an asset filename.
///
/// Given an asset like `ripgrep-15.1.0-x86_64-apple-darwin.tar.gz` and
/// version tag `v15.1.0`, produces a glob like
/// `ripgrep-*-x86_64-apple-darwin.tar.gz` when possible.
pub fn derive_asset_selector(asset_name: &str, version_tag: &str) -> AssetSelector {
    derive_asset_selector_from_assets(asset_name, version_tag, &[asset_name.to_string()])
}

/// Derive a version-agnostic selector using the whole release asset list to
/// prefer selectors that uniquely identify the selected asset.
pub fn derive_asset_selector_from_assets(
    asset_name: &str,
    version_tag: &str,
    all_asset_names: &[String],
) -> AssetSelector {
    let mut candidates: Vec<AssetSelector> = asset_glob_candidates(asset_name, version_tag)
        .into_iter()
        .map(AssetSelector::Glob)
        .collect();
    candidates.push(AssetSelector::Glob(glob_escape(asset_name)));

    let tokens = stable_asset_tokens(asset_name, version_tag);
    if !tokens.is_empty() {
        candidates.push(AssetSelector::Tokens(tokens));
    }

    let mut unique_candidates = Vec::new();
    for candidate in candidates {
        if !unique_candidates.contains(&candidate) {
            unique_candidates.push(candidate);
        }
    }

    let mut scored_candidates: Vec<_> = unique_candidates
        .into_iter()
        .map(|selector| {
            let score = score_asset_selector(&selector, asset_name, version_tag, all_asset_names);
            (selector, score)
        })
        .collect();
    scored_candidates.sort_by_key(|b| std::cmp::Reverse(b.1));

    scored_candidates
        .into_iter()
        .find_map(|(selector, score)| (score > i32::MIN).then_some(selector))
        .unwrap_or_else(|| AssetSelector::Glob(glob_escape(asset_name)))
}

pub fn asset_matches_selector(asset_name: &str, selector: &AssetSelector) -> bool {
    match selector {
        AssetSelector::Glob(pattern) => glob_match(pattern, asset_name),
        AssetSelector::Tokens(tokens) => {
            !tokens.is_empty() && tokens.iter().all(|token| asset_name.contains(token))
        }
    }
}

fn asset_glob_candidates(asset_name: &str, version_tag: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for version in version_variants(version_tag) {
        for (start, _) in asset_name.match_indices(&version) {
            let end = start + version.len();
            let before = clean_asset_glob_prefix(&asset_name[..start]);
            let after = glob_escape(&asset_name[end..]);
            candidates.push(format!("{before}*{after}"));
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn version_variants(version_tag: &str) -> Vec<String> {
    let mut variants = vec![version_tag.to_string()];
    if let Some(bare) = version_tag.strip_prefix('v')
        && !bare.is_empty()
    {
        variants.push(bare.to_string());
    } else if is_plain_version_like(version_tag) {
        variants.push(format!("v{version_tag}"));
    }
    variants.extend(version_like_fragments(version_tag));
    variants.sort_by_key(|s| std::cmp::Reverse(s.len()));
    variants.dedup();
    variants
}

fn version_like_fragments(s: &str) -> Vec<String> {
    let mut fragments = Vec::new();
    let bytes = s.as_bytes();
    let mut start = None;

    for (idx, byte) in bytes.iter().enumerate() {
        let is_version_char = byte.is_ascii_digit()
            || (*byte == b'.' && start.is_some())
            || (*byte == b'-' && start.is_some())
            || (*byte == b'_' && start.is_some());

        if byte.is_ascii_digit() {
            if start.is_none() {
                start = Some(idx);
            }
        } else if !is_version_char && let Some(fragment_start) = start.take() {
            push_version_like_fragment(&mut fragments, &s[fragment_start..idx]);
        }
    }

    if let Some(fragment_start) = start {
        push_version_like_fragment(&mut fragments, &s[fragment_start..]);
    }

    fragments
}

fn push_version_like_fragment(fragments: &mut Vec<String>, raw: &str) {
    let fragment = raw.trim_matches(['-', '_', '.']);
    if is_plain_version_like(fragment) {
        fragments.push(fragment.to_string());
    }
}

fn is_plain_version_like(s: &str) -> bool {
    let mut dot_count = 0usize;
    let mut digit_count = 0usize;

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            digit_count += 1;
        } else if ch == '.' {
            dot_count += 1;
        } else if ch == '-' || ch == '_' || ch.is_ascii_alphabetic() {
            // allow simple pre-release/build-ish suffixes such as 1.2.3-beta.1
        } else {
            return false;
        }
    }

    digit_count > 0
        && dot_count > 0
        && s.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && s.chars().last().is_some_and(|ch| ch.is_ascii_alphanumeric())
}

fn clean_asset_glob_prefix(s: &str) -> String {
    if s == "v" {
        return String::new();
    }
    for suffix in ["-v", "_v", ".v"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            let separator = suffix.chars().next().unwrap_or('-');
            return glob_escape(&format!("{stripped}{separator}"));
        }
    }
    glob_escape(s)
}

fn stable_asset_tokens(asset_name: &str, version_tag: &str) -> Vec<String> {
    let mut stripped = asset_name.to_string();
    for version in version_variants(version_tag) {
        stripped = stripped.replace(&version, " ");
    }

    let mut tokens: Vec<String> = stripped
        .split(['-', '_', ' ', '.'])
        .filter(|part| part.len() >= 3 && !part.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
        .collect();

    if let Some(ext) = asset_extension(asset_name) {
        tokens.push(ext);
    }

    tokens.sort();
    tokens.dedup();
    tokens
}

fn asset_extension(asset_name: &str) -> Option<String> {
    [".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz", ".zip", ".7z", ".exe"]
        .iter()
        .find(|ext| asset_name.ends_with(**ext))
        .map(|ext| ext.trim_start_matches('.').to_string())
}

fn glob_escape(s: &str) -> String {
    s.chars()
        .flat_map(|ch| match ch {
            '*' | '?' | '\\' => ['\\', ch],
            _ => ['\0', ch],
        })
        .filter(|ch| *ch != '\0')
        .collect()
}

pub fn glob_match(pattern: &str, text: &str) -> bool {
    #[derive(Clone, Copy)]
    enum GlobToken {
        Star,
        Any,
        Literal(char),
    }

    let mut tokens = Vec::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => tokens.push(GlobToken::Star),
            '?' => tokens.push(GlobToken::Any),
            '\\' => tokens.push(GlobToken::Literal(chars.next().unwrap_or('\\'))),
            literal => tokens.push(GlobToken::Literal(literal)),
        }
    }

    let text: Vec<char> = text.chars().collect();
    let mut previous = vec![false; text.len() + 1];
    previous[0] = true;

    for token in tokens {
        let mut current = vec![false; text.len() + 1];
        match token {
            GlobToken::Star => {
                current[0] = previous[0];
                for idx in 1..=text.len() {
                    current[idx] = current[idx - 1] || previous[idx];
                }
            }
            GlobToken::Any => {
                current[1..=text.len()].copy_from_slice(&previous[..text.len()]);
            }
            GlobToken::Literal(ch) => {
                for idx in 1..=text.len() {
                    current[idx] = previous[idx - 1] && text[idx - 1] == ch;
                }
            }
        }
        previous = current;
    }

    previous[text.len()]
}

fn score_asset_selector(
    selector: &AssetSelector,
    asset_name: &str,
    version_tag: &str,
    all_asset_names: &[String],
) -> i32 {
    if !asset_matches_selector(asset_name, selector) {
        return i32::MIN;
    }

    let matches =
        all_asset_names.iter().filter(|name| asset_matches_selector(name, selector)).count();
    let mut score = 0;

    if matches == 1 {
        score += 1000;
    } else {
        score -= (matches as i32) * 100;
    }

    let selector_text = selector.to_string();
    let selector_lower = selector_text.to_lowercase();

    if matches!(selector, AssetSelector::Glob(_)) {
        score += 50;
    }

    for version in version_variants(version_tag) {
        if selector_lower.contains(&version.to_lowercase()) {
            score -= 1000;
        }
    }

    if selector_text.contains('*') {
        score += 500;
    }

    if [".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz", ".zip", ".7z", ".exe"]
        .iter()
        .any(|ext| selector_lower.ends_with(ext))
    {
        score += 80;
    }

    if [
        "linux", "darwin", "apple", "macos", "windows", "win", "x86_64", "amd64", "x64", "aarch64",
        "arm64",
    ]
    .iter()
    .any(|part| selector_lower.contains(part))
    {
        score += 60;
    }

    score += selector_text.len().min(80) as i32;
    if selector_text.len() < 4 {
        score -= 200;
    }

    score
}

/// Parses "package" or "package@version".
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

    #[test]
    fn filetype_from_magic_bytes_macho_64() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("chsrc-aarch64-macos");
        fs::write(&path, [0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0]).unwrap();

        assert!(matches!(FileType::detect(&path).unwrap(), FileType::MachO));
    }

    #[test]
    fn filetype_from_magic_bytes_macho_fat() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("universal-macos");
        fs::write(&path, [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0]).unwrap();

        assert!(matches!(FileType::detect(&path).unwrap(), FileType::MachO));
    }

    #[test]
    fn extract_file_copies_macho_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("chsrc-aarch64-macos");
        let dst = tmp.path().join("out");
        fs::create_dir_all(&dst).unwrap();
        fs::write(&src, [0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0]).unwrap();

        let file_type = extract_file(&src, &dst).unwrap();

        assert!(matches!(file_type, FileType::MachO));
        let copied = dst.join("chsrc-aarch64-macos");
        assert!(copied.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(copied).unwrap().permissions().mode();
            assert_ne!(mode & 0o111, 0);
        }
    }

    // ==================== safe_join_path() ====================

    #[test]
    fn safe_join_normal_path() {
        let base = Path::new("/tmp/extract");
        let result = safe_join_path(base, Path::new("dir/file.txt"));
        assert_eq!(result, Some(PathBuf::from("/tmp/extract/dir/file.txt")));
    }

    #[test]
    fn safe_join_rejects_parent_escape() {
        let base = Path::new("/tmp/extract");
        assert!(safe_join_path(base, Path::new("../../etc/passwd")).is_none());
    }

    #[test]
    fn safe_join_rejects_absolute_path() {
        let base = Path::new("/tmp/extract");
        assert!(safe_join_path(base, Path::new("/etc/passwd")).is_none());
    }

    #[test]
    fn safe_join_allows_internal_parent() {
        let base = Path::new("/tmp/extract");
        // "a/b/../c" should resolve to "a/c" which is still inside base
        let result = safe_join_path(base, Path::new("a/b/../c"));
        assert_eq!(result, Some(PathBuf::from("/tmp/extract/a/c")));
    }

    #[test]
    fn safe_join_rejects_exact_boundary_escape() {
        let base = Path::new("/tmp/extract");
        // "a/../../etc" -> would go above base
        assert!(safe_join_path(base, Path::new("a/../../etc")).is_none());
    }

    #[test]
    fn safe_join_ignores_curdir() {
        let base = Path::new("/tmp/extract");
        let result = safe_join_path(base, Path::new("./dir/./file.txt"));
        assert_eq!(result, Some(PathBuf::from("/tmp/extract/dir/file.txt")));
    }

    // ==================== find_binary_in_dir() ====================

    #[test]
    fn find_binary_prefers_bin_dir_over_completions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create two files with the same name: one under usr/bin/, one under
        // usr/share/bash-completion/completions/
        let bin_dir = root.join("usr/bin");
        let comp_dir = root.join("usr/share/bash-completion/completions");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&comp_dir).unwrap();

        fs::write(bin_dir.join("fastfetch"), b"ELF binary").unwrap();
        fs::write(comp_dir.join("fastfetch"), b"# completion script").unwrap();

        let result = find_binary_in_dir(root, "fastfetch").unwrap();
        assert_eq!(result, bin_dir.join("fastfetch"));
    }

    #[test]
    fn find_binary_ignores_only_plain_match() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // A same-name plain data file should not be treated as an installable binary.
        let some_dir = root.join("usr/share/data");
        fs::create_dir_all(&some_dir).unwrap();
        fs::write(some_dir.join("mytool"), b"data").unwrap();

        assert!(find_binary_in_dir(root, "mytool").is_none());
    }

    #[test]
    fn find_binary_ignores_completion_only_match() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let comp_dir = root.join("aria2/release-1.37.0/doc/bash_completion");
        fs::create_dir_all(&comp_dir).unwrap();
        fs::write(comp_dir.join("aria2c"), b"# completion script").unwrap();

        assert!(find_binary_in_dir(root, "aria2c").is_none());
    }

    #[test]
    fn find_binary_accepts_magic_binary_without_exec_bit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let tool = root.join("tool");
        fs::write(&tool, b"\x7fELF binary").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o644)).unwrap();
        }

        let result = find_binary_in_dir(root, "tool").unwrap();
        assert_eq!(result, tool);
    }

    #[test]
    fn find_binary_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_binary_in_dir(tmp.path(), "nonexistent").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_binary_prefers_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Two files with the same name in non-bin directories, but one is
        // executable.
        let dir_a = root.join("share/completions");
        let dir_b = root.join("libexec");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        let non_exec = dir_a.join("tool");
        fs::write(&non_exec, b"# script").unwrap();
        fs::set_permissions(&non_exec, fs::Permissions::from_mode(0o644)).unwrap();

        let exec = dir_b.join("tool");
        fs::write(&exec, b"\x7fELF").unwrap();
        fs::set_permissions(&exec, fs::Permissions::from_mode(0o755)).unwrap();

        let result = find_binary_in_dir(root, "tool").unwrap();
        assert_eq!(result, exec);
    }

    // ==================== derive_asset_selector() ====================

    #[test]
    fn derive_selector_strips_v_prefixed_version() {
        assert_eq!(
            derive_asset_selector("ripgrep-15.1.0-x86_64-apple-darwin.tar.gz", "v15.1.0"),
            AssetSelector::Glob("ripgrep-*-x86_64-apple-darwin.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_strips_bare_version() {
        assert_eq!(
            derive_asset_selector("delta-0.18.2-x86_64-apple-darwin.tar.gz", "0.18.2"),
            AssetSelector::Glob("delta-*-x86_64-apple-darwin.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_with_underscore_separator() {
        assert_eq!(
            derive_asset_selector("fd_10.2.0_amd64.deb", "v10.2.0"),
            AssetSelector::Glob("fd_*_amd64.deb".to_string())
        );
    }

    #[test]
    fn derive_selector_fallback_when_version_not_in_name() {
        assert_eq!(
            derive_asset_selector("tool-linux-amd64", "v1.0.0"),
            AssetSelector::Glob("tool-linux-amd64".to_string())
        );
    }

    #[test]
    fn derive_selector_version_at_end_uses_prefix() {
        assert_eq!(
            derive_asset_selector("tool-1.0.0", "v1.0.0"),
            AssetSelector::Glob("tool-*".to_string())
        );
    }

    #[test]
    fn derive_selector_v_prefix_in_name_uses_prefix() {
        assert_eq!(
            derive_asset_selector("tool-v1.0.0", "v1.0.0"),
            AssetSelector::Glob("tool-*".to_string())
        );
    }

    #[test]
    fn derive_selector_version_at_end_bare_tag_uses_prefix() {
        assert_eq!(
            derive_asset_selector("tool-1.0.0", "1.0.0"),
            AssetSelector::Glob("tool-*".to_string())
        );
    }

    #[test]
    fn derive_selector_version_at_start_uses_suffix() {
        assert_eq!(
            derive_asset_selector("v1.0.0-tool-linux-x86_64.tar.gz", "v1.0.0"),
            AssetSelector::Glob("*-tool-linux-x86_64.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_prefers_unique_versionless_glob() {
        let assets = vec![
            "tool-v1.0.0-linux-x86_64.tar.gz".to_string(),
            "tool-v1.0.0-linux-aarch64.tar.gz".to_string(),
            "tool-v1.0.0-windows-x86_64.zip".to_string(),
        ];

        assert_eq!(
            derive_asset_selector_from_assets("tool-v1.0.0-linux-x86_64.tar.gz", "v1.0.0", &assets),
            AssetSelector::Glob("tool-*-linux-x86_64.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_handles_asset_with_v_when_tag_is_bare() {
        assert_eq!(
            derive_asset_selector("tool-v1.0.0-linux.tar.gz", "1.0.0"),
            AssetSelector::Glob("tool-*-linux.tar.gz".to_string())
        );
    }

    #[test]
    fn derive_selector_handles_release_prefixed_tag() {
        assert_eq!(
            derive_asset_selector("aria2-1.37.0.tar.xz", "release-1.37.0"),
            AssetSelector::Glob("aria2-*.tar.xz".to_string())
        );
    }

    #[test]
    fn version_variants_do_not_prefix_non_plain_versions() {
        let variants = version_variants("release-1.37.0");

        assert!(variants.contains(&"release-1.37.0".to_string()));
        assert!(variants.contains(&"1.37.0".to_string()));
        assert!(!variants.contains(&"vrelease-1.37.0".to_string()));
    }

    #[test]
    fn version_variants_prefix_plain_versions() {
        let variants = version_variants("1.37.0");

        assert!(variants.contains(&"1.37.0".to_string()));
        assert!(variants.contains(&"v1.37.0".to_string()));
    }

    #[test]
    fn asset_selector_tokens_match_all_parts() {
        let selector = AssetSelector::Tokens(vec!["tool".to_string(), "linux.tar.gz".to_string()]);
        assert!(asset_matches_selector("tool-v1.0.0-linux.tar.gz", &selector));
        assert!(!asset_matches_selector("other-v1.0.0-linux.tar.gz", &selector));
    }

    #[test]
    fn glob_match_supports_star_question_and_escape() {
        assert!(glob_match("aria2-*.tar.xz", "aria2-1.37.0.tar.xz"));
        assert!(glob_match("tool-?.zip", "tool-a.zip"));
        assert!(!glob_match("tool-?.zip", "tool-ab.zip"));
        assert!(glob_match(r"literal-\*.zip", "literal-*.zip"));
    }

    #[test]
    fn glob_match_handles_many_stars_iteratively() {
        let pattern = "*a*a*a*a*a*a*a*a*a*a*z";
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaz";

        assert!(glob_match(pattern, text));
        assert!(!glob_match(pattern, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    }

    // ==================== tar symlink extraction ====================

    #[cfg(unix)]
    fn build_tar_with_symlink(tar_path: &Path, link_name: &str, target: &str) {
        let file = File::create(tar_path).unwrap();
        let mut builder = tar::Builder::new(file);

        // Real file the link points at (when target is "real.txt").
        let mut real_header = tar::Header::new_gnu();
        real_header.set_path("real.txt").unwrap();
        real_header.set_size(5);
        real_header.set_mode(0o644);
        real_header.set_entry_type(tar::EntryType::Regular);
        real_header.set_cksum();
        builder.append(&real_header, &b"hello"[..]).unwrap();

        let mut link_header = tar::Header::new_gnu();
        link_header.set_path(link_name).unwrap();
        link_header.set_size(0);
        link_header.set_mode(0o777);
        link_header.set_entry_type(tar::EntryType::Symlink);
        link_header.set_link_name(target).unwrap();
        link_header.set_cksum();
        builder.append(&link_header, std::io::empty()).unwrap();

        builder.into_inner().unwrap().sync_all().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn extract_tar_preserves_relative_symlink_within_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let tar_path = tmp.path().join("with-symlink.tar");
        let dest = tmp.path().join("out");
        fs::create_dir_all(&dest).unwrap();

        // sub/link -> ../real.txt resolves inside dest_dir
        build_tar_with_symlink(&tar_path, "sub/link", "../real.txt");

        let file = File::open(&tar_path).unwrap();
        extract_tar_buffered(file, &dest).expect("symlink within dest must extract");

        let link_path = dest.join("sub/link");
        assert!(link_path.is_symlink(), "expected symlink at {}", link_path.display());
        let resolved_target = std::fs::read_link(&link_path).unwrap();
        assert_eq!(resolved_target, PathBuf::from("../real.txt"));
        // Following the symlink should land on the file inside dest.
        assert_eq!(fs::read_to_string(link_path).unwrap(), "hello");
    }

    #[cfg(unix)]
    #[test]
    fn extract_tar_rejects_symlink_escaping_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let tar_path = tmp.path().join("escaping.tar");
        let dest = tmp.path().join("out");
        fs::create_dir_all(&dest).unwrap();

        // link -> ../../etc/passwd escapes dest_dir
        build_tar_with_symlink(&tar_path, "link", "../../etc/passwd");

        let file = File::open(&tar_path).unwrap();
        let err = extract_tar_buffered(file, &dest).unwrap_err();
        assert!(
            err.to_string().contains("escape destination directory"),
            "unexpected error: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn extract_tar_rejects_absolute_symlink_target() {
        let tmp = tempfile::tempdir().unwrap();
        let tar_path = tmp.path().join("absolute.tar");
        let dest = tmp.path().join("out");
        fs::create_dir_all(&dest).unwrap();

        build_tar_with_symlink(&tar_path, "link", "/etc/passwd");

        let file = File::open(&tar_path).unwrap();
        let err = extract_tar_buffered(file, &dest).unwrap_err();
        assert!(
            err.to_string().contains("escape destination directory"),
            "unexpected error: {err}"
        );
    }
}
