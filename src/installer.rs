use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use tempfile::TempDir;

use crate::config::Config;
use crate::dan::{DanDef, DanError, DanReceipt, InstalledBinary, ResolvedDan};
use crate::layout::InroLayout;
use crate::remotes::{InstallCandidate, create_provider};
use crate::report;
use crate::utils::*;

pub fn find_best_candidate(dan_def: &DanDef) -> Result<InstallCandidate, DanError> {
    let provider = create_provider(&dan_def.remote)?;

    report!(MsgType::Detail, "Fetching candidates from remote...");
    let candidates = provider.find_candidates(dan_def)?;
    let candidate = candidates
        // just take the first one for now
        .first()
        // handled in remotes::github::Release::find_assets with NoMatchingAsset
        .expect("Remote provider violated contract: returned empty candidate list");
    report!(
        MsgType::Detail,
        "Selected candidate: {} ({})",
        candidate.asset_name,
        candidate.version
    );
    Ok(candidate.to_owned())
}

pub fn install_candidate(
    name: &str,
    candidate: &InstallCandidate,
    dan: &ResolvedDan,
    config: &Config,
    layout: &InroLayout,
) -> Result<DanReceipt, DanError> {
    let safe_version = sanitize_version(&candidate.version);
    let dan_install_dir = layout.dans_dir.join(name).join(safe_version);

    prepare_install_dir(&dan_install_dir)?;

    let temp_dir = TempDir::new().map_err(DanError::Io)?;
    let downloaded_file = download_file(&candidate.download_url, temp_dir.path())?;

    unpack_and_process(&downloaded_file, &dan_install_dir, dan)?;

    let binaries_result: Result<Vec<InstalledBinary>, DanError> = dan
        .bin
        .iter()
        .map(|b| {
            let bin_path = find_binary_in_dir(&dan_install_dir, &b.name)
                .ok_or_else(|| DanError::BinaryNotFoundInArchive(b.name.clone()))?;
            Ok(InstalledBinary {
                name: b.link.clone(),
                bin_path,
                link_path: PathBuf::new(),
            })
        })
        .collect();
    let mut receipt = DanReceipt {
        name: name.to_string(),
        version: candidate.version.clone(),
        remote: dan.remote.clone(),
        installed_at: Utc::now(),
        install_dir: dan_install_dir,
        binaries: binaries_result?,
    };
    receipt.relink(&config.bin_dir)?;

    Ok(receipt)

}

fn prepare_install_dir(dir: &Path) -> Result<(), DanError> {
    if dir.exists() {
        report!(MsgType::Warning, "Package already installed. Removing...");
        fs::remove_dir_all(dir)?;
    }
    Ok(fs::create_dir_all(dir)?)
}

fn unpack_and_process(src_path: &Path, dst_dir: &Path, dan: &ResolvedDan) -> Result<(), DanError> {
    let ft = extract_file(src_path, dst_dir).map_err(|e| DanError::Extraction {
        filename: src_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        source: e,
    })?;

    // if asset is a single bin, rename it to the name of the package
    if let FileType::Pe | FileType::Elf = ft {
        rename_single_file(dst_dir, &dan.bin[0].name)?;
    }

    // if there is only one directory, flatten it
    if let Err(e) = flatten_single_directory(dst_dir) {
        report!(
            MsgType::Warning,
            "Failed to flatten directory structure: {e}"
        );
    }

    report!(MsgType::Detail, "Installed to {}", dst_dir.display());
    Ok(())
}
