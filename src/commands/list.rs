use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::report;

pub struct ListCommand {
    pub layout: InroLayout,
}

impl CommandHandler for ListCommand {
    fn handle(&self) -> Result<()> {
        let manifest = Manifest::load(&self.layout.manifest_path)?;

        if manifest.dans.is_empty() {
            report!(MsgType::Warning, "No packages installed");
            return Ok(());
        }

        // prepare
        let mut rows = Vec::new();
        let mut max_name_len = 4; // "Name" length
        let mut max_ver_len = 7; // "Version" length

        let mut sorted_dans: Vec<_> = manifest.dans.iter().collect();
        sorted_dans.sort_by_key(|(name, _)| *name);

        for (name, state) in sorted_dans {
            // get current verison and extra version (if exist)
            let (ver_display, extra_ver) = match &state.current_version {
                Some(v) => {
                    let count = state.versions.len();
                    if count > 1 {
                        (v.clone(), format!("(+{} others)", count - 1))
                    } else {
                        (v.clone(), String::new())
                    }
                }
                None => ("(none)".dimmed().to_string(), String::new()),
            };

            // get source info, actually remote display from current version receipt
            let source_display = if let Some(ver) = &state.current_version {
                state
                    .versions
                    .get(ver)
                    .map(|r| r.remote.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            max_name_len = max_name_len.max(name.len());
            max_ver_len = max_ver_len.max(ver_display.len());

            rows.push((name, ver_display, source_display, extra_ver));
        }

        // print header
        println!(
            "{}  {}  {}",
            pad_str("Name", max_name_len).bold(),
            pad_str("Version", max_ver_len).bold(),
            "Source".bold()
        );
        println!(
            "{}  {}  {}",
            "-".repeat(max_name_len),
            "-".repeat(max_ver_len),
            "-".repeat(10)
        );

        // print rows
        for (name, ver, source, extra) in rows {
            println!(
                "{}  {}  {} {}",
                pad_str(name, max_name_len).green(),
                pad_str(&ver, max_ver_len),
                source,
                extra.dimmed()
            );
        }

        Ok(())
    }
}

fn pad_str(s: &str, width: usize) -> String {
    format!("{:<width$}", s, width = width)
}
