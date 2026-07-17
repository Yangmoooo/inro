//! Cross-process exclusive lock for inro state mutations.
//!
//! Every mutation of live `manifest.json`, local registry, or linked binary
//! state happens while this lock is held. Most writing commands hold it for
//! their full lifetime; interactive source editing works on an isolated
//! staging file and acquires the lock only while taking its snapshot and
//! validating/committing it. Concurrent invocations therefore cannot stomp
//! on each other's live writes.
//!
//! Read-only commands (`list`, `show`, `search`, `export`, `doctor` without
//! `--fix`, `generate`) intentionally skip the lock since the on-disk files are
//! always written via tmp+rename, so a reader sees either the old or the
//! new version, never a half-written one.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::layout::InroLayout;
use crate::warn;

/// RAII guard for the global state lock. The lock is released when the
/// guard is dropped (or when the process exits).
pub struct StateLock {
    _file: File,
}

/// Acquire the global state lock, blocking if another inro instance
/// already holds it. The user is told once we start waiting so a long
/// pause does not look like a hang.
pub fn acquire(layout: &InroLayout) -> Result<StateLock> {
    let lock_path = lock_path(layout);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create lock dir: {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("Failed to open lock file: {}", lock_path.display()))?;

    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            warn!("Another inro process is running, waiting for it to finish...");
            file.lock()
                .with_context(|| format!("Failed to acquire lock: {}", lock_path.display()))?;
        }
        Err(TryLockError::Error(e)) => {
            return Err(e)
                .with_context(|| format!("Failed to lock state file: {}", lock_path.display()));
        }
    }

    Ok(StateLock { _file: file })
}

fn lock_path(layout: &InroLayout) -> PathBuf { layout.inro_dir.join("inro.lock") }

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    fn test_layout(root: &std::path::Path) -> InroLayout {
        let inro_dir = root.join("inro");
        InroLayout {
            home_dir: root.to_path_buf(),
            config_path: inro_dir.join("config.toml"),
            manifest_path: inro_dir.join("manifest.json"),
            pkgs_dir: inro_dir.join("pkgs"),
            managed_registry_dir: inro_dir.join("registry"),
            user_registry_dir: inro_dir.join("registry.d"),
            inro_dir,
        }
    }

    #[test]
    fn acquire_creates_lock_file_under_inro_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = test_layout(tmp.path());

        let _guard = acquire(&layout).unwrap();

        let expected = layout.inro_dir.join("inro.lock");
        assert!(expected.exists(), "lock file should be created at {}", expected.display());
    }

    #[test]
    fn second_acquire_blocks_until_first_is_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = test_layout(tmp.path());

        let first = acquire(&layout).unwrap();

        // Spawn a second waiter on a thread; it must not return until we drop
        // the first guard.
        let layout_clone = layout.clone();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _g = acquire(&layout_clone).unwrap();
            tx.send(()).unwrap();
        });

        // Give the waiter time to actually call lock(); it should still be
        // blocked because we hold `first`.
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "second acquire should block while first guard is alive",
        );

        let started_release = Instant::now();
        drop(first);

        // After release, the waiter should make progress quickly.
        rx.recv_timeout(Duration::from_secs(2)).expect("second acquire never completed");
        assert!(
            started_release.elapsed() < Duration::from_secs(2),
            "second acquire should unblock promptly after the first lock is released",
        );

        handle.join().unwrap();
    }
}
