use anyhow::{Result, anyhow};
use chrono_humanize::HumanTime;
use colored::Colorize;

use super::CommandHandler;
use crate::layout::InroLayout;
use crate::manifest::Manifest;
use crate::registry::Registry;
use crate::remotes::create_provider;
use crate::report;
use crate::utils::{format_date, terminal_link};

pub struct InfoCommand {
    pub name: String,
    pub layout: InroLayout,
}

impl CommandHandler for InfoCommand {
    fn handle(&self) -> Result<()> {
        let registry = Registry::load(&self.layout)?;

        let dan_def = registry
            .dans
            .get(&self.name)
            .ok_or_else(|| anyhow!("Package '{}' not found in registry.", self.name))?;

        let width = 15;
        let resolved = dan_def.clone().resolve(&self.name);
        println!("{:<width$}{}", "Name:".bold(), self.name.green());
        println!("{:<width$}{}", "Source:".bold(), resolved.remote);

        let bins_str = resolved
            .bin
            .iter()
            .map(|b| b.link.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("{:<width$}{}", "Binaries:".bold(), bins_str);

        print!("{:<width$}", "Status:".bold());
        let manifest = Manifest::load(&self.layout.manifest_path).ok();
        let install_state = manifest.as_ref().and_then(|m| m.dans.get(&self.name));
        if let Some(state) = install_state {
            if let Some(curr) = &state.current_version {
                println!("{} (active version: {})", "Installed".green(), curr);
                if let Some(receipt) = state.versions.get(curr) {
                    println!(
                        "{:<width$}{}",
                        "Installed At:".bold(),
                        format_date(&receipt.installed_at)
                    );
                    println!(
                        "{:<width$}{}",
                        "Location:".bold(),
                        receipt.install_dir.display()
                    );
                }
            } else {
                println!("{}", "Installed (No active version)".yellow());
            }
        } else {
            println!("{}", "Not Installed".dimmed());
        }

        report!(MsgType::Step, "\nFetching remote info...");
        let provider = create_provider(&resolved.remote)?;
        match provider.list_versions(dan_def) {
            Ok(versions) => {
                report!(MsgType::Success, "Recent available versions:");
                for ver in versions {
                    let clickable_tag = terminal_link(&ver.tag, &ver.url);
                    let time_display = format!("({})", HumanTime::from(ver.published_at)).dimmed();
                    let pre_tag = if ver.prerelease {
                        " (pre-release)".yellow().to_string()
                    } else {
                        String::new()
                    };
                    println!("  - {clickable_tag}  {time_display}{pre_tag}");
                }
            }
            Err(e) => report!(MsgType::Warning, "Failed to fetch remote versions: {e}"),
        }

        Ok(())
    }
}
