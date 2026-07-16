use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use chrono::Utc;
use dialoguer::Select;
use futures::stream::{self, StreamExt};
use humansize::{BINARY, format_size};
use tempfile::TempDir;

use crate::archive::{FileType, extract_file};
use crate::config::Config;
use crate::layout::InroLayout;
use crate::package::{InstalledBin, PkgDef, PkgError, PkgReceipt, ResolvedPkg};
use crate::platform::PlatformInfo;
use crate::progress::{OpPhase, PkgProgress, ProgressManager};
use crate::registry::{AssetSelectionWriteBack, Registry};
use crate::remotes::{
    CandidateResult, InstallCandidate, MatchKind, create_provider, derive_asset_selector,
    derive_asset_selector_from_assets,
};
use crate::reporter::print_error_chain;
use crate::utils::*;
use crate::warn;

pub(crate) struct InstallRequest {
    name: String,
    version: Option<String>,
    current_version: Option<String>,
}

impl InstallRequest {
    pub(crate) fn install(name: String, version: Option<String>) -> Self {
        Self { name, version, current_version: None }
    }

    pub(crate) fn update(name: String, version: Option<String>, current_version: String) -> Self {
        Self { name, version, current_version: Some(current_version) }
    }
}

pub(crate) struct BatchOutcome {
    pub(crate) receipts: Vec<PkgReceipt>,
    pub(crate) write_backs: Vec<AssetSelectionWriteBack>,
    pub(crate) unchanged: usize,
    pub(crate) failed: usize,
}

struct BatchTask {
    name: String,
    version: Option<String>,
    current_version: Option<String>,
    progress: PkgProgress,
}

pub(crate) fn execute_install_batch(
    requests: Vec<InstallRequest>,
    progress: &ProgressManager,
    registry: &Registry,
    config: &Config,
    layout: &InroLayout,
) -> std::io::Result<BatchOutcome> {
    let mut tasks = Vec::new();
    let mut unchanged = 0usize;
    let mut failed = 0usize;

    for request in requests {
        if registry.pkgs.contains_key(&request.name) {
            let package_progress = progress.add_package(&request.name);
            tasks.push(BatchTask {
                name: request.name,
                version: request.version,
                current_version: request.current_version,
                progress: package_progress,
            });
        } else {
            let error = PkgError::NotFound(request.name.clone());
            progress.add_package(&request.name).finish_error(&error.to_string());
            failed += 1;
        }
    }

    let runtime = tokio::runtime::Runtime::new()?;
    let parallel_limit = config.parallel_downloads;

    let fetch_results: Vec<(BatchTask, Result<CandidateResult, PkgError>)> =
        runtime.block_on(async {
            stream::iter(tasks)
                .map(|task| async move {
                    let result = match registry.pkgs.get(&task.name) {
                        Some(pkg_def) => {
                            find_candidates(pkg_def, task.version.as_deref(), &task.progress).await
                        }
                        None => Err(PkgError::NotFound(task.name.clone())),
                    };
                    (task, result)
                })
                .buffer_unordered(parallel_limit)
                .collect()
                .await
        });

    let mut install_tasks = Vec::new();
    for (task, result) in fetch_results {
        match result {
            Ok(candidate_result) => {
                let candidate_version =
                    candidate_result.candidates.first().map(|candidate| candidate.version.as_str());
                let target_version = task.version.as_deref().or(candidate_version);
                if let Some(current_version) = task.current_version.as_deref()
                    && target_version == Some(current_version)
                {
                    task.progress.finish_unchanged(current_version);
                    unchanged += 1;
                    continue;
                }

                let selection = progress.suspend(|| select_candidate(&task.name, candidate_result));
                match selection {
                    Ok(selection) => {
                        install_tasks.push((task, selection.candidate, selection.write_back));
                    }
                    Err(error) => {
                        task.progress.finish_error(&error.to_string());
                        print_error_chain(&error);
                        failed += 1;
                    }
                }
            }
            Err(error) => {
                task.progress.finish_error(&error.to_string());
                print_error_chain(&error);
                failed += 1;
            }
        }
    }

    let results: Vec<Option<(PkgReceipt, Option<AssetSelectionWriteBack>)>> =
        runtime.block_on(async {
            stream::iter(install_tasks)
                .map(|(task, candidate, write_back)| async move {
                    let Some(pkg_def) = registry.pkgs.get(&task.name) else {
                        task.progress.finish_error("not found in registry");
                        return None;
                    };
                    let pkg = pkg_def.resolve(&task.name);

                    match install_candidate(
                        &task.name,
                        &candidate,
                        &pkg,
                        config,
                        layout,
                        &task.progress,
                    )
                    .await
                    {
                        Ok(receipt) => {
                            task.progress.finish_success(&candidate.version);
                            Some((receipt, write_back))
                        }
                        Err(error) => {
                            task.progress.finish_error(&error.to_string());
                            print_error_chain(&error);
                            None
                        }
                    }
                })
                .buffer_unordered(parallel_limit)
                .collect()
                .await
        });

    let mut receipts = Vec::new();
    let mut write_backs = Vec::new();
    for result in results {
        match result {
            Some((receipt, write_back)) => {
                receipts.push(receipt);
                if let Some(write_back) = write_back {
                    write_backs.push(write_back);
                }
            }
            None => failed += 1,
        }
    }

    Ok(BatchOutcome { receipts, write_backs, unchanged, failed })
}

