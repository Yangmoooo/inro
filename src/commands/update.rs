use anyhow::Result;
use colored::Colorize;
use futures::stream::{self, StreamExt};

use super::CommandHandler;
use crate::config::Config;
use crate::installer::{find_candidates, install_candidate, select_candidate};
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::{PkgError, PkgReceipt};
use crate::progress::{PkgProgress, ProgressManager};
use crate::registry::{AssetSelectionWriteBack, Registry};
use crate::remotes::CandidateResult;
use crate::reporter::print_error_chain;
use crate::utils::{parse_package_version, unique};
use crate::warn;

pub struct UpdateCommand {
    pub names: Vec<String>,
    pub force: bool,
}

struct UpdateTask {
    name: String,
    pkg_ver: Option<String>,
    current_version: String,
    progress: PkgProgress,
}

impl CommandHandler for UpdateCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
        let config = Config::load(&layout)?;
        let registry = Registry::load(&layout)?;
        let mut manifest = Manifest::load(&layout.manifest_path)?;

        let names: Vec<String> = if self.names.is_empty() {
            manifest.pkgs.keys().cloned().collect()
        } else {
            unique(&self.names)
        };

        if names.is_empty() {
            eprintln!("{} No packages installed", "·".dimmed());
            return Ok(());
        }

        // Parse and collect package names
        let parsed: Vec<_> = names
            .iter()
            .map(|n| {
                let (pkg_name, pkg_ver) = parse_package_version(n);
                (pkg_name.to_string(), pkg_ver.map(|s| s.to_string()))
            })
            .collect();

        let pkg_names: Vec<&str> = parsed.iter().map(|(name, _)| name.as_str()).collect();
        let pm = ProgressManager::new(&pkg_names);

        // Create tasks for installed packages
        let mut tasks = Vec::new();
        let mut not_installed = 0usize;

        for (pkg_name, pkg_ver) in &parsed {
            match manifest.pkgs.get(pkg_name) {
                Some(state) => {
                    if state.pinned && !self.force {
                        pm.add_package(pkg_name).finish_error("pinned, skipping");
                        continue;
                    }
                    let current_ver = state.current_version.as_deref().unwrap_or_default();
                    let progress = pm.add_package(pkg_name);
                    tasks.push(UpdateTask {
                        name: pkg_name.clone(),
                        pkg_ver: pkg_ver.clone(),
                        current_version: current_ver.to_string(),
                        progress,
                    });
                }
                None => {
                    pm.add_package(pkg_name).finish_error("not installed");
                    not_installed += 1;
                }
            }
        }

        let rt = tokio::runtime::Runtime::new()?;
        let parallel_limit = config.parallel_downloads;

        // Phase 1: Parallel fetch candidates
        let fetch_results: Vec<(UpdateTask, Result<CandidateResult, PkgError>)> =
            rt.block_on(async {
                stream::iter(tasks)
                    .map(|task| {
                        let registry = &registry;
                        async move {
                            let result = match registry.pkgs.get(&task.name) {
                                Some(pkg_def) => {
                                    find_candidates(pkg_def, task.pkg_ver.as_deref(), &task.progress).await
                                }
                                None => Err(PkgError::NotFound(task.name.clone())),
                            };
                            (task, result)
                        }
                    })
                    .buffer_unordered(parallel_limit)
                    .collect()
                    .await
            });

        // Phase 2: Sequential selection + up-to-date check
        let mut install_tasks = Vec::new();
        let mut up_to_date = 0usize;
        let mut failed = not_installed;

        for (task, result) in fetch_results {
            match result {
                Ok(candidate_result) => {
                    // All candidates come from the same release, so compare the release tag before
                    // prompting for an asset choice.
                    let first_candidate_ver =
                        candidate_result.candidates.first().map(|c| c.version.as_str());
                    let target_ver = task.pkg_ver.as_deref().or(first_candidate_ver);
                    if let Some(target) = target_ver
                        && target == task.current_version
                    {
                        task.progress.finish_unchanged(&task.current_version);
                        up_to_date += 1;
                        continue;
                    }

                    let selection = pm.suspend(|| select_candidate(&task.name, candidate_result));
                    match selection {
                        Ok(sel) => {
                            install_tasks.push((task, sel.candidate, sel.write_back));
                        }
                        Err(e) => {
                            task.progress.finish_error(&e.to_string());
                            print_error_chain(&e);
                            failed += 1;
                        }
                    }
                }
                Err(e) => {
                    task.progress.finish_error(&e.to_string());
                    print_error_chain(&e);
                    failed += 1;
                }
            }
        }

        // Phase 3: Parallel install
        let mut updated = 0usize;

        let results: Vec<Option<(PkgReceipt, Option<AssetSelectionWriteBack>)>> =
            rt.block_on(async {
                stream::iter(install_tasks)
                    .map(|(task, candidate, write_back)| {
                        let registry = &registry;
                        let config = &config;
                        let layout = &layout;
                        async move {
                            let Some(pkg_def) = registry.pkgs.get(&task.name) else {
                                task.progress.finish_error("not found in registry");
                                return None;
                            };
                            let pkg = pkg_def.clone().resolve(&task.name);
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
                                    Some((receipt, write_back))
                                }
                                Err(e) => {
                                    task.progress.finish_error(&e.to_string());
                                    print_error_chain(&e);
                                    None
                                }
                            }
                        }
                    })
                    .buffer_unordered(parallel_limit)
                    .collect()
                    .await
            });

        let mut write_backs = Vec::new();
        for result in results {
            match result {
                Some((receipt, write_back)) => {
                    if let Err(e) = receipt.save_to_install_dir(&layout.pkgs_dir) {
                        warn!(
                            "Updated '{}' but failed to write receipt to {}: {e:#}",
                            receipt.name,
                            receipt.install_dir(&layout.pkgs_dir).display()
                        );
                    }
                    manifest.add(receipt);
                    if let Some(wb) = write_back {
                        write_backs.push(wb);
                    }
                    updated += 1;
                }
                None => failed += 1,
            }
        }

        if !write_backs.is_empty()
            && let Err(e) = Registry::write_asset_selections(&layout, &write_backs)
        {
            warn!("Failed to save asset selections: {e}");
        }

        if updated > 0 {
            manifest.save(&layout.manifest_path)?;
        }

        print_summary(updated, up_to_date, failed);

        if failed > 0 {
            std::process::exit(1);
        }
        Ok(())
    }
}

fn print_summary(updated: usize, up_to_date: usize, failed: usize) {
    eprintln!();
    if updated > 0 && failed == 0 {
        eprintln!("{} Updated {} package(s)", "✓".green().bold(), updated);
    } else if updated > 0 && failed > 0 {
        eprintln!("{} Updated {} package(s), {} failed", "!".yellow().bold(), updated, failed);
    } else if failed > 0 {
        eprintln!("{} All {} package(s) failed", "✗".red().bold(), failed);
    } else if up_to_date > 0 {
        eprintln!("{} All {} package(s) are up to date", "✓".green().bold(), up_to_date);
    } else {
        eprintln!("{} Nothing to update", "·".dimmed());
    }
}
