use std::fs;

use anyhow::Result;
use colored::Colorize;

use super::CommandHandler;
use crate::cli::SourceSubCommand;
use crate::config::Config;
use crate::layout::InroLayout;
use crate::utils::download_file;
use crate::{done, fail, hint, warn};

pub struct SourceCommand {
    pub command: SourceSubCommand,
}

impl CommandHandler for SourceCommand {
    fn handle(&self) -> Result<()> {
        let layout = InroLayout::new()?;
        let config = Config::load(&layout)?;
        let upstreams = &config.upstreams;
        let upstream_registry_dir = &layout.upstream_registry_dir;

        match &self.command {
            SourceSubCommand::List => {
                if upstreams.is_empty() {
                    println!("No upstream sources configured");
                    return Ok(());
                }

                println!("{:<4}  {:<10}  {:<10}  {:<50}", "Prio", "Name", "Status", "URL");
                println!("{:-<4}  {:-<10}  {:-<10}  {:-<50}", "", "", "", "");

                for upstream in upstreams {
                    let cached_name = format!("{:02}-{}.toml", upstream.priority, upstream.name);
                    let cached_path = upstream_registry_dir.join(&cached_name);

                    let status_display =
                        if cached_path.exists() { "Cached".green() } else { "Not cached".yellow() };

                    println!(
                        "{:<4}  {:<10}  {:<10}  {:<50}",
                        upstream.priority, upstream.name, status_display, upstream.url
                    );
                }
            }
            SourceSubCommand::Update => {
                if upstreams.is_empty() {
                    warn!("No upstream sources configured to update");
                    return Ok(());
                }

                hint!("Updating {} upstream sources...", upstreams.len());

                fs::create_dir_all(upstream_registry_dir)?;

                for upstream in upstreams {
                    match download_file(&upstream.url, upstream_registry_dir) {
                        Ok(raw_path) => {
                            let cached_name =
                                format!("{:02}-{}.toml", upstream.priority, upstream.name);
                            let cached_path = upstream_registry_dir.join(&cached_name);

                            if let Err(e) = fs::rename(&raw_path, cached_path) {
                                fail!(
                                    "Downloaded '{}' but failed to rename it: {}",
                                    upstream.name,
                                    e
                                );
                                let _ = fs::remove_file(raw_path);
                            } else {
                                done!("'{}' Updated", upstream.name);
                            }
                        }
                        Err(e) => {
                            fail!("Failed to update '{}': {}", upstream.name, e);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
