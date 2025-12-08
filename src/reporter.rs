use std::io::{self, Write};

use colored::{ColoredString, Colorize};

/// Prints a formatted message to stderr with a specific style.
///
/// Info, Success, Error, Warning, Step and Detail.
///
/// Notice Detail could be ignored when verbosity is set to 0.
///
/// # Examples
///
/// ```
/// report!(MsgType::Success, "Package '{}' installed.", pkg_name);
/// report!(MsgType::Warning, "INRO_GITHUB_TOKEN is not set.");
/// ```
#[macro_export]
macro_rules! report {
    ($msg_type:expr, $fmt:literal $(, $($arg:expr),*)?) => {
        {
            use std::sync::atomic::Ordering;
            use $crate::reporter::{MsgType, report_impl};

            let should_print = match $msg_type {
                MsgType::Detail => $crate::VERBOSITY.load(Ordering::Relaxed) > 0,
                _ => true,
            };
            if should_print {
                report_impl($msg_type, &format!($fmt $(, $($arg),*)?));
            }
        }
    };
}

pub enum MsgType {
    Info,    // General information
    Success, // Something good happened
    Error,   // Something bad happened
    Warning, // Something might be wrong
    Step,    // The current major step of an operation
    Detail,  // A less important detail of a step
}

// Actually do the reporting
#[doc(hidden)]
pub fn report_impl(msg_type: MsgType, msg_content: &str) {
    match msg_type {
        MsgType::Info => print_raw(msg_content),
        MsgType::Success => print_with_prefix(&"✔".green(), msg_content),
        MsgType::Error => print_with_prefix(&"✖".red(), msg_content),
        MsgType::Warning => print_with_prefix(&"⚠".yellow(), msg_content),
        MsgType::Step => print_with_prefix(&"==>".bold().cyan(), msg_content),
        MsgType::Detail => print_with_prefix(&"  ->".normal(), msg_content),
    }
}

fn print_raw(message: &str) { writeln!(io::stderr(), "{message}").ok(); }

// Make report! can start with '\n'
fn print_with_prefix(prefix: &ColoredString, message: &str) {
    let mut stderr = io::stderr();

    if let Some(stripped_message) = message.strip_prefix('\n') {
        writeln!(stderr).ok();
        writeln!(stderr, "{prefix} {stripped_message}").ok();
    } else {
        writeln!(stderr, "{prefix} {message}").ok();
    }
}
