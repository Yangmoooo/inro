use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::installer::{find_best_candidate, install_candidate};
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::PkgError;
use crate::registry::Registry;
use crate::report;
use crate::utils::unique;

pub struct InstallCommand {
    pub names: Vec<String>,
    pub layout: InroLayout,
}

impl CommandHandler for InstallCommand {
    fn handle(&self) -> Result<()> {
        let names = unique(&self.names);
        report!(MsgType::Info, "Starting installation of {} package(s)...", names.len());

        // prepare
        let config = Config::load(&self.layout)?;
        report!(MsgType::Detail, "Loaded inro config");
        let registry = Registry::load(&self.layout)?;
        if registry.pkgs.is_empty() {
            report!(
                MsgType::Warning,
                "Registry is empty. Run 'inro source update' to fetch packages"
            );
            return Ok(());
        }
        report!(MsgType::Detail, "Loaded inro registry");
        let mut manifest = Manifest::load(&self.layout.manifest_path)?;

        let mut successes = Vec::new();
        let mut failures = Vec::new();

        // install one by one
        for name in &names {
            let pkg_def = registry.pkgs.get(name).ok_or(PkgError::NotFound(name.to_string()))?;
            let candidate = find_best_candidate(pkg_def)?;
            let pkg = pkg_def.clone().resolve(name);

            match install_candidate(name, &candidate, &pkg, &config, &self.layout) {
                Ok(receipt) => {
                    if let Err(e) = receipt.save_to_install_dir() {
                        report!(MsgType::Warning, "Failed to save backup receipt: {e}");
                    }
                    manifest.add(receipt.clone());
                    successes.push(receipt);
                }
                Err(e) => {
                    report!(MsgType::Error, "Failed to install '{name}': {e}");
                    failures.push((name.clone(), e.to_string()));
                }
            }
        }

        // save manifest
        manifest.save(&self.layout.manifest_path)?;
        report!(MsgType::Detail, "Manifest updated");

        // summary
        eprintln!();
        let has_success = !successes.is_empty();
        let has_failure = !failures.is_empty();

        if has_success {
            report!(MsgType::Success, "Successfully installed {} package(s):", successes.len());

            let max_len = successes.iter().map(|r| r.name.len()).max().unwrap_or(0);

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
                    "  {} {:<max_len$} : {} {}",
                    "+".green(),
                    receipt.name.bold(),
                    receipt.version,
                    bin_name.dimmed(),
                );
            }
        }

        if has_failure {
            if !successes.is_empty() {
                eprintln!();
            }

            report!(MsgType::Error, "Failed to install {} package(s):", failures.len());

            let max_len = failures.iter().map(|t| t.0.len()).max().unwrap_or(0);
            for (name, err) in &failures {
                eprintln!("  {} {:<max_len$} : {}", "•".red(), name.bold(), err);
            }
        }

        if !has_success && !has_failure {
            report!(MsgType::Warning, "Nothing was installed");
            return Ok(());
        }

        if has_failure {
            std::process::exit(1);
        }

        Ok(())
    }
}
