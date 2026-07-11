use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::warn;

/// Buffer size for file I/O operations (1 MB).
/// Using a large buffer significantly reduces system calls for large files.
const BUFFER_SIZE: usize = 1024 * 1024;

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

    pub(crate) fn from_magic_bytes(path: &Path) -> io::Result<Option<Self>> {
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
                warn!("Skipping zip entry with suspicious path: '{}'", zip_file.name());
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

#[cfg(test)]
mod tests {
    use super::*;

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
