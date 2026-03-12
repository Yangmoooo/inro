//! Styled message output for CLI feedback.
//!
//! Provides macros for different message types:
//! - `hint!` - General information
//! - `done!` - Success message
//! - `fail!` - Error message
//! - `warn!` - Warning message
//! - `step!` - Current operation step
//! - `detail!` - Verbose details (requires `-v`)

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

/// Verbose detail (only shown with `-v`).
#[macro_export]
macro_rules! detail {
    ($($arg:tt)*) => {
        if $crate::VERBOSITY.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            $crate::reporter::print_detail(&format!($($arg)*))
        }
    };
}

// Implementation functions

#[doc(hidden)]
pub fn print_msg(msg: &str) {
    writeln!(io::stderr(), "{msg}").ok();
}

#[doc(hidden)]
pub fn print_done(msg: &str) {
    print_with_prefix("✔".green(), msg);
}

#[doc(hidden)]
pub fn print_fail(msg: &str) {
    print_with_prefix("✖".red(), msg);
}

#[doc(hidden)]
pub fn print_warn(msg: &str) {
    print_with_prefix("⚠".yellow(), msg);
}

#[doc(hidden)]
pub fn print_step(msg: &str) {
    print_with_prefix("==>".cyan().bold(), msg);
}

#[doc(hidden)]
pub fn print_detail(msg: &str) {
    print_with_prefix("  ->".normal(), msg);
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
