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
use crate::report;
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
    Skipped,
    NotInstalled,
    Failed(String, String),
}

/// Task for parallel update checking
struct UpdateTask {
    name: String,
    current_version: String,
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

        // Prepare tasks - filter out not installed packages
        let mut tasks: Vec<UpdateTask> = Vec::new();
        let mut results: Vec<UpdateResult> = Vec::new();

        for name in &names {
            let (pkg_name, pkg_ver) = parse_package_version(name);

            if pkg_ver.is_some() {
                report!(
                    MsgType::Warning,
                    "Version specifier ignored for '{name}'. Update always targets the latest version"
                );
            }

            match manifest.pkgs.get(pkg_name) {
                Some(state) => {
                    let current_ver = state.current_version.as_deref().unwrap_or_default();
                    tasks.push(UpdateTask {
                        name: pkg_name.to_string(),
                        current_version: current_ver.to_string(),
                    });
                }
                None => {
                    report!(MsgType::Warning, "'{pkg_name}' not installed, skipping");
                    results.push(UpdateResult::NotInstalled);
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
                                return UpdateResult::Failed(task.name.clone(), err.to_string());
                            }
                        };

                        // Fetch latest candidate
                        let candidate = match find_best_candidate_async(pkg_def, None).await {
                            Ok(c) => c,
                            Err(e) => {
                                return UpdateResult::Failed(task.name.clone(), e.to_string());
                            }
                        };

                        // Check if already up to date
                        if candidate.version == task.current_version {
                            report!(
                                MsgType::Info,
                                "'{}' is up to date ({})",
                                task.name,
                                task.current_version
                            );
                            return UpdateResult::Skipped;
                        }

                        report!(
                            MsgType::Step,
                            "Updating '{}': {} -> {}",
                            task.name,
                            task.current_version,
                            candidate.version
                        );

                        let pkg = pkg_def.clone().resolve(&task.name);

                        // Download and install
                        match install_candidate_async(&task.name, &candidate, &pkg, config, layout)
                            .await
                        {
                            Ok(receipt) => UpdateResult::Updated(Box::new(UpdateReceipt {
                                name: task.name.clone(),
                                old_version: task.current_version.clone(),
                                new_version: candidate.version,
                                full_receipt: receipt,
                            })),
                            Err(e) => UpdateResult::Failed(task.name.clone(), e.to_string()),
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
                    report!(
                        MsgType::Warning,
                        "Failed to save backup receipt for '{}': {e}",
                        receipt.name
                    );
                }
                any_updated = true;
            }
        }

        if any_updated {
            manifest.save(&layout.manifest_path)?;
            report!(MsgType::Detail, "Manifest updated");
        }

        print_summary(&results);

        Ok(())
    }
}

fn print_summary(results: &[UpdateResult]) {
    let mut updated = Vec::new();
    let mut failed = Vec::new();

    for res in results {
        match res {
            UpdateResult::Updated(r) => updated.push(r),
            UpdateResult::Failed(name, error) => failed.push((name, error)),
            _ => (),
        }
    }

    eprintln!();

    if !updated.is_empty() {
        report!(MsgType::Success, "Updated {} package(s):", updated.len());
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

        report!(MsgType::Error, "Failed to update {} package(s):", failed.len());
        let max_len = failed.iter().map(|t| t.0.len()).max().unwrap_or(0);
        for (name, err) in failed {
            eprintln!("  {} {:<max_len$} : {}", "•".red(), name.bold(), err);
        }
        std::process::exit(1);
    }

    if updated.is_empty() && failed.is_empty() {
        report!(MsgType::Success, "All packages are up to date.");
    }
}
