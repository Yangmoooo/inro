use std::fs;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Local};
use chrono_humanize::{Accuracy, HumanTime, Tense};
use colored::{ColoredString, Colorize};

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
        let local_registry_dir = &layout.local_registry_dir;

        match &self.command {
            SourceSubCommand::List { check_remote } => {
                // Collect local registry files
                let local_files = if local_registry_dir.exists() {
                    fs::read_dir(local_registry_dir)?
                        .filter_map(|entry| entry.ok())
                        .filter(|entry| {
                            entry.path().extension().and_then(|s| s.to_str()) == Some("toml")
                        })
                        .map(|entry| {
                            entry.path().file_stem().unwrap().to_string_lossy().to_string()
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                };

                if upstreams.is_empty() && local_files.is_empty() {
                    println!("No sources configured");
                    return Ok(());
                }

                struct Row {
                    type_str: &'static str,
                    name: String,
                    enabled_plain: &'static str,
                    enabled_display: ColoredString,
                    last_update_plain: String,
                    last_update_display: ColoredString,
                    url_path: String,
                }

                let mut rows: Vec<Row> = Vec::new();

                for upstream in upstreams {
                    let cached_name = format!("{:02}-{}.toml", upstream.priority, upstream.name);
                    let cached_path = upstream_registry_dir.join(&cached_name);

                    let (enabled_plain, enabled_display) =
                        if upstream.enabled { ("Yes", "Yes".green()) } else { ("No", "No".red()) };

                    let (last_update_plain, last_update_display) = if cached_path.exists() {
                        let metadata = fs::metadata(&cached_path)?;
                        let modified = metadata.modified()?;
                        let datetime: DateTime<Local> = modified.into();
                        let human_time = HumanTime::from(datetime);
                        let time_str = human_time.to_text_en(Accuracy::Rough, Tense::Past);

                        if *check_remote {
                            match check_remote_update(&upstream.url, &cached_path) {
                                Ok(true) => {
                                    let s = format!("{} (update available)", time_str);
                                    let d = s.as_str().yellow();
                                    (s, d)
                                }
                                Ok(false) => {
                                    let s = format!("{} (up-to-date)", time_str);
                                    let d = s.as_str().green();
                                    (s, d)
                                }
                                Err(_) => {
                                    let s = format!("{} (check failed)", time_str);
                                    let d = s.as_str().red();
                                    (s, d)
                                }
                            }
                        } else {
                            let d = time_str.as_str().normal();
                            (time_str, d)
                        }
                    } else {
                        ("Not cached".to_string(), "Not cached".normal())
                    };

                    rows.push(Row {
                        type_str: "Remote",
                        name: upstream.name.clone(),
                        enabled_plain,
                        enabled_display,
                        last_update_plain,
                        last_update_display,
                        url_path: upstream.url.clone(),
                    });
                }

                for local_file in local_files {
                    let local_path = local_registry_dir.join(format!("{}.toml", local_file));
                    let metadata = fs::metadata(&local_path)?;
                    let modified = metadata.modified()?;
                    let datetime: DateTime<Local> = modified.into();
                    let human_time = HumanTime::from(datetime);
                    let time_str = human_time.to_text_en(Accuracy::Rough, Tense::Past);
                    let last_update_display = time_str.as_str().normal();

                    rows.push(Row {
                        type_str: "Local",
                        name: local_file,
                        enabled_plain: "Always",
                        enabled_display: "Always".cyan(),
                        last_update_plain: time_str,
                        last_update_display,
                        url_path: local_path.display().to_string(),
                    });
                }

                let col0_w =
                    rows.iter().map(|r| r.type_str.len()).max().unwrap_or(0).max("Type".len());
                let col1_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(0).max("Name".len());
                let col2_w = rows
                    .iter()
                    .map(|r| r.enabled_plain.len())
                    .max()
                    .unwrap_or(0)
                    .max("Enabled".len());
                let col3_w = rows
                    .iter()
                    .map(|r| r.last_update_plain.len())
                    .max()
                    .unwrap_or(0)
                    .max("Last Update".len());
                let col4_w =
                    rows.iter().map(|r| r.url_path.len()).max().unwrap_or(0).max("URL/Path".len());

                println!(
                    "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
                    "Type",
                    "Name",
                    "Enabled",
                    "Last Update",
                    "URL/Path",
                    w0 = col0_w,
                    w1 = col1_w,
                    w2 = col2_w,
                    w3 = col3_w,
                    w4 = col4_w
                );
                println!(
                    "{:-<w0$}  {:-<w1$}  {:-<w2$}  {:-<w3$}  {:-<w4$}",
                    "",
                    "",
                    "",
                    "",
                    "",
                    w0 = col0_w,
                    w1 = col1_w,
                    w2 = col2_w,
                    w3 = col3_w,
                    w4 = col4_w
                );

                for row in &rows {
                    let enabled_pad = " ".repeat(col2_w.saturating_sub(row.enabled_plain.len()));
                    let update_pad = " ".repeat(col3_w.saturating_sub(row.last_update_plain.len()));
                    println!(
                        "{:<w0$}  {:<w1$}  {}{}  {}{}  {}",
                        row.type_str,
                        row.name,
                        row.enabled_display,
                        enabled_pad,
                        row.last_update_display,
                        update_pad,
                        row.url_path,
                        w0 = col0_w,
                        w1 = col1_w
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
                    if !upstream.enabled {
                        hint!("Skipping disabled source '{}'", upstream.name);
                        continue;
                    }

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
            SourceSubCommand::Enable { name } => {
                enable_disable_source(&layout, name, true)?;
            }
            SourceSubCommand::Disable { name } => {
                enable_disable_source(&layout, name, false)?;
            }
        }

        Ok(())
    }
}

fn check_remote_update(url: &str, local_path: &std::path::Path) -> Result<bool> {
    // Download remote file to temp location
    let temp_dir = tempfile::tempdir()?;
    let remote_path = download_file(url, temp_dir.path())?;

    // Compare file contents
    let local_content = fs::read(local_path)?;
    let remote_content = fs::read(&remote_path)?;

    Ok(local_content != remote_content)
}

fn enable_disable_source(layout: &InroLayout, name: &str, enable: bool) -> Result<()> {
    let config_path = &layout.config_path;

    if !config_path.exists() {
        return Err(anyhow!("Config file not found. Please run 'inro source update' first."));
    }

    // Read the config file
    let content = fs::read_to_string(config_path)?;
    let mut config: Config = toml::from_str(&content)?;

    // Find the upstream by name
    let upstream = config
        .upstreams
        .iter_mut()
        .find(|u| u.name == name)
        .ok_or_else(|| anyhow!("Source '{}' not found", name))?;

    upstream.enabled = enable;

    // Write back to config file
    let new_content = toml::to_string_pretty(&config)?;
    fs::write(config_path, new_content)?;

    if enable {
        done!("Source '{name}' enabled");
    } else {
        done!("Source '{name}' disabled");
    }

    Ok(())
}
