use std::env;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, ExitStatus};

use anyhow::{Context, Result, anyhow, bail};

#[cfg(not(windows))]
const FALLBACK_EDITORS: &[&str] = &["vi"];

#[cfg(windows)]
const FALLBACK_EDITORS: &[&str] = &["edit.exe", "notepad.exe"];

pub fn edit_file(path: &Path) -> Result<()> {
    if let Some((program, args)) = configured_editor()? {
        let status = Command::new(&program)
            .args(&args)
            .arg(path)
            .status()
            .with_context(|| format!("Failed to launch editor '{program}'"))?;
        return require_success(&program, status);
    }

    for program in FALLBACK_EDITORS {
        match Command::new(program).arg(path).status() {
            Ok(status) => return require_success(program, status),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to launch editor '{program}'"));
            }
        }
    }

    bail!("No text editor found; set VISUAL or EDITOR")
}

fn configured_editor() -> Result<Option<(String, Vec<String>)>> {
    for key in ["VISUAL", "EDITOR"] {
        let Some(value) = env::var_os(key) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let value =
            value.into_string().map_err(|_| anyhow!("{key} contains non-UTF-8 characters"))?;
        let mut parts = shell_words::split(&value)
            .with_context(|| format!("Failed to parse {key} editor command"))?;
        if parts.is_empty() {
            continue;
        }
        let program = parts.remove(0);
        return Ok(Some((program, parts)));
    }
    Ok(None)
}

fn require_success(program: &str, status: ExitStatus) -> Result<()> {
    if status.success() { Ok(()) } else { bail!("Editor '{program}' exited with {status}") }
}
