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

pub struct UpdateCommand {
    pub names: Vec<String>,
}

struct UpdateReceipt {
    name: String,
    old_version: String,
    new_version: String,
    full_receipt: PkgReceipt,
}

enum UpdateResult {
    Updated(Box<UpdateReceipt>),
    Skipped(String),
    #[allow(dead_code)]
    NotInstalled(String),
    Failed(String, String),
}

/// Task for parallel update checking
struct UpdateTask {
    name: String,
    current_version: String,
    progress: PkgProgress,
}

impl CommandHandler for UpdateCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let config = Config::load(&layout)?;
        let registry = Registry::load(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        let names: Vec<String> = if self.names.is_empty() {
            manifest.pkgs.keys().cloned().collect()
        } else {
            unique(&self.names)
        };

        // Create progress manager
        let pm = ProgressManager::new();

        // Prepare tasks - filter out not installed packages
        let mut tasks: Vec<UpdateTask> = Vec::new();
        let mut results: Vec<UpdateResult> = Vec::new();

        for name in &names {
            let (pkg_name, pkg_ver) = parse_package_version(name);

            if pkg_ver.is_some() {
                eprintln!(
                    "{} Version specifier ignored for '{name}'. Update always targets the latest version",
                    "warning:".yellow().bold()
                );
            }

            match manifest.pkgs.get(pkg_name) {
                Some(state) => {
                    let current_ver = state.current_version.as_deref().unwrap_or_default();
                    let progress = pm.add_package(pkg_name);
                    tasks.push(UpdateTask {
                        name: pkg_name.to_string(),
                        current_version: current_ver.to_string(),
                        progress,
                    });
                }
                None => {
                    let progress = pm.add_package(pkg_name);
                    progress.finish_error("not installed");
                    results.push(UpdateResult::NotInstalled(pkg_name.to_string()));
                }
            }
        }

        // Create tokio runtime for async operations
        let rt = tokio::runtime::Runtime::new()?;
        let parallel_limit = config.parallel_downloads;

        // Parallel update checking and installation
        let async_results: Vec<UpdateResult> = rt.block_on(async {
            stream::iter(tasks)
                .map(|task| {
                    let registry = &registry;
                    let config = &config;
                    let layout = &layout;
                    async move {
                        let pkg_def = match registry.pkgs.get(&task.name) {
                            Some(def) => def,
                            None => {
                                let err = PkgError::NotFound(task.name.clone());
                                task.progress.finish_error(&err.to_string());
                                return UpdateResult::Failed(task.name.clone(), err.to_string());
                            }
                        };

                        // Fetch latest candidate
                        let candidate =
                            match find_best_candidate(pkg_def, None, &task.progress).await {
                                Ok(c) => c,
                                Err(e) => {
                                    task.progress.finish_error(&e.to_string());
                                    return UpdateResult::Failed(task.name.clone(), e.to_string());
                                }
                            };

                        // Check if already up to date
                        if candidate.version == task.current_version {
                            task.progress.finish_success(&task.current_version);
                            return UpdateResult::Skipped(task.name.clone());
                        }

                        let pkg = pkg_def.clone().resolve(&task.name);

                        // Download and install
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
                                UpdateResult::Updated(Box::new(UpdateReceipt {
                                    name: task.name.clone(),
                                    old_version: task.current_version.clone(),
                                    new_version: candidate.version.clone(),
                                    full_receipt: receipt,
                                }))
                            }
                            Err(e) => {
                                task.progress.finish_error(&e.to_string());
                                UpdateResult::Failed(task.name.clone(), e.to_string())
                            }
                        }
                    }
                })
                .buffer_unordered(parallel_limit)
                .collect()
                .await
        });

        // Merge results
        results.extend(async_results);

        // Process results
        let mut any_updated = false;
        for result in &results {
            if let UpdateResult::Updated(receipt) = result {
                manifest.add(receipt.full_receipt.clone());
                if let Err(e) = receipt.full_receipt.save_to_install_dir() {
                    eprintln!(
                        "{} Failed to save backup receipt for '{}': {e}",
                        "warning:".yellow().bold(),
                        receipt.name
                    );
                }
                any_updated = true;
            }
        }

        if any_updated {
            manifest.save(&layout.manifest_path)?;
        }

        print_summary(&results);

        Ok(())
    }
}

fn print_summary(results: &[UpdateResult]) {
    let mut updated = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for res in results {
        match res {
            UpdateResult::Updated(r) => updated.push(r),
            UpdateResult::Skipped(name) => skipped.push(name),
            UpdateResult::Failed(name, error) => failed.push((name, error)),
            UpdateResult::NotInstalled(_) => (),
        }
    }

    eprintln!();

    if !updated.is_empty() {
        eprintln!("{} Updated {} package(s):", "✓".green().bold(), updated.len());
        let max_len = updated.iter().map(|r| r.name.len()).max().unwrap_or(0);
        for r in &updated {
            eprintln!(
                "  {} {:<max_len$} : {} -> {}",
                "+".green(),
                r.name.bold(),
                r.old_version.dimmed(),
                r.new_version,
            );
        }
    }

    if !failed.is_empty() {
        if !updated.is_empty() {
            eprintln!();
        }

        eprintln!("{} Failed to update {} package(s):", "✗".red().bold(), failed.len());
        let max_len = failed.iter().map(|t| t.0.len()).max().unwrap_or(0);
        for (name, err) in failed {
            eprintln!("  {} {:<max_len$} : {}", "•".red(), name.bold(), err);
        }
        std::process::exit(1);
    }

    if updated.is_empty() && failed.is_empty() {
        if skipped.is_empty() {
            eprintln!("{} Nothing to update", "info:".cyan().bold());
        } else {
            eprintln!("{} All {} package(s) are up to date", "✓".green().bold(), skipped.len());
        }
    }
}
