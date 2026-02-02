//! Progress tracking for concurrent package operations.
//!
//! Provides multi-progress bar display for install/update commands.

use std::sync::Arc;
use std::time::Duration;

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
}

/// A handle to update a single package's progress
#[derive(Clone)]
pub struct PkgProgress {
    bar: ProgressBar,
    name: String,
    max_name_width: usize,
}

impl PkgProgress {
    /// Update the operation phase
    pub fn set_phase(&self, phase: OpPhase) {
        let symbol = phase.symbol();
        let msg = phase.message();
        let name = &self.name;
        let w = self.max_name_width;

        match phase {
            OpPhase::Downloading => {
                self.bar.set_style(download_style());
                self.bar.set_message(format!("{name:<w$}"));
            }
            _ => {
                self.bar.set_style(spinner_style());
                self.bar.enable_steady_tick(Duration::from_millis(80));
                self.bar.set_message(format!("{symbol} {name:<w$}  {msg}"));
            }
        }
    }

    /// Set the total size for download progress
    pub fn set_length(&self, len: u64) { self.bar.set_length(len); }

    /// Increment download progress
    pub fn inc(&self, delta: u64) { self.bar.inc(delta); }

    /// Mark as completed with version
    pub fn finish_success(&self, version: &str) {
        let name = &self.name;
        let w = self.max_name_width;
        self.bar.set_style(status_style());
        self.bar.finish_with_message(format!("✓ {name:<w$}  {version}"));
    }

    /// Mark as failed with error message
    pub fn finish_error(&self, err: &str) {
        let name = &self.name;
        let w = self.max_name_width;
        self.bar.set_style(status_style());
        self.bar.finish_with_message(format!("✗ {name:<w$}  {err}"));
    }
}

/// Manager for multi-package progress display
pub struct ProgressManager {
    multi: Arc<MultiProgress>,
    name_width: usize,
}

impl ProgressManager {
    /// Create a new progress manager with calculated name width
    pub fn new(pkg_names: &[&str]) -> Self {
        let name_width = pkg_names.iter().map(|n| n.len()).max().unwrap_or(0);
        Self { multi: Arc::new(MultiProgress::new()), name_width }
    }

    /// Add a package to track
    pub fn add_package(&self, name: &str) -> PkgProgress {
        let bar = self.multi.add(ProgressBar::new(0));
        bar.set_style(spinner_style());

        let w = self.name_width;
        bar.set_message(format!("◦ {name:<w$}  pending"));

        PkgProgress { bar, name: name.to_string(), max_name_width: self.name_width }
    }
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
