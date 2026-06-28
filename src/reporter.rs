//! Styled message output for CLI feedback.
//!
//! Provides macros for different message types:
//! - `hint!` - General information
//! - `done!` - Success message
//! - `fail!` - Error message
//! - `warn!` - Warning message
//! - `step!` - Current operation step
//! - `detail!` - Verbose details (requires at least `-v`)
//! - `debug!` - Debug tracing output (requires `-vv`)

use std::io::{self, Write};

use colored::Colorize;

/// General information message.
#[macro_export]
macro_rules! hint {
    ($($arg:tt)*) => {
        $crate::reporter::print_msg(&format!($($arg)*))
    };
}

/// Success message with green checkmark.
#[macro_export]
macro_rules! done {
    ($($arg:tt)*) => {
        $crate::reporter::print_done(&format!($($arg)*))
    };
}

/// Error message with red cross.
#[macro_export]
macro_rules! fail {
    ($($arg:tt)*) => {
        $crate::reporter::print_fail(&format!($($arg)*))
    };
}

/// Warning message with yellow symbol.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::reporter::print_warn(&format!($($arg)*))
    };
}

/// Step indicator with cyan arrow.
#[macro_export]
macro_rules! step {
    ($($arg:tt)*) => {
        $crate::reporter::print_step(&format!($($arg)*))
    };
}

/// Verbose detail (shown with `-v` or higher).
#[macro_export]
macro_rules! detail {
    ($($arg:tt)*) => {
        if $crate::VERBOSITY.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            $crate::reporter::print_detail(&format!($($arg)*))
        }
    };
}

/// Debug tracing output (only shown with `-vv`).
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::VERBOSITY.load(std::sync::atomic::Ordering::Relaxed) >= 2 {
            $crate::reporter::print_debug(&format!($($arg)*))
        }
    };
}

// Implementation functions

#[doc(hidden)]
pub fn print_msg(msg: &str) { writeln!(io::stderr(), "{msg}").ok(); }

#[doc(hidden)]
pub fn print_done(msg: &str) { print_with_prefix("✔".green(), msg); }

#[doc(hidden)]
pub fn print_fail(msg: &str) { print_with_prefix("✖".red(), msg); }

#[doc(hidden)]
pub fn print_warn(msg: &str) { print_with_prefix("⚠".yellow(), msg); }

#[doc(hidden)]
pub fn print_step(msg: &str) { print_with_prefix("==>".cyan().bold(), msg); }

#[doc(hidden)]
pub fn print_skip(msg: &str) { print_with_prefix("=".dimmed(), msg); }

#[doc(hidden)]
pub fn print_detail(msg: &str) { print_with_prefix("  ->".normal(), msg); }

#[doc(hidden)]
#[allow(dead_code)]
pub fn print_debug(msg: &str) { print_with_prefix("    ··".dimmed(), msg); }

#[doc(hidden)]
pub fn format_error_chain(error: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut messages = Vec::new();
    let mut source = error.source();

    while let Some(err) = source {
        let message = err.to_string();
        if messages.last() != Some(&message) {
            messages.push(message);
        }
        source = err.source();
    }

    messages
}

#[doc(hidden)]
pub fn print_error_chain(error: &(dyn std::error::Error + 'static)) {
    if crate::VERBOSITY.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return;
    }

    let messages = format_error_chain(error);
    if messages.is_empty() {
        return;
    }

    print_detail("Caused by:");
    for (idx, message) in messages.iter().enumerate() {
        print_detail(&format!("  {}. {}", idx + 1, message));
    }
}

fn print_with_prefix(prefix: impl std::fmt::Display, msg: &str) {
    let mut stderr = io::stderr();
    if let Some(rest) = msg.strip_prefix('\n') {
        writeln!(stderr).ok();
        writeln!(stderr, "{prefix} {rest}").ok();
    } else {
        writeln!(stderr, "{prefix} {msg}").ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner")]
    struct Inner;

    #[test]
    fn format_error_chain_returns_sources() {
        let error = Outer(Inner);

        assert_eq!(format_error_chain(&error), vec!["inner".to_string()]);
    }

    #[derive(Debug, thiserror::Error)]
    #[error("top")]
    struct DuplicateTop(#[source] DuplicateOuter);

    #[derive(Debug, thiserror::Error)]
    #[error("same")]
    struct DuplicateOuter(#[source] DuplicateInner);

    #[derive(Debug, thiserror::Error)]
    #[error("same")]
    struct DuplicateInner;

    #[test]
    fn format_error_chain_deduplicates_consecutive_messages() {
        let error = DuplicateTop(DuplicateOuter(DuplicateInner));

        assert_eq!(format_error_chain(&error), vec!["same".to_string()]);
    }
}
