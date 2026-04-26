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

pub struct InstallCommand {
    /// Package names, optionally with version (e.g. "ripgrep@15.1.0")
    pub names: Vec<String>,
}

struct InstallTask {
    pkg_name: String,
    pkg_ver: Option<String>,
    progress: PkgProgress,
}

impl CommandHandler for InstallCommand {
    fn handle(&self) -> Result<()> {
        let names = unique(&self.names);

        let layout = InroLayout::new()?;
        let _lock = crate::lock::acquire(&layout)?;
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

        // Parse and collect package names for width calculation
        let parsed: Vec<_> = names
            .iter()
            .map(|n| {
                let (pkg_name, pkg_ver) = parse_package_version(n);
                (n.clone(), pkg_name.to_string(), pkg_ver.map(|s| s.to_string()))
            })
            .collect();
        let pkg_names: Vec<&str> = parsed.iter().map(|(_, p, _)| p.as_str()).collect();
        let pm = ProgressManager::new(&pkg_names);

        // Validate and create tasks
        let mut valid_tasks = Vec::new();
        let mut fail_count = 0usize;

        for (_, pkg_name, pkg_ver) in parsed {
            match registry.pkgs.get(&pkg_name) {
                Some(_) => {
                    let progress = pm.add_package(&pkg_name);
                    valid_tasks.push(InstallTask { pkg_name, pkg_ver, progress });
                }
                None => {
                    let err = PkgError::NotFound(pkg_name.clone());
                    pm.add_package(&pkg_name).finish_error(&err.to_string());
                    fail_count += 1;
                }
            }
        }

        let rt = tokio::runtime::Runtime::new()?;
        let parallel_limit = config.parallel_downloads;

        // Phase 1: Parallel fetch candidates
        let fetch_results: Vec<(InstallTask, Result<CandidateResult, PkgError>)> =
            rt.block_on(async {
                stream::iter(valid_tasks)
                    .map(|task| {
                        let registry = &registry;
                        async move {
                            let result = match registry.pkgs.get(&task.pkg_name) {
                                Some(pkg_def) => {
                                    find_candidates(
                                        pkg_def,
                                        task.pkg_ver.as_deref(),
                                        &task.progress,
                                    )
                                    .await
                                }
                                None => Err(PkgError::NotFound(task.pkg_name.clone())),
                            };
                            (task, result)
                        }
                    })
                    .buffer_unordered(parallel_limit)
                    .collect()
                    .await
            });

        // Phase 2: Sequential selection (may prompt interactively)
        let mut install_tasks = Vec::new();

        for (task, result) in fetch_results {
            match result {
                Ok(candidate_result) => {
                    let selection =
                        pm.suspend(|| select_candidate(&task.pkg_name, candidate_result));
                    match selection {
                        Ok(sel) => {
                            install_tasks.push((task, sel.candidate, sel.write_back));
                        }
                        Err(e) => {
                            task.progress.finish_error(&e.to_string());
                            print_error_chain(&e);
                            fail_count += 1;
                        }
                    }
                }
                Err(e) => {
                    task.progress.finish_error(&e.to_string());
                    print_error_chain(&e);
                    fail_count += 1;
                }
            }
        }

        // Phase 3: Parallel install
        let results: Vec<Option<(PkgReceipt, Option<AssetSelectionWriteBack>)>> =
            rt.block_on(async {
                stream::iter(install_tasks)
                    .map(|(task, candidate, write_back)| {
                        let registry = &registry;
                        let config = &config;
                        let layout = &layout;
                        async move {
                            let pkg_def = registry.pkgs.get(&task.pkg_name)?;
                            let pkg = pkg_def.clone().resolve(&task.pkg_name);

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

        // Process results
        let mut success_count = 0usize;
        let mut write_backs = Vec::new();
        for result in results {
            match result {
                Some((receipt, write_back)) => {
                    if let Err(e) = receipt.save_to_install_dir() {
                        warn!(
                            "Installed '{}' but failed to write receipt to {}: {e:#}",
                            receipt.name,
                            receipt.install_dir.display()
                        );
                    }
                    manifest.add(receipt);
                    if let Some(wb) = write_back {
                        write_backs.push(wb);
                    }
                    success_count += 1;
                }
                None => fail_count += 1,
            }
        }

        if !write_backs.is_empty()
            && let Err(e) = Registry::write_asset_selections(&layout, &write_backs)
        {
            warn!("Failed to save asset selections: {e}");
        }

        manifest.save(&layout.manifest_path)?;
        print_summary(success_count, fail_count);

        if fail_count > 0 {
            std::process::exit(1);
        }
        Ok(())
    }
}

fn print_summary(success: usize, failed: usize) {
    eprintln!();
    if success > 0 && failed == 0 {
        eprintln!("{} Installed {} package(s)", "✓".green().bold(), success);
    } else if success > 0 && failed > 0 {
        eprintln!("{} Installed {} package(s), {} failed", "!".yellow().bold(), success, failed);
    } else if failed > 0 {
        eprintln!("{} All {} package(s) failed", "✗".red().bold(), failed);
    } else {
        eprintln!("{} Nothing to install", "·".dimmed());
    }
}
