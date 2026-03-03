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
        let source_dirs = [&layout.upstream_registry_dir, &layout.local_registry_dir];
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
                for (pkg_name, state) in &m.pkgs {
                    let is_current_fn = |ver: &str| state.current_version.as_deref() == Some(ver);
                    for (version, receipt) in &state.versions {
                        for bin in &receipt.binaries {
                            let label = format!("{pkg_name}@{version}/{}", bin.name);

                            // Link missing, only relevant for the active version
                            let link_present = bin.link_path.exists() || bin.link_path.is_symlink();
                            if !link_present {
                                if is_current_fn(version) {
                                    warn!("{label}: link missing ({})", bin.link_path.display());
                                    warnings += 1;
                                    link_issues += 1;
                                }
                                continue;
                            }

                            // Symlink exists but its target file has been deleted
                            if bin.link_path.is_symlink() && !bin.link_path.exists() {
                                fail!(
                                    "{label}: broken symlink {} → {}",
                                    bin.link_path.display(),
                                    bin.bin_path.display()
                                );
                                failed += 1;
                                link_issues += 1;
                                continue;
                            }

                            // The extracted binary file itself is gone
                            if !bin.bin_path.exists() {
                                fail!("{label}: binary gone ({})", bin.bin_path.display());
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
            Ok(mut m) => {
                let cfg_bin_dir = config.as_ref().map(|c| c.bin_dir.clone());
                let cfg_canon = cfg_bin_dir
                    .as_deref()
                    .map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf()));
                let mut stale = 0usize;
                let mut mismatches = 0usize;
                let mut fixed = 0usize;

                for (pkg_name, state) in m.pkgs.iter_mut() {
                    let current = state.current_version.clone();

                    for (version, receipt) in state.versions.iter_mut() {
                        // Stale entry, package directory no longer on disk
                        if !receipt.install_dir.exists() {
                            warn!(
                                "{pkg_name}@{version}: install directory missing ({})",
                                receipt.install_dir.display()
                            );
                            warnings += 1;
                            stale += 1;
                        }

                        // bin_dir mismatch, only check the currently active version
                        if current.as_deref() == Some(version.as_str())
                            && let (Some(bin_dir), Some(expected_canon)) =
                                (&cfg_bin_dir, &cfg_canon)
                        {
                            let has_mismatch = receipt.binaries.iter().any(|bin| {
                                let lp = bin.link_path.parent().unwrap_or(Path::new(""));
                                fs::canonicalize(lp).unwrap_or_else(|_| lp.to_path_buf())
                                    != *expected_canon
                            });

                            if has_mismatch {
                                let old_dir = receipt
                                    .binaries
                                    .first()
                                    .and_then(|b| b.link_path.parent())
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                warn!(
                                    "{pkg_name}: symlinks are in '{old_dir}' but bin_dir \
                                         is '{}'",
                                    bin_dir.display()
                                );
                                warnings += 1;
                                mismatches += 1;

                                if self.fix {
                                    match receipt.relink(bin_dir) {
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

                // Persist manifest only if we actually fixed something
                if fixed > 0 {
                    m.save(&layout.manifest_path)?;
                }
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
