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
    let safe_version = sanitize_version(&candidate.version);
    let pkg_install_dir = layout.pkgs_dir.join(name).join(&safe_version);

    prepare_install_dir(&pkg_install_dir)?;

    // Download with progress
    progress.set_phase(OpPhase::Downloading);
    let temp_dir = TempDir::new().map_err(PkgError::Io)?;
    let downloaded_file = download_file_with_progress(
        &candidate.download_url,
        temp_dir.path(),
        candidate.size,
        progress,
    )
    .await?;

    // Extract
    progress.set_phase(OpPhase::Extracting);
    unpack_and_process(&downloaded_file, &pkg_install_dir, pkg)?;

    let binaries_result: Result<Vec<InstalledBin>, PkgError> = pkg
        .bin
        .iter()
        .map(|b| {
            let bin_path = find_binary_in_dir(&pkg_install_dir, &b.name)
                .ok_or_else(|| PkgError::BinaryNotFoundInArchive(b.name.clone()))?;
            Ok(InstalledBin { name: b.link.clone(), bin_path, link_path: PathBuf::new() })
        })
        .collect();
    let mut receipt = PkgReceipt {
        name: name.to_string(),
        version: candidate.version.clone(),
        remote: pkg.remote.clone(),
        installed_at: Utc::now(),
        install_dir: pkg_install_dir,
        binaries: binaries_result?,
    };
    receipt.relink(&config.bin_dir)?;

    Ok(receipt)
}

/// Prepare the installation directory by removing it if it exists and creating
/// a new one.
fn prepare_install_dir(dir: &Path) -> Result<(), PkgError> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    Ok(fs::create_dir_all(dir)?)
}

/// Unpack the downloaded file and perform post-processing like renaming and
/// flattening.
fn unpack_and_process(src_path: &Path, dst_dir: &Path, pkg: &ResolvedPkg) -> Result<(), PkgError> {
    let ft = extract_file(src_path, dst_dir).map_err(|e| PkgError::Extraction {
        filename: src_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        source: e,
    })?;

    // If asset is a single bin, rename it to the name of the package
    if let FileType::Pe | FileType::Elf | FileType::MachO = ft {
        rename_single_file(dst_dir, &pkg.bin[0].name)?;
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
