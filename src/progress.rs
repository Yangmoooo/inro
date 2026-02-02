//! Progress tracking for concurrent package operations.
//!
//! Provides multi-progress bar display for install/update commands.

use std::sync::Arc;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Status of a package operation
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum PkgStatus {
    Pending,
    Fetching,
    Downloading,
    Extracting,
    Done,
    Failed(String),
}

impl PkgStatus {
    fn symbol(&self) -> &'static str {
        match self {
            PkgStatus::Pending => "◦",
            PkgStatus::Fetching => "⠸",
            PkgStatus::Downloading => "↓",
            PkgStatus::Extracting => "⠸",
            PkgStatus::Done => "✓",
            PkgStatus::Failed(_) => "✗",
        }
    }

    fn message(&self) -> &'static str {
        match self {
            PkgStatus::Pending => "pending",
            PkgStatus::Fetching => "fetching release...",
            PkgStatus::Downloading => "", // handled by progress bar
            PkgStatus::Extracting => "extracting...",
            PkgStatus::Done => "done",
            PkgStatus::Failed(_) => "failed",
        }
    }
}

/// A handle to update a single package's progress
#[derive(Clone)]
pub struct PkgProgress {
    bar: ProgressBar,
    name: String,
    max_name_len: usize,
}

impl PkgProgress {
    /// Update the status (for non-download phases)
    pub fn set_status(&self, status: PkgStatus) {
        let symbol = status.symbol();
        let msg = status.message();
        let name = &self.name;
        let width = self.max_name_len;

        match status {
            PkgStatus::Downloading => {
                // Switch to download progress style
                self.bar.set_style(download_style());
                self.bar.set_message(format!("{name:<width$}"));
            }
            PkgStatus::Done | PkgStatus::Failed(_) => {
                self.bar.set_style(status_style());
                self.bar.finish_with_message(format!("{symbol} {name:<width$}  {msg}"));
            }
            _ => {
                self.bar.set_style(spinner_style());
                self.bar.enable_steady_tick(Duration::from_millis(80));
                self.bar.set_message(format!("{symbol} {name:<width$}  {msg}"));
            }
        }
    }

    /// Set the total size for download progress
    pub fn set_length(&self, len: u64) { self.bar.set_length(len); }

    /// Update download progress
    #[allow(dead_code)]
    pub fn set_position(&self, pos: u64) { self.bar.set_position(pos); }

    /// Increment download progress
    pub fn inc(&self, delta: u64) { self.bar.inc(delta); }

    /// Mark as completed
    pub fn finish_success(&self, version: &str) {
        let name = &self.name;
        let width = self.max_name_len;
        self.bar.set_style(status_style());
        self.bar.finish_with_message(format!("✓ {name:<width$}  {version}"));
    }

    /// Mark as failed
    pub fn finish_error(&self, err: &str) {
        let name = &self.name;
        let width = self.max_name_len;
        self.bar.set_style(status_style());
        self.bar.finish_with_message(format!("✗ {name:<width$}  {err}"));
    }
}

/// Manager for multi-package progress display
pub struct ProgressManager {
    multi: Arc<MultiProgress>,
    max_name_len: usize,
}

impl ProgressManager {
    /// Create a new progress manager
    pub fn new() -> Self { Self { multi: Arc::new(MultiProgress::new()), max_name_len: 20 } }

    /// Create a new progress manager with known name length
    #[allow(dead_code)]
    pub fn with_max_len(max_name_len: usize) -> Self {
        Self { multi: Arc::new(MultiProgress::new()), max_name_len }
    }

    /// Set the max name length for alignment
    #[allow(dead_code)]
    pub fn set_max_name_len(&mut self, len: usize) { self.max_name_len = len; }

    /// Add a package to track
    pub fn add_package(&self, name: &str) -> PkgProgress {
        let bar = self.multi.add(ProgressBar::new(0));
        bar.set_style(spinner_style());

        let width = self.max_name_len;
        bar.set_message(format!("◦ {name:<width$}  pending"));

        PkgProgress { bar, name: name.to_string(), max_name_len: self.max_name_len }
    }

    /// Clear all progress bars (call before printing summary)
    #[allow(dead_code)]
    pub fn clear(&self) { self.multi.clear().ok(); }
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner().template("  {msg}").unwrap()
}

fn status_style() -> ProgressStyle { ProgressStyle::default_bar().template("  {msg}").unwrap() }

fn download_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("  ↓ {msg}  [{bar:20.cyan/dim}] {bytes}/{total_bytes} ({bytes_per_sec})")
        .unwrap()
        .progress_chars("━╸─")
}
