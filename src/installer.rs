use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use chrono::Utc;
use dialoguer::Select;
use humansize::{BINARY, format_size};
use tempfile::TempDir;

use crate::config::Config;
use crate::layout::InroLayout;
use crate::package::{InstalledBin, PkgDef, PkgError, PkgReceipt, ResolvedPkg};
use crate::platform::PlatformInfo;
use crate::progress::{OpPhase, PkgProgress};
use crate::registry::AssetSelectionWriteBack;
use crate::remotes::{CandidateResult, InstallCandidate, MatchKind, create_provider};
use crate::utils::*;
use crate::warn;

/// Find all installation candidates for the given package definition and
/// optional version.
pub async fn find_candidates(
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
pub struct AssetSelection {
    pub candidate: InstallCandidate,
    pub write_back: Option<AssetSelectionWriteBack>,
}

/// Select a candidate from the result, prompting interactively if needed.
pub fn select_candidate(
    pkg_name: &str,
    result: CandidateResult,
) -> Result<AssetSelection, PkgError> {
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
pub async fn install_candidate(
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

    // Take ownership of the staging path, dismissing TempDir's auto-cleanup
    // so it survives the rename below. From here on, any error before the
    // rename completes must remove the staging dir explicitly.
    let staging_path = staging_dir.keep();
    let receipt = PkgReceipt {
        name: name.to_string(),
        version: candidate.version.clone(),
        remote: pkg.remote.clone(),
        installed_at: Utc::now(),
        install_subdir,
        binaries,
    };
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
    use super::*;
    use crate::package::ResolvedBin;
    use crate::remotes::{GitHubAssetDef, RemoteType};

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

    #[test]
    fn unpack_and_process_with_empty_bin_does_not_panic() {
        // When PlatformAwareString filters out every binary, `pkg.bin` is empty.
        // Extracting a standalone binary asset must NOT panic on `pkg.bin[0]`.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("standalone-binary");
        let dst = tmp.path().join("out");
        let pkg = ResolvedPkg {
            ver: Some("v1.0.0".to_string()),
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
            ver: Some("v1.0.0".to_string()),
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
