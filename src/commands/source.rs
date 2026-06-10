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
        let _lock = match &self.command {
            SourceSubCommand::List { .. } => None,
            _ => Some(crate::lock::acquire(&layout)?),
        };
        let config = Config::load(&layout)?;
        let upstreams = &config.upstreams;
        let managed_registry_dir = &layout.managed_registry_dir;
        let user_registry_dir = &layout.user_registry_dir;

        match &self.command {
            SourceSubCommand::List { check_remote } => {
                // Collect user-written registry files
                let user_files = if user_registry_dir.exists() {
                    fs::read_dir(user_registry_dir)?
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

                if upstreams.is_empty() && user_files.is_empty() {
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
                    let cached_path = managed_registry_dir.join(&cached_name);

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

                for user_file in user_files {
                    let user_path = user_registry_dir.join(format!("{}.toml", user_file));
                    let metadata = fs::metadata(&user_path)?;
                    let modified = metadata.modified()?;
                    let datetime: DateTime<Local> = modified.into();
                    let human_time = HumanTime::from(datetime);
                    let time_str = human_time.to_text_en(Accuracy::Rough, Tense::Past);
                    let last_update_display = time_str.as_str().normal();

                    rows.push(Row {
                        type_str: "Local",
                        name: user_file,
                        enabled_plain: "Always",
                        enabled_display: "Always".cyan(),
                        last_update_plain: time_str,
                        last_update_display,
                        url_path: user_path.display().to_string(),
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

                fs::create_dir_all(managed_registry_dir)?;

                for upstream in upstreams {
                    if !upstream.enabled {
                        hint!("Skipping disabled source '{}'", upstream.name);
                        continue;
                    }

                    match download_file(&upstream.url, managed_registry_dir) {
                        Ok(raw_path) => {
                            let cached_name =
                                format!("{:02}-{}.toml", upstream.priority, upstream.name);
                            let cached_path = managed_registry_dir.join(&cached_name);

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
    use toml_edit::{Array, DocumentMut, InlineTable, Item, Value};

    // Verify the source exists in the effective config (including defaults)
    let config = Config::load(layout)?;
    let upstream_def = config
        .upstreams
        .iter()
        .find(|u| u.name == name)
        .ok_or_else(|| anyhow!("Source '{}' not found", name))?;

    let config_path = &layout.config_path;

    // Parse existing file, or start with an empty document
    let content =
        if config_path.exists() { fs::read_to_string(config_path)? } else { String::new() };
    let mut doc: DocumentMut =
        content.parse().map_err(|e| anyhow!("Failed to parse config file: {e}"))?;

    // Ensure upstreams key exists as an inline array
    let upstreams_item =
        doc.entry("upstreams").or_insert_with(|| Item::Value(Value::Array(Array::new())));
    let upstreams_array = upstreams_item
        .as_array_mut()
        .ok_or_else(|| anyhow!("'upstreams' in config is not an array"))?;

    // Find the entry by name
    let entry_idx = upstreams_array.iter().position(|v| {
        v.as_inline_table().and_then(|t| t.get("name")).and_then(|v| v.as_str()) == Some(name)
    });

    match entry_idx {
        Some(idx) => {
            let table = upstreams_array
                .get_mut(idx)
                .and_then(|v| v.as_inline_table_mut())
                .ok_or_else(|| anyhow!("upstream entry is not an inline table"))?;
            table.insert("enabled", enable.into());
        }
        None => {
            // The entry only exists as a default — materialize it with the enabled flag
            let mut tbl = InlineTable::new();
            tbl.insert("name", upstream_def.name.as_str().into());
            tbl.insert("priority", i64::from(upstream_def.priority).into());
            tbl.insert("url", upstream_def.url.as_str().into());
            tbl.insert("enabled", enable.into());
            upstreams_array.push(Value::InlineTable(tbl));
        }
    }

    // Ensure parent directory exists, then write atomically
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = config_path.with_extension("tmp");
    fs::write(&temp_path, doc.to_string())?;
    fs::rename(&temp_path, config_path)?;

    if enable {
        done!("Source '{name}' enabled");
    } else {
        done!("Source '{name}' disabled");
    }

    Ok(())
}
