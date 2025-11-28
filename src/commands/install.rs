use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use chrono::Utc;
use colored::Colorize;
use tempfile::TempDir;
use walkdir::WalkDir;

use super::CommandHandler;
use crate::config::Config;
use crate::dan::{DanError, DanReceipt, InstalledBinary};
use crate::layout::InroLayout;
use crate::registry::Registry;
use crate::remotes::github::GitHubProvider;
use crate::remotes::{self, RemoteProvider, RemoteType};
use crate::report;
use crate::utils::{download_file, extract_file};

pub struct InstallCommand {
    pub names: Vec<String>,
}

impl CommandHandler for InstallCommand {
    fn handle(&self) -> Result<()> {
        let names = super::unique(&self.names);
        report!(
            MsgType::Info,
            "Starting installation of {} package(s)...",
            names.len()
        );

        let layout = InroLayout::new()?;
        let config = Config::load(&layout)?;
        report!(MsgType::Detail, "Loaded inro config.");
        let registry = Registry::load(&layout)?;
        report!(MsgType::Detail, "Loaded sources.");

        let mut successes = Vec::new();
        let mut failures = Vec::new();

        for name in &names {
            match do_install(name, &registry, &config, &layout) {
                Ok(receipt) => successes.push(receipt),
                Err(e) => {
                    report!(MsgType::Error, "Failed to install '{name}': {e:?}");
                    failures.push((name.clone(), e.to_string()));
                }
            }
        }

        eprintln!();
        let has_success = !successes.is_empty();
        let has_failure = !failures.is_empty();

        if has_success {
            report!(
                MsgType::Success,
                "Successfully installed {} package(s):",
                successes.len()
            );

            let max_name_len = successes.iter().map(|r| r.name.len()).max().unwrap_or(0);

            for receipt in &successes {
                let bin_name = if receipt.binaries.len() == 1 {
                    format!("(bin: {})", receipt.binaries[0].name)
                } else {
                    format!(
                        "(bins: {})",
                        receipt
                            .binaries
                            .iter()
                            .map(|b| b.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                eprintln!(
                    "  {} {:<width$} {} {}",
                    "+".green(),
                    receipt.name.bold(),
                    receipt.version.italic(),
                    bin_name.dimmed(),
                    width = max_name_len
                );
            }
        }

        if has_failure {
            if !successes.is_empty() {
                eprintln!();
            }

            report!(
                MsgType::Error,
                "Failed to install {} package(s):",
                failures.len()
            );

            let max_name_len = failures
                .iter()
                .map(|(name, _)| name.len())
                .max()
                .unwrap_or(0);
            for (name, reason) in &failures {
                eprintln!(
                    "  {} {:width$} : {}",
                    "•".red(),
                    name.bold(),
                    reason,
                    width = max_name_len
                );
            }
        }

        if !has_success && !has_failure {
            report!(MsgType::Warning, "Nothing was installed.");
            return Ok(());
        }

        if has_failure {
            std::process::exit(1);
        }

        Ok(())
    }
}

fn do_install(
    name: &str,
    registry: &Registry,
    config: &Config,
    layout: &InroLayout,
) -> Result<DanReceipt, DanError> {
    report!(MsgType::Step, "Processing package '{name}'...");

    // 1. get package definition
    let dan_info = registry
        .dans
        .get(name)
        .ok_or(DanError::NotFound(name.to_string()))?;
    let dan_info = dan_info.clone().resolve(name);

    // 2. initialize remote provider
    let provider: Box<dyn RemoteProvider> = match &dan_info.remote {
        RemoteType::GitHub(_) => {
            let gh_provider = GitHubProvider::new().map_err(remotes::Error::GitHub)?;
            Box::new(gh_provider)
        } // Remote::Direct => ...
    };

    // 3. find asset candidates
    report!(MsgType::Detail, "Fetching candidates from source...");
    let candidates = provider.find_candidates(&dan_info)?;
    let candidate = candidates
        .first()
        // handled in remotes::github::Release::find_assets with NoMatchingAsset
        .expect("Remote provider violated contract: returned empty candidate list");
    report!(
        MsgType::Detail,
        "Selected candidate: {} ({})",
        candidate.asset_name,
        candidate.version
    );

    // 4. download to temp dir
    let temp_dir = TempDir::new().map_err(DanError::Io)?;
    report!(
        MsgType::Detail,
        "Downloading from {}...",
        candidate.download_url
    );
    let downloaded_file = download_file(&candidate.download_url, temp_dir.path())?;

    // 5. prepare install dir
    let dan_install_dir = layout.dans_dir.join(name).join(&candidate.version);

    if dan_install_dir.exists() {
        report!(MsgType::Warning, "Package already installed. Removing...");
        fs::remove_dir_all(&dan_install_dir)?;
    }
    fs::create_dir_all(&dan_install_dir)?;

    // 6. extract the asset
    report!(
        MsgType::Detail,
        "Extracting file: {:?}...",
        downloaded_file
            .file_name()
            .ok_or(anyhow!("Invalid asset file name"))?
    );
    extract_file(&downloaded_file, &dan_install_dir).map_err(|e| DanError::Extraction {
        filename: candidate.asset_name.clone(),
        source: e,
    })?;
    report!(
        MsgType::Detail,
        "Installed to {}.",
        &dan_install_dir.display()
    );

    // 7. link the bins
    let bin_dir = config.bin_dir.clone();
    if !bin_dir.exists() {
        fs::create_dir_all(&bin_dir)?;
    }

    let mut installed_bins_info = Vec::new();

    for bin_info in dan_info.bin.iter() {
        // 7.1. find the binary in the install dir
        let src_path = find_binary_in_dir(&dan_install_dir, &bin_info.name)
            .ok_or_else(|| DanError::BinaryNotFoundInArchive(bin_info.name.clone()))?;
        // 7.2. construct the destination path
        let dst_path = bin_dir.join(bin_info.link.clone());
        // 7.3. create the symlink
        report!(
            MsgType::Detail,
            "Linking {} to {}...",
            src_path.display(),
            dst_path.display()
        );
        create_symlink(&src_path, &dst_path)?;

        installed_bins_info.push(InstalledBinary {
            name: bin_info.name.clone(),
            source_path: src_path,
            link_path: dst_path,
        });
    }

    Ok(DanReceipt {
        name: name.to_string(),
        version: candidate.version.clone(),
        remote_type: dan_info.remote,
        installed_at: Utc::now(),
        install_dir: dan_install_dir,
        binaries: installed_bins_info,
    })
}

fn find_binary_in_dir(root: &Path, bin_name: &str) -> Option<PathBuf> {
    let walker = WalkDir::new(root).into_iter();

    for entry in walker.filter_map(|e| e.ok()) {
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

fn create_symlink(original: &Path, link: &Path) -> Result<(), DanError> {
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
            Ok(_) => {}
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
