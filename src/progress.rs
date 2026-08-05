//! Progress tracking for concurrent package operations.
//!
//! Provides multi-progress bar display for install/update commands.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Phase of a package operation (for progress display)
#[derive(Clone, Copy)]
pub enum OpPhase {
    Fetching,
    Downloading,
    Extracting,
}

impl OpPhase {
    fn symbol(self) -> &'static str {
        match self {
            OpPhase::Fetching | OpPhase::Extracting => "⠸",
            OpPhase::Downloading => "↓",
        }
    }

    fn message(self) -> &'static str {
        match self {
            OpPhase::Fetching => "fetching release...",
            OpPhase::Downloading => "", // handled by progress bar
            OpPhase::Extracting => "extracting...",
        }
    }

    fn plain_message(self) -> &'static str {
        match self {
            OpPhase::Fetching => "fetching release...",
            OpPhase::Downloading => "downloading...",
            OpPhase::Extracting => "extracting...",
        }
    }
}

/// A handle to update a single package's progress.
#[derive(Clone)]
pub struct PkgProgress {
    bar: Option<ProgressBar>,
    name: String,
    max_name_width: usize,
}

impl PkgProgress {
    /// Update the operation phase.
    pub fn set_phase(&self, phase: OpPhase) {
        let symbol = phase.symbol();
        let msg = phase.message();
        let name = &self.name;
        let w = self.max_name_width;

        let Some(bar) = &self.bar else {
            crate::reporter::print_detail(&format!("{name:<w$}  {}", phase.plain_message()));
            return;
        };

        match phase {
            OpPhase::Downloading => {
                bar.set_style(download_style());
                bar.set_message(format!("{name:<w$}"));
            }
            _ => {
                bar.set_style(spinner_style());
                bar.enable_steady_tick(Duration::from_millis(80));
                bar.set_message(format!("{symbol} {name:<w$}  {msg}"));
            }
        }
    }

    /// Set the total size for download progress.
    pub fn set_length(&self, len: u64) {
        if let Some(bar) = &self.bar {
            bar.set_length(len);
        }
    }

    /// Increment download progress.
    pub fn inc(&self, delta: u64) {
        if let Some(bar) = &self.bar {
            bar.inc(delta);
        }
    }

    /// Mark as completed with version.
    pub fn finish_success(&self, version: &str) {
        let name = &self.name;
        let w = self.max_name_width;
        if let Some(bar) = &self.bar {
            bar.set_style(status_style());
            bar.finish_with_message(format!("✓ {name:<w$}  {version}"));
        } else {
            crate::reporter::print_done(&format!("{name:<w$}  {version}"));
        }
    }

    /// Mark as already up to date (no install needed).
    pub fn finish_unchanged(&self, version: &str) {
        let name = &self.name;
        let w = self.max_name_width;
        if let Some(bar) = &self.bar {
            bar.set_style(status_style());
            let line = format!("= {name:<w$}  {version}  (up to date)");
            bar.finish_with_message(line.dimmed().to_string());
        } else {
            crate::reporter::print_skip(&format!("{name:<w$}  {version}  (up to date)"));
        }
    }

    /// Mark as failed with error message.
    pub fn finish_error(&self, err: &str) {
        let name = &self.name;
        let w = self.max_name_width;
        if let Some(bar) = &self.bar {
            bar.set_style(status_style());
            bar.finish_with_message(format!("✗ {name:<w$}  {err}"));
        } else {
            crate::reporter::print_fail(&format!("{name:<w$}  {err}"));
        }
    }
}

/// Manager for multi-package progress display.
pub struct ProgressManager {
    retained_bars: Mutex<Vec<ProgressBar>>,
    multi: Option<Arc<MultiProgress>>,
    name_width: usize,
}

impl ProgressManager {
    /// Create a new progress manager with calculated name width.
    pub fn new(pkg_names: &[&str]) -> Self {
        let name_width = pkg_names.iter().map(|n| n.len()).max().unwrap_or(0);
        let multi = if crate::VERBOSITY.load(Ordering::Relaxed) > 0 {
            None
        } else {
            Some(Arc::new(MultiProgress::new()))
        };
        Self { retained_bars: Mutex::new(Vec::new()), multi, name_width }
    }

    /// Temporarily suspend progress bars for interactive prompts.
    pub fn suspend<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        if let Some(multi) = &self.multi { multi.suspend(f) } else { f() }
    }

    /// Add a package to track.
    pub fn add_package(&self, name: &str) -> PkgProgress {
        let w = self.name_width;
        let bar = self.multi.as_ref().map(|multi| {
            let bar = multi.add(ProgressBar::new(0));
            bar.set_style(spinner_style());
            bar.set_message(format!("◦ {name:<w$}  pending"));
            self.retained_bars.lock().expect("progress bar lock poisoned").push(bar.clone());
            bar
        });

        PkgProgress { bar, name: name.to_string(), max_name_width: self.name_width }
    }

    #[cfg(test)]
    fn is_plain_mode(&self) -> bool { self.multi.is_none() }
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner().template("  {msg}").expect("invalid template")
}

fn status_style() -> ProgressStyle {
    ProgressStyle::default_bar().template("  {msg}").expect("invalid template")
}

fn download_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("  ↓ {msg}  [{bar:20.cyan/dim}] {bytes}/{total_bytes} ({bytes_per_sec})")
        .expect("invalid template")
        .progress_chars("━╸─")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{LazyLock, Mutex, MutexGuard};

    use super::*;

    static VERBOSITY_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct VerbosityReset<'a> {
        _guard: MutexGuard<'a, ()>,
        previous: u8,
    }

    impl Drop for VerbosityReset<'_> {
        fn drop(&mut self) { crate::VERBOSITY.store(self.previous, Ordering::Relaxed); }
    }

    fn set_test_verbosity(level: u8) -> VerbosityReset<'static> {
        let guard = VERBOSITY_TEST_LOCK.lock().unwrap();
        let previous = crate::VERBOSITY.swap(level, Ordering::Relaxed);
        VerbosityReset { _guard: guard, previous }
    }

    #[test]
    fn progress_manager_uses_bars_without_verbose() {
        let _reset = set_test_verbosity(0);
        let manager = ProgressManager::new(&["tool"]);

        assert!(!manager.is_plain_mode());
    }

    #[test]
    fn progress_manager_uses_plain_output_with_verbose() {
        let _reset = set_test_verbosity(1);
        let manager = ProgressManager::new(&["tool"]);

        assert!(manager.is_plain_mode());
    }

    #[test]
    fn finish_unchanged_in_plain_mode_does_not_panic() {
        let _reset = set_test_verbosity(1);
        let manager = ProgressManager::new(&["tool"]);
        let progress = manager.add_package("tool");
        progress.finish_unchanged("1.2.3");
    }

    #[test]
    fn progress_manager_retains_finished_bars_across_suspend() {
        let _reset = set_test_verbosity(0);
        let manager = ProgressManager::new(&["tool"]);
        let progress = manager.add_package("tool");
        let weak_bar = progress.bar.as_ref().unwrap().downgrade();

        progress.finish_unchanged("1.2.3");
        drop(progress);

        manager.suspend(|| {});
        assert!(weak_bar.upgrade().is_some());
        drop(manager);
        assert!(weak_bar.upgrade().is_none());
    }
}
