use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use tempfile::TempDir;

use crate::config::Config;
use crate::layout::InroLayout;
use crate::package::{InstalledBin, PkgDef, PkgError, PkgReceipt, ResolvedPkg};
use crate::progress::{OpPhase, PkgProgress};
use crate::remotes::{InstallCandidate, create_provider};
use crate::utils::*;
use crate::warn;

/// Find the best installation candidate for the given package definition and
/// optional version.
pub async fn find_best_candidate(
    pkg_def: &PkgDef,
    ver: Option<&str>,
    progress: &PkgProgress,
) -> Result<InstallCandidate, PkgError> {
    progress.set_phase(OpPhase::Fetching);

    let provider = create_provider(&pkg_def.remote)?;
    let candidates = provider.find_candidates_async(pkg_def, ver).await?;
    candidates.into_iter().next().ok_or(PkgError::NoCandidates)
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
    if let FileType::Pe | FileType::Elf = ft {
        rename_single_file(dst_dir, &pkg.bin[0].name)?;
    }

    // If there is only one directory, flatten it
    if let Err(e) = flatten_single_directory(dst_dir) {
        warn!("Failed to flatten single directory: {e}");
    }

    Ok(())
}