/// Find all installation candidates for the given package definition and
/// optional version.
async fn find_candidates(
    pkg_def: &PkgDef,
    ver: Option<&str>,
    progress: &PkgProgress,
) -> Result<CandidateResult, PkgError> {
    progress.set_phase(OpPhase::Fetching);

    let provider = create_provider(&pkg_def.remote)?;
    let result = provider.find_candidates_async(pkg_def, ver).await?;
    if result.candidates.is_empty() {
        return Err(PkgError::NoCandidates);
    }
    Ok(result)
}

/// Result of asset selection, with optional write-back info.
struct AssetSelection {
    candidate: InstallCandidate,
    write_back: Option<AssetSelectionWriteBack>,
}

/// Select a candidate from the result, prompting interactively if needed.
fn select_candidate(pkg_name: &str, result: CandidateResult) -> Result<AssetSelection, PkgError> {
    select_candidate_with_interactivity(
        pkg_name,
        result,
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
    )
}

fn select_candidate_with_interactivity(
    pkg_name: &str,
    result: CandidateResult,
    interactive: bool,
) -> Result<AssetSelection, PkgError> {
    let platform_key = PlatformInfo::current().key();

    // Explicit config with a unique match: auto-select, no write-back needed.
    if result.match_kind == MatchKind::Explicit && result.candidates.len() == 1 {
        let candidate = result.candidates.into_iter().next().ok_or(PkgError::NoCandidates)?;
        return Ok(AssetSelection { candidate, write_back: None });
    }

    // Heuristic with single candidate: auto-select, but don't write back.
    // Only explicit user choices should become persistent local config.
    if result.match_kind == MatchKind::PlatformHeuristic && result.candidates.len() == 1 {
        let candidate = result.candidates.into_iter().next().ok_or(PkgError::NoCandidates)?;
        return Ok(AssetSelection { candidate, write_back: None });
    }

    if matches!(result.match_kind, MatchKind::Fallback | MatchKind::Explicit)
        && result.candidates.len() > 1
        && !interactive
    {
        let reason = if result.match_kind == MatchKind::Explicit {
            format!(
                "Configured asset selector '{}' matched multiple assets",
                result.matched_selector.as_deref().unwrap_or("<unknown>")
            )
        } else {
            "Multiple fallback assets found".to_string()
        };
        return Err(PkgError::Other(format!(
            "{reason}; run in an interactive terminal or configure a more specific asset \
                 selector"
        )));
    }

    // Heuristic with multiple candidates, or fallback in non-interactive mode with
    // one candidate.
    if !interactive {
        // Non-interactive: auto-select first (highest score)
        let candidate = result.candidates.into_iter().next().ok_or(PkgError::NoCandidates)?;
        return Ok(AssetSelection { candidate, write_back: None });
    }

    // Interactive: prompt user to select
    let prompt = match result.match_kind {
        MatchKind::Explicit => format!(
            "Configured asset selector '{}' matched multiple assets for '{pkg_name}' ({platform_key}). Select one",
            result.matched_selector.as_deref().unwrap_or("<unknown>")
        ),
        MatchKind::Fallback => {
            format!(
                "No platform-specific asset found for '{pkg_name}' ({platform_key}). Select one"
            )
        }
        MatchKind::PlatformHeuristic => {
            format!("Multiple assets found for '{pkg_name}' ({platform_key}). Select one")
        }
    };

    let mut items: Vec<String> = result
        .candidates
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let recommended = if idx == 0 { " recommended" } else { "" };
            format!("{}  ({}){recommended}", c.asset_name, format_size(c.size, BINARY))
        })
        .collect();
    if result.match_kind == MatchKind::Fallback {
        items.push("Cancel".to_string());
    }

    eprintln!();
    let selection = Select::new()
        .with_prompt(prompt)
        .items(&items)
        .default(0)
        .interact()
        .map_err(|e| PkgError::Other(e.to_string()))?;

    if result.match_kind == MatchKind::Fallback && selection == result.candidates.len() {
        return Err(PkgError::Other("Asset selection cancelled".to_string()));
    }

    let candidate = result.candidates.into_iter().nth(selection).ok_or(PkgError::NoCandidates)?;
    let selector = if result.asset_names.is_empty() {
        derive_asset_selector(&candidate.asset_name, &candidate.version)
    } else {
        derive_asset_selector_from_assets(
            &candidate.asset_name,
            &candidate.version,
            &result.asset_names,
        )
    };

    Ok(AssetSelection {
        write_back: Some(AssetSelectionWriteBack {
            pkg_name: pkg_name.to_string(),
            platform_key,
            selector,
        }),
        candidate,
    })
}

