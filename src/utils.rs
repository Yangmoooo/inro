use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, IsTerminal, copy};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, Utc};
use chrono_humanize::HumanTime;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::archive::FileType;
use crate::progress::PkgProgress;
use crate::{client, detail};

pub fn ensure_unique_package_args(args: &[String]) -> Result<()> {
    let mut seen = HashSet::new();
    for arg in args {
        let (name, _) = parse_package_version(arg);
        if !seen.insert(name) {
            bail!("Package '{name}' was specified more than once");
        }
    }
    Ok(())
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

    let file_name = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                .map(str::to_string)
        })
        .unwrap_or_else(|| "inro-download.tmp".to_string());

    let dest_path = dest_dir.join(file_name);
    let mut dest_file = tokio::fs::File::create(&dest_path)
        .await
        .with_context(|| format!("Failed to create destination file: {}", dest_path.display()))?;

    // Set progress bar length
    progress.set_length(if size == 0 { response.content_length().unwrap_or(0) } else { size });

    // Stream the response body and update progress
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Failed to read response chunk")?;
        dest_file.write_all(&chunk).await.context("Failed to write chunk to disk")?;
        progress.inc(chunk.len() as u64);
    }
    dest_file.flush().await.context("Failed to flush downloaded file")?;

    Ok(dest_path)
}

/// Sync download (for source update).
pub fn download_file(url: &str, dest_dir: &Path) -> Result<PathBuf> {
    detail!("Downloading from {url}...");

    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("inro/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build()
        .context("Failed to build HTTP client")?;
    let response =
        client.get(url).send().with_context(|| format!("Failed to download from URL: {url}"))?;
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

/// Rename the single file in the root directory to the target name.
pub fn rename_single_file(root_dir: &Path, target_name: &str) -> Result<()> {
    let entries: Vec<_> = fs::read_dir(root_dir)?.collect::<io::Result<_>>()?;

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
/// every candidate that looks like a real binary is scored and the highest
/// scorer wins. Plain data files with the same name are ignored. Ties are
/// broken deterministically by path depth and then lexicographically so the
/// result does not depend on filesystem traversal order.
pub fn find_binary_in_dir(root: &Path, bin_name: &str) -> Option<PathBuf> {
    let target = bin_name.to_lowercase();
    let mut candidates: Vec<(PathBuf, i32)> = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if fname.to_lowercase() != target {
            continue;
        }
        if let Some(score) = score_binary_candidate(path) {
            candidates.push((path.to_path_buf(), score));
        }
    }

    candidates.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.components().count().cmp(&b.0.components().count()))
            .then_with(|| a.0.cmp(&b.0))
    });
    candidates.into_iter().next().map(|(path, _)| path)
}

/// Score a same-name candidate so we can pick the most likely binary.
/// Returns `None` for files that show no positive signal (plain data /
/// completion scripts), so they are filtered out entirely.
fn score_binary_candidate(path: &Path) -> Option<i32> {
    let mut score = 0;
    let mut likely = false;

    if matches!(
        FileType::from_magic_bytes(path),
        Ok(Some(FileType::Elf | FileType::MachO | FileType::Pe))
    ) {
        score += 100;
        likely = true;
    }

    if let Some(parent) = path.parent()
        && parent.ancestors().any(|a| a.file_name().and_then(|n| n.to_str()) == Some("bin"))
    {
        score += 50;
        likely = true;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata()
            && meta.permissions().mode() & 0o111 != 0
        {
            score += 20;
            likely = true;
        }
    }

    likely.then_some(score)
}

