use anyhow::Result;
use colored::Colorize;
use futures::stream::{self, StreamExt};

use super::CommandHandler;
use crate::config::Config;
use crate::installer::{find_best_candidate_async, install_candidate_async};
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::{PkgError, PkgReceipt};
use crate::registry::Registry;
use crate::remotes::InstallCandidate;
use crate::report;
use crate::utils::{parse_package_version, unique};

pub struct InstallCommand {
    /// Package names, optionally with version (e.g. "ripgrep@15.1.0")
    pub names: Vec<String>,
}

/// Intermediate struct for parallel processing
struct InstallTask {
    name: String,
    pkg_name: String,
    pkg_ver: Option<String>,
}

enum InstallResult {
    Success(PkgReceipt),
    Failure(String, String),
}

impl CommandHandler for InstallCommand {
    fn handle(&self) -> Result<()> {
        let names = unique(&self.names);
        report!(MsgType::Info, "Starting installation of {} package(s)...", names.len());

        // prepare
        let layout = InroLayout::new()?;
        let config = Config::load(&layout)?;
        report!(MsgType::Detail, "Loaded inro config");
        let registry = Registry::load(&layout)?;
        if registry.pkgs.is_empty() {
            report!(
                MsgType::Warning,
                "Registry is empty. Run 'inro source update' to fetch packages"
            );
            return Ok(());
        }
        report!(MsgType::Detail, "Loaded inro registry");
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        // Create tokio runtime for async operations
        let rt = tokio::runtime::Runtime::new()?;

        // Phase 1: Parallel fetch candidates
        let tasks: Vec<InstallTask> = names
            .iter()
            .map(|name| {
                let (pkg_name, pkg_ver) = parse_package_version(name);
                InstallTask {
                    name: name.clone(),
                    pkg_name: pkg_name.to_string(),
                    pkg_ver: pkg_ver.map(|s| s.to_string()),
                }
            })
            .collect();

        // Validate all packages exist in registry first
        let mut valid_tasks = Vec::new();
        let mut failures: Vec<(String, String)> = Vec::new();

        for task in tasks {
            match registry.pkgs.get(&task.pkg_name) {
                Some(_) => valid_tasks.push(task),
                None => {
                    let err = PkgError::NotFound(task.pkg_name.clone());
                    report!(MsgType::Error, "Failed to install '{}': {err}", task.name);
                    failures.push((task.name.clone(), err.to_string()));
                }
            }
        }

        // Phase 2: Parallel fetch candidates and download
        let parallel_limit = config.parallel_downloads;
        let results: Vec<InstallResult> = rt.block_on(async {
            stream::iter(valid_tasks)
                .map(|task| {
                    let registry = &registry;
                    let config = &config;
                    let layout = &layout;
                    async move {
                        let pkg_def = registry.pkgs.get(&task.pkg_name).unwrap();
                        let pkg = pkg_def.clone().resolve(&task.pkg_name);

                        // Fetch candidate (async network request)
                        let candidate: InstallCandidate =
                            match find_best_candidate_async(pkg_def, task.pkg_ver.as_deref()).await
                            {
                                Ok(c) => c,
                                Err(e) => {
                                    return InstallResult::Failure(
                                        task.name.clone(),
                                        e.to_string(),
                                    );
                                }
                            };

                        // Download and install (async download, sync extraction)
                        match install_candidate_async(
                            &task.pkg_name,
                            &candidate,
                            &pkg,
                            config,
                            layout,
                        )
                        .await
                        {
                            Ok(receipt) => InstallResult::Success(receipt),
                            Err(e) => InstallResult::Failure(task.name.clone(), e.to_string()),
                        }
                    }
                })
                .buffer_unordered(parallel_limit)
                .collect()
                .await
        });

        // Process results
        let mut successes = Vec::new();
        for result in results {
            match result {
                InstallResult::Success(receipt) => {
                    if let Err(e) = receipt.save_to_install_dir() {
                        report!(MsgType::Warning, "Failed to save backup receipt: {e}");
                    }
                    manifest.add(receipt.clone());
                    successes.push(receipt);
                }
                InstallResult::Failure(name, err) => {
                    report!(MsgType::Error, "Failed to install '{name}': {err}");
                    failures.push((name, err));
                }
            }
        }

        // save manifest
        manifest.save(&layout.manifest_path)?;
        report!(MsgType::Detail, "Manifest updated");

        // summary
        print_summary(&successes, &failures);

        if !failures.is_empty() {
            std::process::exit(1);
        }

        Ok(())
    }
}

fn print_summary(successes: &[PkgReceipt], failures: &[(String, String)]) {
    eprintln!();
    let has_success = !successes.is_empty();
    let has_failure = !failures.is_empty();

    if has_success {
        report!(MsgType::Success, "Successfully installed {} package(s):", successes.len());

        let max_len = successes.iter().map(|r| r.name.len()).max().unwrap_or(0);

        for receipt in successes {
            let bin_name = if receipt.binaries.len() == 1 {
                format!("(bin: {})", receipt.binaries[0].name)
            } else {
                format!(
                    "(bins: {})",
                    receipt.binaries.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")
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
        for (name, err) in failures {
            eprintln!("  {} {:<max_len$} : {}", "•".red(), name.bold(), err);
        }
    }

    if !has_success && !has_failure {
        report!(MsgType::Warning, "Nothing was installed");
    }
}