/// Install the given candidate for the package, returning a PkgReceipt on
/// success.
async fn install_candidate(
    name: &str,
    candidate: &InstallCandidate,
    pkg: &ResolvedPkg,
    config: &Config,
    layout: &InroLayout,
    progress: &PkgProgress,
) -> Result<PkgReceipt, PkgError> {
    validate_path_component(name, "package name")
        .map_err(|error| PkgError::Other(error.to_string()))?;
    if pkg.bin.is_empty() {
        return Err(PkgError::Other(format!(
            "Package '{name}' has no binary defined for the current platform ({}); check the \
             registry's [bin] entries",
            crate::platform::PlatformInfo::current().key()
        )));
    }

    let safe_version = sanitize_version(&candidate.version);
    validate_path_component(&safe_version, "version")
        .map_err(|error| PkgError::Other(error.to_string()))?;
    for bin in &pkg.bin {
        validate_path_component(&bin.name, "binary name")
            .map_err(|error| PkgError::Other(error.to_string()))?;
        validate_path_component(&bin.link, "link name")
            .map_err(|error| PkgError::Other(error.to_string()))?;
    }
    let install_subdir = PathBuf::from(name).join(&safe_version);
    let pkg_dir = layout.pkgs_dir.join(name);
    let final_install_dir = layout.pkgs_dir.join(&install_subdir);
    fs::create_dir_all(&pkg_dir).map_err(PkgError::Io)?;

    // Stage all work in a sibling directory on the same filesystem. Any
    // failure before the final rename leaves the existing installation
    // (if any) untouched and removes the half-built staging dir on drop.
    let staging_dir = tempfile::Builder::new()
        .prefix(&format!("{safe_version}.staging."))
        .rand_bytes(8)
        .tempdir_in(&pkg_dir)
        .map_err(PkgError::Io)?;

    progress.set_phase(OpPhase::Downloading);
    let download_dir = TempDir::new().map_err(PkgError::Io)?;
    let downloaded_file = download_file_with_progress(
        &candidate.download_url,
        download_dir.path(),
        candidate.size,
        progress,
    )
    .await?;

    progress.set_phase(OpPhase::Extracting);
    unpack_and_process(&downloaded_file, staging_dir.path(), pkg)?;

    // Capture each binary's subpath relative to the staging tree so the
    // receipt stays portable across $INRO_HOME changes.
    let binaries: Vec<InstalledBin> = pkg
        .bin
        .iter()
        .map(|b| {
            let staged_bin = find_binary_in_dir(staging_dir.path(), &b.name)
                .ok_or_else(|| PkgError::BinaryNotFoundInArchive(b.name.clone()))?;
            let bin_subpath = staged_bin
                .strip_prefix(staging_dir.path())
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(&b.name));
            Ok(InstalledBin { name: b.link.clone(), bin_subpath })
        })
        .collect::<Result<_, PkgError>>()?;

    let receipt = PkgReceipt {
        name: name.to_string(),
        version: candidate.version.clone(),
        remote: pkg.remote.clone(),
        installed_at: Utc::now(),
        install_subdir,
        binaries,
    };
    receipt.save_to_dir(staging_dir.path()).map_err(|source| PkgError::Receipt {
        name: name.to_string(),
        version: candidate.version.clone(),
        source,
    })?;

    // Take ownership of the complete staging path, including its receipt,
    // so it survives the rename below. From here on, any error before the
    // rename completes must remove the staging dir explicitly.
    let staging_path = staging_dir.keep();
    promote_and_relink_install(
        &staging_path,
        &final_install_dir,
        &receipt,
        &config.bin_dir,
        &layout.pkgs_dir,
    )?;

    Ok(receipt)
}

