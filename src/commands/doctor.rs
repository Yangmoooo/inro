use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::package::PkgDef;
use crate::{done, fail, step, warn};

pub struct DoctorCommand {
    pub fix: bool,
}

impl CommandHandler for DoctorCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let _lock = if self.fix { Some(crate::lock::acquire(&layout)?) } else { None };
        let mut warnings = 0usize;
        let mut failed = 0usize;

        // ── 1. Config file ────────────────────────────────────────────────
        step!("Checking config file");
        let config = match Config::load(&layout) {
            Ok(cfg) => {
                if !layout.config_path.exists() {
                    warn!(
                        "config file not found, using defaults ({})",
                        layout.config_path.display()
                    );
                    warnings += 1;
                } else {
                    done!("config file parsed successfully");
                }
                Some(cfg)
            }
            Err(e) => {
                fail!("failed to parse config: {e}");
                failed += 1;
                None
            }
        };

        // ── 2. Source (registry) files ────────────────────────────────────
        step!("Checking source files");
        let source_dirs = [&layout.managed_registry_dir, &layout.user_registry_dir];
        let mut any_source = false;
        for dir in source_dirs {
            if !dir.exists() {
                continue;
            }
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                any_source = true;
                match try_parse_registry_file(&path) {
                    Ok(()) => done!("{}", path.display()),
                    Err(e) => {
                        fail!("{}: {e}", path.display());
                        failed += 1;
                    }
                }
            }
        }
        if !any_source {
            warn!("no source files found — run 'inro source update' to fetch the registry");
            warnings += 1;
        }

        // ── 3. bin_dir in PATH ────────────────────────────────────────────
        step!("Checking bin_dir in PATH");
        if let Some(ref cfg) = config {
            if is_in_path(&cfg.bin_dir) {
                done!("{} is in PATH", cfg.bin_dir.display());
            } else {
                warn!("{} is NOT in PATH — add it to your shell profile", cfg.bin_dir.display());
                warnings += 1;
            }
        } else {
            warn!("skipped (config failed to load)");
            warnings += 1;
        }

        // ── 4. Symlink & binary integrity ─────────────────────────────────
        step!("Checking installed package links");
        let manifest = Manifest::load(&layout.manifest_path);
        let pkgs_dir = &layout.pkgs_dir;
        match &manifest {
            Err(e) => {
                fail!("failed to load manifest: {e}");
                failed += 1;
            }
            Ok(m) if m.pkgs.is_empty() => {
                done!("no packages installed");
            }
            Ok(m) => {
                let mut link_issues = 0usize;
                let cfg_bin_dir = config.as_ref().map(|c| c.bin_dir.as_path());
                for (pkg_name, state) in &m.pkgs {
                    let is_current_fn = |ver: &str| state.current_version.as_deref() == Some(ver);
                    for (version, receipt) in &state.versions {
                        for bin in &receipt.binaries {
                            let label = format!("{pkg_name}@{version}/{}", bin.name);
                            let bin_path = receipt.bin_path(bin, pkgs_dir);
                            let link_path = cfg_bin_dir.map(|d| receipt.link_path(bin, d));

                            // Link checks require a known bin_dir
                            let Some(link) = link_path.as_ref() else {
                                if !bin_path.exists() {
                                    fail!("{label}: binary gone ({})", bin_path.display());
                                    failed += 1;
                                    link_issues += 1;
                                }
                                continue;
                            };

                            let link_present = link.exists() || link.is_symlink();
                            if !link_present {
                                if is_current_fn(version) {
                                    warn!("{label}: link missing ({})", link.display());
                                    warnings += 1;
                                    link_issues += 1;
                                }
                                continue;
                            }

                            // Symlink exists but its target file has been deleted
                            if link.is_symlink() && !link.exists() {
                                fail!(
                                    "{label}: broken symlink {} → {}",
                                    link.display(),
                                    bin_path.display()
                                );
                                failed += 1;
                                link_issues += 1;
                                continue;
                            }

                            // The extracted binary file itself is gone
                            if !bin_path.exists() {
                                fail!("{label}: binary gone ({})", bin_path.display());
                                failed += 1;
                                link_issues += 1;
                            }
                        }
                    }
                }
                if link_issues == 0 {
                    done!("all package links are healthy");
                }
            }
        }

        // ── 5. Manifest consistency & bin_dir mismatch ───────────────────
        step!("Checking manifest consistency");
        match manifest {
            Err(_) => {
                warn!("skipped (manifest failed to load)");
                warnings += 1;
            }
            Ok(m) => {
                let cfg_bin_dir = config.as_ref().map(|c| c.bin_dir.clone());
                let mut stale = 0usize;
                let mut mismatches = 0usize;
                let mut fixed = 0usize;

                for (pkg_name, state) in &m.pkgs {
                    let current = state.current_version.as_deref();

                    for (version, receipt) in &state.versions {
                        let install_dir = receipt.install_dir(pkgs_dir);
                        // Stale entry, package directory no longer on disk
                        if !install_dir.exists() {
                            warn!(
                                "{pkg_name}@{version}: install directory missing ({})",
                                install_dir.display()
                            );
                            warnings += 1;
                            stale += 1;
                        }

                        // Link integrity for the currently active version
                        if current == Some(version.as_str())
                            && let Some(bin_dir) = &cfg_bin_dir
                        {
                            let needs_relink = receipt.binaries.iter().any(|bin| {
                                let link = receipt.link_path(bin, bin_dir);
                                let want = receipt.bin_path(bin, pkgs_dir);
                                match fs::read_link(&link) {
                                    Ok(target) => target != want,
                                    Err(_) => true, // missing or not a symlink
                                }
                            });

                            if needs_relink {
                                warn!(
                                    "{pkg_name}: links under '{}' don't match installed binaries",
                                    bin_dir.display()
                                );
                                warnings += 1;
                                mismatches += 1;

                                if self.fix {
                                    match receipt.relink(bin_dir, pkgs_dir) {
                                        Ok(()) => {
                                            done!("re-linked {pkg_name} → {}", bin_dir.display());
                                            fixed += 1;
                                        }
                                        Err(e) => {
                                            fail!("failed to re-link {pkg_name}: {e}");
                                            failed += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if stale == 0 && mismatches == 0 {
                    done!("manifest is consistent");
                } else if mismatches > 0 && !self.fix {
                    warn!("{mismatches} mismatch(es) found — run 'inro doctor --fix' to repair");
                }

                // doctor --fix re-links symlinks but does not mutate the receipt
                // itself (paths are derived from the layout, not stored), so no
                // manifest save is needed when fixed > 0.
                let _ = fixed;
            }
        }

        // ── Summary ───────────────────────────────────────────────────────
        eprintln!(
            "\n{} {}",
            format!("{warnings} warning(s)").yellow(),
            format!("{failed} error(s)").red()
        );

        Ok(())
    }
}

/// Check whether `dir` is present in the `PATH` environment variable.
fn is_in_path(dir: &Path) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    let target = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    std::env::split_paths(&paths).any(|p| fs::canonicalize(&p).unwrap_or(p) == target)
}

/// Attempt to deserialize a `.toml` registry file into the expected format.
/// Returns an error if the file is syntactically invalid or contains
/// entries that don't match the `PkgDef` schema.
fn try_parse_registry_file(path: &Path) -> Result<()> {
    let content = fs::read_to_string(path)?;
    let _: HashMap<String, PkgDef> = toml::from_str(&content)?;
    Ok(())
}
