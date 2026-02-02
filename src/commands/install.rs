use anyhow::Result;
use colored::Colorize;
use futures::stream::{self, StreamExt};

use super::CommandHandler;
use crate::config::Config;
use crate::installer::{find_best_candidate, install_candidate};
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::{PkgError, PkgReceipt};
use crate::progress::{PkgProgress, ProgressManager};
use crate::registry::Registry;
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
    progress: PkgProgress,
}

enum InstallResult {
    Success(PkgReceipt),
    Failure(String, String),
}

impl CommandHandler for InstallCommand {
    fn handle(&self) -> Result<()> {
        let names = unique(&self.names);

        // prepare
        let layout = InroLayout::new()?;
        let config = Config::load(&layout)?;
        let registry = Registry::load(&layout)?;
        if registry.pkgs.is_empty() {
            eprintln!(
                "{} Registry is empty. Run 'inro source update' to fetch packages",
                "warning:".yellow().bold()
            );
            return Ok(());
        }
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        // Create progress manager
        let pm = ProgressManager::new();

        // Create tokio runtime for async operations
        let rt = tokio::runtime::Runtime::new()?;

        // Parse package names and validate
        let mut valid_tasks = Vec::new();
        let mut failures: Vec<(String, String)> = Vec::new();

        for name in &names {
            let (pkg_name, pkg_ver) = parse_package_version(name);
            match registry.pkgs.get(pkg_name) {
                Some(_) => {
                    let progress = pm.add_package(pkg_name);
                    valid_tasks.push(InstallTask {
                        name: name.clone(),
                        pkg_name: pkg_name.to_string(),
                        pkg_ver: pkg_ver.map(|s| s.to_string()),
                        progress,
                    });
                }
                None => {
                    let err = PkgError::NotFound(pkg_name.to_string());
                    // Add failed package to progress display
                    let progress = pm.add_package(pkg_name);
                    progress.finish_error(&err.to_string());
                    failures.push((name.clone(), err.to_string()));
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
                        let candidate = match find_best_candidate(
                            pkg_def,
                            task.pkg_ver.as_deref(),
                            &task.progress,
                        )
                        .await
                        {
                            Ok(c) => c,
                            Err(e) => {
                                task.progress.finish_error(&e.to_string());
                                return InstallResult::Failure(task.name.clone(), e.to_string());
                            }
                        };

                        // Download and install (async download, sync extraction)
                        match install_candidate(
                            &task.pkg_name,
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
                                InstallResult::Success(receipt)
                            }
                            Err(e) => {
                                task.progress.finish_error(&e.to_string());
                                InstallResult::Failure(task.name.clone(), e.to_string())
                            }
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
                        eprintln!(
                            "{} Failed to save backup receipt: {e}",
                            "warning:".yellow().bold()
                        );
                    }
                    manifest.add(receipt.clone());
                    successes.push(receipt);
                }
                InstallResult::Failure(name, err) => {
                    failures.push((name, err));
                }
            }
        }

        // save manifest
        manifest.save(&layout.manifest_path)?;

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
        eprintln!("{} Successfully installed {} package(s):", "✓".green().bold(), successes.len());

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

        eprintln!("{} Failed to install {} package(s):", "✗".red().bold(), failures.len());

        let max_len = failures.iter().map(|t| t.0.len()).max().unwrap_or(0);
        for (name, err) in failures {
            eprintln!("  {} {:<max_len$} : {}", "•".red(), name.bold(), err);
        }
    }

    if !has_success && !has_failure {
        eprintln!("{} Nothing was installed", "warning:".yellow().bold());
    }
}