/// Create a symlink from `link` to `original`, refusing to overwrite anything
/// that is not already a symlink managed by inro.
///
/// `owned_root` is the path inro considers its own (typically the layout's
/// `pkgs_dir`). An existing entry is replaced silently only when it is a
/// symlink whose target lies within `owned_root`. Foreign symlinks and
/// regular files trigger a hard error so the user's pre-existing tools do
/// not vanish behind inro's back.
pub fn create_symlink(original: &Path, link: &Path, owned_root: &Path) -> Result<()> {
    ensure_link_replaceable(link, owned_root)?;

    if link.is_symlink() {
        fs::remove_file(link)
            .with_context(|| format!("Failed to remove existing symlink: {}", link.display()))?;
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
                // error code 1314: a required privilege is not held by the
                // client
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

pub fn ensure_link_replaceable(link: &Path, owned_root: &Path) -> Result<()> {
    if link.is_symlink() {
        let raw_target = fs::read_link(link)
            .with_context(|| format!("Failed to read existing symlink: {}", link.display()))?;
        let abs_target = if raw_target.is_absolute() {
            raw_target.clone()
        } else {
            link.parent().unwrap_or(Path::new(".")).join(&raw_target)
        };
        let normalized_target = canonicalize_or_lexical(&abs_target);
        let normalized_owned = canonicalize_or_lexical(owned_root);
        if !normalized_target.starts_with(&normalized_owned) {
            bail!(
                "Refusing to replace '{}': it is a symlink pointing to '{}', outside inro's \
                 package directory. Remove it manually or change `bin_dir` in your config.",
                link.display(),
                raw_target.display()
            );
        }
    } else if link.exists() {
        bail!(
            "Refusing to overwrite '{}': it is not a symlink managed by inro. Remove it \
             manually or change `bin_dir` in your config.",
            link.display()
        );
    }
    Ok(())
}

/// Whether `link` is a symlink whose target resolves under `owned_root`.
/// Returns `false` for anything that is not a symlink (regular file,
/// directory, missing entry) or for symlinks pointing outside
/// `owned_root`. Use this to decide whether inro is allowed to remove an
/// entry it once linked, without risking deletion of a file the user
/// later replaced by hand.
pub fn is_inro_managed_symlink(link: &Path, owned_root: &Path) -> bool {
    if !link.is_symlink() {
        return false;
    }
    let Ok(raw_target) = fs::read_link(link) else {
        return false;
    };
    let abs_target = if raw_target.is_absolute() {
        raw_target
    } else {
        link.parent().unwrap_or(Path::new(".")).join(&raw_target)
    };
    let target_norm = canonicalize_or_lexical(&abs_target);
    let owned_norm = canonicalize_or_lexical(owned_root);
    target_norm.starts_with(&owned_norm)
}

pub fn symlink_points_to(link: &Path, expected: &Path) -> bool {
    let Ok(raw_target) = fs::read_link(link) else {
        return false;
    };
    let target = if raw_target.is_absolute() {
        raw_target
    } else {
        link.parent().unwrap_or(Path::new(".")).join(raw_target)
    };
    canonicalize_or_lexical(&target) == canonicalize_or_lexical(expected)
}

/// Resolve `p` to an absolute, normalized form: prefers `fs::canonicalize`
/// (which follows existing symlinks), falling back to a purely lexical
/// normalization for paths whose target does not exist (e.g. broken
/// symlinks).
fn canonicalize_or_lexical(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| normalize_lexical(p))
}

fn normalize_lexical(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Sanitize version string to be filesystem-safe.
pub fn sanitize_version(raw_version: &str) -> String { raw_version.replace(['/', '\\', ':'], "-") }

pub fn validate_path_component(value: &str, label: &str) -> Result<()> {
    use std::path::Component;

    let mut components = Path::new(value).components();
    let is_single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', ':', '\0'])
        || !is_single_normal
    {
        bail!("Invalid {label} '{value}': expected a single file name");
    }
    Ok(())
}

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
    terminal_link_with_support(text, url, terminal_supports_hyperlinks())
}

fn terminal_link_with_support(text: &str, url: &str, supported: bool) -> String {
    if supported { format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\") } else { text.to_string() }
}

fn terminal_supports_hyperlinks() -> bool {
    terminal_supports_hyperlinks_with(io::stdout().is_terminal(), |name| std::env::var(name).ok())
}

fn terminal_supports_hyperlinks_with<F>(is_terminal: bool, env: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = env("FORCE_HYPERLINK") {
        return value.trim() != "0";
    }
    if !is_terminal {
        return false;
    }

    // tmux masks the outer terminal identity, but preserves OSC 8 when the
    // client terminal advertises the `hyperlinks` feature.
    env("TMUX").is_some()
        || env("DOMTERM").is_some()
        || env("VTE_VERSION")
            .and_then(|version| version.parse::<u32>().ok())
            .is_some_and(|version| version >= 5000)
        || matches!(
            env("TERM_PROGRAM").as_deref(),
            Some(
                "Hyper"
                    | "iTerm.app"
                    | "terminology"
                    | "WezTerm"
                    | "vscode"
                    | "ghostty"
                    | "zed"
                    | "tmux"
            )
        )
        || matches!(env("TERM").as_deref(), Some("xterm-kitty" | "alacritty" | "alacritty-direct"))
        || matches!(env("COLORTERM").as_deref(), Some("xfce4-terminal"))
        || env("WT_SESSION").is_some()
        || env("KONSOLE_VERSION").is_some()
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

    fn hyperlinks_supported(is_terminal: bool, vars: &[(&str, &str)]) -> bool {
        terminal_supports_hyperlinks_with(is_terminal, |name| {
            vars.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_string())
        })
    }

    #[test]
    fn hyperlink_force_setting_overrides_terminal_detection() {
        assert!(hyperlinks_supported(false, &[("FORCE_HYPERLINK", "1")]));
        assert!(!hyperlinks_supported(true, &[("FORCE_HYPERLINK", " 0 "), ("TMUX", "/tmp/tmux")]));
    }

    #[test]
    fn hyperlinks_are_disabled_for_redirected_output() {
        assert!(!hyperlinks_supported(false, &[("TERM_PROGRAM", "ghostty")]));
    }

    #[test]
    fn hyperlinks_detect_direct_terminals() {
        assert!(hyperlinks_supported(true, &[("TERM_PROGRAM", "ghostty")]));
        assert!(hyperlinks_supported(true, &[("WT_SESSION", "session-id")]));
        assert!(!hyperlinks_supported(true, &[("TERM_PROGRAM", "unknown")]));
    }

    #[test]
    fn hyperlinks_detect_tmux() {
        assert!(hyperlinks_supported(true, &[("TMUX", "/tmp/tmux-501/default,1,0")]));
        assert!(hyperlinks_supported(true, &[("TERM_PROGRAM", "tmux")]));
    }

    #[test]
    fn terminal_link_emits_osc_8_only_when_supported() {
        let expected = "\x1b]8;;https://example.com/v1\x1b\\v1\x1b]8;;\x1b\\";

        assert_eq!(terminal_link_with_support("v1", "https://example.com/v1", true), expected);
        assert_eq!(terminal_link_with_support("v1", "https://example.com/v1", false), "v1");
    }

    #[test]
    fn duplicate_package_versions_are_rejected() {
        let args = vec!["tool@1.0.0".to_string(), "tool@2.0.0".to_string()];

        let error = ensure_unique_package_args(&args).unwrap_err();

        assert!(error.to_string().contains("tool"));
        assert!(error.to_string().contains("more than once"));
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

        // A same-name plain data file should not be treated as an installable
        // binary.
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
    fn find_binary_prefers_strongest_signal_over_traversal_order() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Three same-name candidates, all "likely binary" by at least one
        // signal but with different strengths. The real ELF + bin/ + exec
        // entry must win regardless of which one WalkDir yields first.
        let exec_only = root.join("a/libexec/tool"); // exec bit only (+20)
        let bin_no_magic = root.join("z/bin/tool"); // bin/ + exec  (+70)
        let real_elf = root.join("m/usr/bin/tool"); // magic + bin/ + exec (+170)
        for p in [&exec_only, &bin_no_magic, &real_elf] {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
        }
        fs::write(&exec_only, b"#!/bin/sh\necho hi\n").unwrap();
        fs::write(&bin_no_magic, b"# script\n").unwrap();
        fs::write(&real_elf, b"\x7fELF binary").unwrap();
        for p in [&exec_only, &bin_no_magic, &real_elf] {
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let result = find_binary_in_dir(root, "tool").unwrap();
        assert_eq!(result, real_elf);
    }

    #[cfg(unix)]
    #[test]
    fn find_binary_breaks_score_ties_by_shallower_path() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Two equally-strong candidates (both ELF magic + exec, neither in a
        // bin/ ancestor). The shallower path wins for determinism.
        let shallow = root.join("tool");
        let deep = root.join("nested/dir/tool");
        fs::create_dir_all(deep.parent().unwrap()).unwrap();
        for p in [&shallow, &deep] {
            fs::write(p, b"\x7fELF").unwrap();
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let result = find_binary_in_dir(root, "tool").unwrap();
        assert_eq!(result, shallow);
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

    #[test]
    fn validate_path_component_rejects_paths() {
        assert!(validate_path_component("ripgrep", "package name").is_ok());
        assert!(validate_path_component("rg.exe", "binary name").is_ok());

        for invalid in ["", ".", "..", "../outside", "dir/tool", "dir\\tool", "/tool"] {
            assert!(
                validate_path_component(invalid, "binary name").is_err(),
                "accepted unsafe component: {invalid:?}"
            );
        }
    }
}