fn promote_and_relink_install(
    staging: &Path,
    final_dir: &Path,
    receipt: &PkgReceipt,
    bin_dir: &Path,
    pkgs_dir: &Path,
) -> Result<(), PkgError> {
    let backup = match promote_install_dir(staging, final_dir) {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_dir_all(staging);
            return Err(error);
        }
    };

    if let Err(link_error) = receipt.relink(bin_dir, pkgs_dir) {
        if let Err(remove_error) = fs::remove_dir_all(final_dir) {
            return Err(PkgError::Other(format!(
                "{link_error}; additionally failed to remove incomplete install '{}': \
                 {remove_error}",
                final_dir.display()
            )));
        }
        if let Some(backup_path) = backup
            && let Err(restore_error) = fs::rename(&backup_path, final_dir)
        {
            return Err(PkgError::Other(format!(
                "{link_error}; additionally failed to restore previous install '{}': \
                 {restore_error}",
                final_dir.display()
            )));
        }
        return Err(PkgError::Other(link_error.to_string()));
    }

    if let Some(backup_path) = backup {
        // Best-effort: a leftover backup dir is harmless; `clean` can sweep it.
        let _ = fs::remove_dir_all(backup_path);
    }
    Ok(())
}

/// Atomically move `staging` into `final_dir`. If `final_dir` already
/// exists, swap it aside to a sibling backup directory first so the rename
/// can succeed on platforms where it cannot replace a non-empty directory.
///
/// On success returns `Some(backup_path)` if a previous installation was
/// swapped aside (so the caller can drop it), or `None` otherwise. On
/// failure the previous installation, if any, is restored and the original
/// error is returned; the caller is still responsible for removing
/// `staging`.
fn promote_install_dir(staging: &Path, final_dir: &Path) -> Result<Option<PathBuf>, PkgError> {
    let backup = if final_dir.exists() {
        let parent = final_dir.parent().ok_or_else(|| {
            PkgError::Other(format!(
                "Cannot determine parent of install dir '{}'",
                final_dir.display()
            ))
        })?;
        let placeholder = tempfile::Builder::new()
            .prefix(&format!(
                "{}.backup.",
                final_dir.file_name().unwrap_or_default().to_string_lossy()
            ))
            .rand_bytes(8)
            .tempdir_in(parent)
            .map_err(PkgError::Io)?;
        // Take the placeholder's path and remove it so `rename` can take its
        // place. Dropping the TempDir directly would race with the rename.
        let backup_path = placeholder.keep();
        fs::remove_dir_all(&backup_path).map_err(PkgError::Io)?;
        fs::rename(final_dir, &backup_path).map_err(PkgError::Io)?;
        Some(backup_path)
    } else {
        None
    };

    if let Err(e) = fs::rename(staging, final_dir) {
        if let Some(ref backup_path) = backup {
            // Restore the previous install. If this restore itself fails the
            // user is left with the backup dir on disk, which `clean` will
            // pick up; we still surface the original rename error.
            let _ = fs::rename(backup_path, final_dir);
        }
        return Err(PkgError::Io(e));
    }

    Ok(backup)
}

