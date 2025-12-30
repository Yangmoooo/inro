use std::cmp::max;

use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::report;

pub struct ListCommand {}

struct ListEntry {
    name: String,
    ver_display: String,
    remote_str: String,
    extra_ver: String,
}

impl CommandHandler for ListCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let manifest = Manifest::load(&layout.manifest_path)?;

        if manifest.pkgs.is_empty() {
            report!(MsgType::Warning, "No packages installed");
            return Ok(());
        }

        let mut results = Vec::new();
        let mut max_name_len = 4;
        let mut max_ver_len = 7;

        let mut sorted_pkgs: Vec<_> = manifest.pkgs.iter().collect();
        sorted_pkgs.sort_by_key(|(name, _)| *name);

        for (name, state) in sorted_pkgs {
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
                None => ("(none)".to_string(), String::new()),
            };

            // get source info, actually remote display from current version receipt
            let remote_str = state
                .current_version
                .as_ref()
                .and_then(|v| state.versions.get(v))
                .or_else(|| state.versions.values().next())
                .map(|r| r.remote.to_string())
                .unwrap_or_default();

            max_name_len = max(max_name_len, name.len());
            max_ver_len = max(max_ver_len, ver_display.len());

            results.push(ListEntry { name: name.clone(), ver_display, remote_str, extra_ver });
        }

        // print header
        println!(
            "{:<max_name_len$}  {:<max_ver_len$}  {}",
            "Name".bold(),
            "Version".bold(),
            "Source".bold(),
        );
        println!("{:-<max_name_len$}  {:-<max_ver_len$}  {:-<30}", "", "", "",);

        // print rows
        for res in results {
            println!(
                "{:<max_name_len$}  {:<max_ver_len$}  {} {}",
                res.name.green(),
                res.ver_display,
                res.remote_str,
                res.extra_ver.dimmed(),
            );
        }

        Ok(())
    }
}
