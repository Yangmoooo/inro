use anyhow::{Result, anyhow};
use chrono_humanize::HumanTime;
use colored::Colorize;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::registry::Registry;
use crate::remotes::create_provider;
use crate::utils::{format_date, terminal_link};
use crate::{done, step, warn};

pub struct ShowCommand {
    pub name: String,
}

const REMOTE_DISPLAY_LIMIT: usize = 5;

impl CommandHandler for ShowCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let registry = Registry::load(&layout)?;

        let pkg_def = registry
            .pkgs
            .get(&self.name)
            .ok_or_else(|| anyhow!("Package '{}' not found in registry.", self.name))?;

        let width = 12;
        let resolved = pkg_def.resolve(&self.name);
        println!("{:<width$}{}", "Name:".bold(), self.name.green());
        println!("{:<width$}{}", "Source:".bold(), resolved.remote);

        let bins_str = resolved.bin.iter().map(|b| b.link.as_str()).collect::<Vec<_>>().join(", ");
        println!("{:<width$}{}", "Binaries:".bold(), bins_str);

        print!("{:<width$}", "Status:".bold());
        let manifest = Manifest::load(&layout.manifest_path).ok();
        let install_state = manifest.as_ref().and_then(|m| m.pkgs.get(&self.name));
        if let Some(state) = install_state {
            // show status
            print!("{}", "Installed".green());
            if let Some(curr) = &state.current_version {
                if let Some(receipt) = state.versions.get(curr) {
                    let pinned_indicator = if state.pinned { " [Pinned]" } else { "" };
                    println!(" at {}{}", format_date(&receipt.installed_at), pinned_indicator);
                }
            } else {
                println!(" but no active version");
            }

            // show all versions
            let mut local_versions: Vec<_> = state.versions.keys().collect();
            local_versions.sort();
            local_versions.reverse();
            println!("{}:", "Installed Versions".bold());

            if local_versions.is_empty() {
                println!("  (none)");
            } else {
                let versions_str = local_versions
                    .iter()
                    .map(|ver| {
                        let is_current = state.current_version.as_ref() == Some(ver);
                        if is_current {
                            format!("{ver}*").green().bold().to_string()
                        } else {
                            (*ver).clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("  {versions_str}");
            }
        } else {
            println!("{}", "Not Installed".dimmed());
        }

        step!("\nFetching remote info...");
        let provider = create_provider(&resolved.remote)?;
        match provider.list_versions(pkg_def) {
            Ok(versions) => {
                done!("Recent available versions:");
                let has_more = versions.len() > REMOTE_DISPLAY_LIMIT;
                for ver in versions.iter().take(REMOTE_DISPLAY_LIMIT) {
                    let clickable_tag = terminal_link(&ver.tag, &ver.url);
                    let time_display = format!("({})", HumanTime::from(ver.published_at)).dimmed();
                    let pre_tag = if ver.prerelease {
                        " (pre-release)".dimmed().to_string()
                    } else {
                        String::new()
                    };
                    println!("  - {clickable_tag}  {time_display}{pre_tag}");
                }
                if has_more {
                    println!("  {}", "... (and more, check the remote for details)".dimmed());
                }
            }
            Err(e) => warn!("Failed to fetch remote versions: {e}"),
        }

        Ok(())
    }
}
