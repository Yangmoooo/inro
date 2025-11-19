use std::path::Path;

use anyhow::{Result, bail};
use figment::Figment;
use figment::providers::{Format, Toml};
use tempfile::TempDir;

use super::CommandHandler;
use crate::package::remotes::github::GitHubProvider;
use crate::package::remotes::{Remote, RemoteProvider};
use crate::package::{PackageConfig, PackageError, PackgeReceipt};
use crate::report;
use crate::utils::*;

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

        let install_dir = dirs::home_dir().expect("No home dir").join("bin");

        let pkgconf_file = "sources.default.toml";
        let pkgconf: PackageConfig = Figment::new().merge(Toml::file(pkgconf_file)).extract()?;
        report!(MsgType::Info, "Loaded package config!\n{:#?}", pkgconf.pkgs);

        let mut success_receipts = Vec::new();
        let mut failed_packages = Vec::new();

        for name in &names {
            match do_install(name, &pkgconf, &install_dir) {
                Ok(receipt) => success_receipts.push(receipt),
                Err(e) => {
                    report!(MsgType::Error, "Failed to install '{name}': {e:?}");
                    failed_packages.push(name.clone());
                }
            }
        }

        if !success_receipts.is_empty() {
            let success_packages: Vec<_> = success_receipts.into_iter().map(|r| r.name).collect();
            report!(
                MsgType::Info,
                "Installed {} package(s): {success_packages:?}",
                success_packages.len()
            )
        }

        if !failed_packages.is_empty() {
            report!(
                MsgType::Error,
                "\nDone with errors. Failed to install: {failed_packages:?}"
            );
            bail!("One or more packages failed to install.");
        } else {
            report!(MsgType::Success, "\nAll packages installed successfully.")
        }

        Ok(())
    }
}

fn do_install(
    name: &str,
    config: &PackageConfig,
    install_dir: &Path,
) -> Result<PackgeReceipt, PackageError> {
    report!(MsgType::Step, "Processing package '{name}'...");

    let pkg_def = config
        .pkgs
        .get(name)
        .ok_or(PackageError::NotFound(name.to_string()))?;

    let provider: Box<dyn RemoteProvider> = match &pkg_def.remote {
        Remote::GitHub(_) => {
            let gh_provider = GitHubProvider::new()?;
            Box::new(gh_provider)
        } // Remote::Direct => ...
    };

    report!(MsgType::Detail, "Fetching candidates from source...");

    let candidates = provider.find_candidates(pkg_def)?;
    let candidate = candidates
        .first()
        .expect("Remote provider violated contract: returned empty candidate list");

    report!(
        MsgType::Detail,
        "Selected candidate: {} ({})",
        candidate.asset_name,
        candidate.version
    );

    let temp_dir = TempDir::new().map_err(PackageError::Io)?;
    let temp_path = temp_dir.path();

    report!(
        MsgType::Detail,
        "Downloading from {}...",
        candidate.download_url
    );

    let downloaded_file = download_file(&candidate.download_url, temp_path)?;

    report!(
        MsgType::Detail,
        "Trying to extract file: {:?}...",
        downloaded_file.file_name()
    );

    // TODO: extract the archive

    report!(MsgType::Detail, "Installing to {:?}...", install_dir);

    // TODO: install the bins

    Ok(PackgeReceipt {
        name: name.to_string(),
        ver: candidate.version.clone(),
        bins: vec![name.to_string()],
    })
}
