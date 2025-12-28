use std::cmp::max;

use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::registry::Registry;
use crate::report;

pub struct SearchCommand {
    pub query: String,
    pub layout: InroLayout,
}

struct SearchEntry {
    name: String,
    remote_str: String,
    bins_display: String,
    is_installed: bool,
}

impl CommandHandler for SearchCommand {
    fn handle(&self) -> Result<()> {
        let registry = Registry::load(&self.layout)?;
        if registry.pkgs.is_empty() {
            report!(MsgType::Warning, "Registry is empty.");
            return Ok(());
        }

        let manifest = Manifest::load(&self.layout.manifest_path).ok();

        let query = self.query.to_lowercase();
        let mut results = Vec::new();

        let mut max_name_len = 4;
        let mut max_source_len = 6;

        for (name, pkg_def) in &registry.pkgs {
            let resolved = pkg_def.clone().resolve(name);
            let name_lower = name.to_lowercase();

            let pkg_match = name_lower.contains(&query);

            let mut bin_match = false;
            let mut bin_parts = Vec::new();

            for bin in &resolved.bin {
                let link_name = &bin.link;
                if link_name.to_lowercase().contains(&query) {
                    bin_match = true;
                    bin_parts.push(link_name.clone());
                } else {
                    bin_parts.push(link_name.dimmed().to_string());
                }
            }

            if pkg_match || bin_match {
                let is_installed = manifest
                    .as_ref()
                    .and_then(|m| m.pkgs.get(name))
                    .is_some_and(|state| state.current_version.is_some());
                let remote_str = resolved.remote.to_string();

                max_name_len = max(max_name_len, name.len());
                max_source_len = max(max_source_len, remote_str.len());

                results.push(SearchEntry {
                    name: name.clone(),
                    remote_str,
                    bins_display: bin_parts.join(", "),
                    is_installed,
                });
            }
        }

        if results.is_empty() {
            report!(MsgType::Warning, "No packages found matching '{}'", self.query);
            return Ok(());
        }

        results.sort_by(|a, b| a.name.cmp(&b.name));

        // print header
        println!(
            "{:<3}  {:<max_name_len$}  {:<max_source_len$}  {}",
            "St", // Status
            "Name".bold(),
            "Source".bold(),
            "Binaries".bold(),
        );
        println!("---  {:-<max_name_len$}  {:-<max_source_len$}  {:-<10}", "", "", "",);

        // print rows
        for res in results {
            let status_icon = if res.is_installed { "i".green().bold() } else { " ".normal() };

            println!(
                "[{}]  {:<max_name_len$}  {:<max_source_len$}  {}",
                status_icon, res.name, res.remote_str, res.bins_display,
            );
        }

        Ok(())
    }
}