/// Unpack the downloaded file and perform post-processing like renaming and
/// flattening.
fn unpack_and_process(src_path: &Path, dst_dir: &Path, pkg: &ResolvedPkg) -> Result<(), PkgError> {
    let ft = extract_file(src_path, dst_dir).map_err(|e| PkgError::Extraction {
        filename: src_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        source: e,
    })?;

    // If asset is a single bin, rename it to the name of the package
    if let FileType::Pe | FileType::Elf | FileType::MachO = ft
        && let Some(first_bin) = pkg.bin.first()
    {
        rename_single_file(dst_dir, &first_bin.name)?;
    }

    // If there is only one directory, flatten it
    if let Err(e) = flatten_single_directory(dst_dir) {
        warn!("Failed to flatten single directory: {e}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::io::{ErrorKind, Read, Write};
    #[cfg(unix)]
    use std::net::TcpListener;
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Duration;

    use super::*;
    use crate::package::ResolvedBin;
    #[cfg(unix)]
    use crate::progress::ProgressManager;
    use crate::remotes::{GitHubAssetDef, RemoteType};

    #[cfg(unix)]
    fn test_layout(root: &Path) -> InroLayout {
        let inro_dir = root.join("inro");
        InroLayout {
            home_dir: root.to_path_buf(),
            config_path: inro_dir.join("config.toml"),
            manifest_path: inro_dir.join("manifest.json"),
            pkgs_dir: inro_dir.join("pkgs"),
            managed_registry_dir: inro_dir.join("registry"),
            user_registry_dir: inro_dir.join("registry.d"),
            inro_dir,
        }
    }

    #[cfg(unix)]
    fn test_config(root: &Path) -> Config {
        Config { bin_dir: root.join("bin"), upstreams: vec![], parallel_downloads: 1 }
    }

    #[cfg(unix)]
    fn serve_bytes(path: &str, body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut request = Vec::new();
                        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                            let mut chunk = [0u8; 1024];
                            let read = stream.read(&mut chunk).unwrap();
                            assert!(read > 0, "connection closed before request headers completed");
                            request.extend_from_slice(&chunk[..read]);
                            assert!(request.len() <= 16 * 1024, "request headers too large");
                        }
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(&body).unwrap();
                        stream.flush().unwrap();
                        break;
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            }
        });
        (format!("http://{address}/{path}"), handle)
    }

    #[cfg(unix)]
    fn package_with_tool() -> ResolvedPkg {
        ResolvedPkg {
            remote: RemoteType::default(),
            bin: vec![ResolvedBin { name: "tool".to_string(), link: "tool".to_string() }],
        }
    }

    #[cfg(unix)]
    fn tar_gz_with_reserved_receipt_dir() -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);

        let binary = b"\x7fELFtest";
        let mut binary_header = tar::Header::new_gnu();
        binary_header.set_path("tool").unwrap();
        binary_header.set_size(binary.len() as u64);
        binary_header.set_mode(0o755);
        binary_header.set_cksum();
        archive.append(&binary_header, &binary[..]).unwrap();

        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_path("inro-receipt.json").unwrap();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        archive.append(&dir_header, std::io::empty()).unwrap();

        archive.into_inner().unwrap().finish().unwrap()
    }

    fn candidate(asset_name: &str) -> InstallCandidate {
        InstallCandidate {
            version: "v1.0.0".to_string(),
            asset_name: asset_name.to_string(),
            download_url: format!("https://example.com/{asset_name}"),
            size: 1024,
        }
    }

    fn candidate_result(
        candidates: Vec<InstallCandidate>,
        match_kind: MatchKind,
    ) -> CandidateResult {
        let asset_names = candidates.iter().map(|candidate| candidate.asset_name.clone()).collect();
        CandidateResult {
            candidates,
            asset_names,
            match_kind,
            matched_selector: if match_kind == MatchKind::Explicit {
                Some("tool".to_string())
            } else {
                None
            },
        }
    }

    #[test]
    fn heuristic_single_candidate_does_not_write_back() {
        let result = candidate_result(
            vec![candidate("tool-v1.0.0-linux-x86_64.tar.gz")],
            MatchKind::PlatformHeuristic,
        );

        let selection = select_candidate("tool", result).unwrap();

        assert_eq!(selection.candidate.asset_name, "tool-v1.0.0-linux-x86_64.tar.gz");
        assert!(selection.write_back.is_none());
    }

    #[test]
    fn explicit_candidate_does_not_write_back() {
        let result = candidate_result(
            vec![candidate("tool-v1.0.0-linux-x86_64.tar.gz")],
            MatchKind::Explicit,
        );

        let selection = select_candidate("tool", result).unwrap();

        assert_eq!(selection.candidate.asset_name, "tool-v1.0.0-linux-x86_64.tar.gz");
        assert!(selection.write_back.is_none());
    }

    #[test]
    fn non_interactive_multiple_fallback_candidates_error() {
        let result = candidate_result(
            vec![candidate("tool.tar.gz"), candidate("tool.zip")],
            MatchKind::Fallback,
        );

        let error = match select_candidate_with_interactivity("tool", result, false) {
            Ok(_) => panic!("expected fallback selection to fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Multiple fallback assets found"));
    }

    #[test]
    fn non_interactive_multiple_explicit_candidates_error() {
        let result = candidate_result(
            vec![
                candidate("tool-v1.0.0-linux-x86_64.tar.gz"),
                candidate("tool-v1.0.0-linux-aarch64.tar.gz"),
            ],
            MatchKind::Explicit,
        );

        let error = match select_candidate_with_interactivity("tool", result, false) {
            Ok(_) => panic!("expected explicit multi-match selection to fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Configured asset selector"));
        assert!(error.to_string().contains("matched multiple assets"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_install_commits_receipt_with_final_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = test_layout(tmp.path());
        let config = test_config(tmp.path());
        let body = b"\x7fELFtest".to_vec();
        let (url, server) = serve_bytes("tool", body.clone());
        let candidate = InstallCandidate {
            version: "v1.0.0".to_string(),
            asset_name: "tool".to_string(),
            download_url: url,
            size: body.len() as u64,
        };
        let pkg = package_with_tool();
        let progress = ProgressManager::new(&["tool"]).add_package("tool");

        let receipt =
            install_candidate("tool", &candidate, &pkg, &config, &layout, &progress).await.unwrap();
        server.join().unwrap();

        let receipt_path = receipt.install_dir(&layout.pkgs_dir).join("inro-receipt.json");
        let saved: PkgReceipt = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(saved.name, "tool");
        assert_eq!(saved.version, "v1.0.0");
        assert_eq!(saved.install_subdir, PathBuf::from("tool/v1.0.0"));
        assert_eq!(saved.binaries.len(), 1);
        assert_eq!(saved.binaries[0].name, "tool");
        assert_eq!(saved.binaries[0].bin_subpath, PathBuf::from("tool"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn receipt_write_failure_keeps_existing_install_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = test_layout(tmp.path());
        let config = test_config(tmp.path());
        let final_dir = layout.pkgs_dir.join("tool/v1.0.0");
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("tool"), b"old").unwrap();

        let body = tar_gz_with_reserved_receipt_dir();
        let (url, server) = serve_bytes("tool.tar.gz", body.clone());
        let candidate = InstallCandidate {
            version: "v1.0.0".to_string(),
            asset_name: "tool.tar.gz".to_string(),
            download_url: url,
            size: body.len() as u64,
        };
        let pkg = package_with_tool();
        let progress = ProgressManager::new(&["tool"]).add_package("tool");

        let result = install_candidate("tool", &candidate, &pkg, &config, &layout, &progress).await;
        server.join().unwrap();

        let error = result.unwrap_err();
        assert_eq!(error.to_string(), "Failed to persist install receipt for 'tool@v1.0.0'");
        let causes = crate::reporter::format_error_chain(&error);
        assert_eq!(causes.first().map(String::as_str), Some("Failed to create install receipt"));
        assert!(causes.len() >= 2, "missing underlying OS error: {causes:?}");
        assert!(causes.iter().all(|cause| !cause.contains(".staging.")));
        assert_eq!(fs::read(final_dir.join("tool")).unwrap(), b"old");
        let leftovers: Vec<_> = fs::read_dir(final_dir.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".staging."))
            .collect();
        assert!(leftovers.is_empty(), "staging dirs leaked: {leftovers:?}");
    }

    #[test]
    fn unpack_and_process_with_empty_bin_does_not_panic() {
        // When PlatformAwareString filters out every binary, `pkg.bin` is empty.
        // Extracting a standalone binary asset must NOT panic on `pkg.bin[0]`.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("standalone-binary");
        let dst = tmp.path().join("out");
        let pkg = ResolvedPkg {
            remote: RemoteType::GitHub(GitHubAssetDef {
                repo: "test/empty".to_string(),
                asset: Default::default(),
            }),
            bin: vec![],
        };
        fs::write(&src, [0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0]).unwrap();
        fs::create_dir_all(&dst).unwrap();

        // Should complete without panic; the binary stays under its original name.
        unpack_and_process(&src, &dst, &pkg).unwrap();
        assert!(dst.join("standalone-binary").exists());
    }

    // ==================== promote_install_dir ====================

    #[test]
    fn promote_into_empty_parent_just_renames_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        let staging = pkg_dir.join("v1.0.0.staging.abc");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("rg"), b"new").unwrap();

        let final_dir = pkg_dir.join("v1.0.0");
        let backup = promote_install_dir(&staging, &final_dir).unwrap();

        assert!(backup.is_none());
        assert!(!staging.exists());
        assert_eq!(fs::read(final_dir.join("rg")).unwrap(), b"new");
    }

    #[test]
    fn promote_swaps_aside_existing_install() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        let final_dir = pkg_dir.join("v1.0.0");
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("rg"), b"old").unwrap();

        let staging = pkg_dir.join("v1.0.0.staging.abc");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("rg"), b"new").unwrap();

        let backup = promote_install_dir(&staging, &final_dir).unwrap();

        let backup_path = backup.expect("expected a backup of the previous install");
        assert!(backup_path.exists(), "backup must remain on disk for the caller to delete");
        assert_eq!(fs::read(backup_path.join("rg")).unwrap(), b"old");
        assert_eq!(fs::read(final_dir.join("rg")).unwrap(), b"new");
        assert!(!staging.exists());
    }

    #[test]
    fn install_failure_before_promote_keeps_existing_install_intact() {
        // Simulate the "extract succeeded into staging but binary not found"
        // scenario by promoting only after we artificially abort: we drop the
        // staging tempdir without calling promote, mirroring an early `?`
        // bail. The previous install at final_dir must be untouched.
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        let final_dir = pkg_dir.join("v1.0.0");
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("rg"), b"original").unwrap();

        {
            let staging_dir = tempfile::Builder::new()
                .prefix("v1.0.0.staging.")
                .rand_bytes(8)
                .tempdir_in(&pkg_dir)
                .unwrap();
            fs::write(staging_dir.path().join("rg-broken"), b"partial").unwrap();
            // staging_dir drops here without promote, simulating an error path.
        }

        // Existing install must still be intact.
        assert_eq!(fs::read(final_dir.join("rg")).unwrap(), b"original");
        // No staging residue should remain in the package dir.
        let leftovers: Vec<_> = fs::read_dir(&pkg_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".staging."))
            .collect();
        assert!(leftovers.is_empty(), "staging dirs leaked: {leftovers:?}");
    }

    #[test]
    fn relink_failure_after_promote_restores_existing_install() {
        let tmp = tempfile::tempdir().unwrap();
        let pkgs_dir = tmp.path().join("pkgs");
        let pkg_dir = pkgs_dir.join("tool");
        let final_dir = pkg_dir.join("v1.0.0");
        let staging = pkg_dir.join("v1.0.0.staging.abc");
        let bin_dir = tmp.path().join("bin");

        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("tool"), b"old").unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("tool"), b"new").unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("tool"), b"user-owned").unwrap();

        let receipt = PkgReceipt {
            name: "tool".to_string(),
            version: "v1.0.0".to_string(),
            remote: RemoteType::default(),
            installed_at: Utc::now(),
            install_subdir: PathBuf::from("tool").join("v1.0.0"),
            binaries: vec![InstalledBin {
                name: "tool".to_string(),
                bin_subpath: PathBuf::from("tool"),
            }],
        };

        let error = promote_and_relink_install(&staging, &final_dir, &receipt, &bin_dir, &pkgs_dir)
            .unwrap_err();

        assert!(error.to_string().contains("Refusing to overwrite"));
        assert_eq!(fs::read(final_dir.join("tool")).unwrap(), b"old");
        assert_eq!(fs::read(bin_dir.join("tool")).unwrap(), b"user-owned");
        assert!(!staging.exists());
    }

    #[test]
    fn unpack_and_process_renames_macho_binary_to_resolved_bin_name() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("chsrc-aarch64-macos");
        let dst = tmp.path().join("out");
        let pkg = ResolvedPkg {
            remote: RemoteType::GitHub(GitHubAssetDef {
                repo: "RubyMetric/chsrc".to_string(),
                asset: Default::default(),
            }),
            bin: vec![ResolvedBin { name: "chsrc".to_string(), link: "chsrc".to_string() }],
        };
        fs::write(&src, [0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0]).unwrap();
        fs::create_dir_all(&dst).unwrap();

        unpack_and_process(&src, &dst, &pkg).unwrap();

        assert!(dst.join("chsrc").exists());
        assert!(!dst.join("chsrc-aarch64-macos").exists());
    }
}
