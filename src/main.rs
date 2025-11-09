mod cli;

use clap::{CommandFactory, Parser};

use cli::{Args, Command};

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match &args.command {
        Some(command) => {
            let mut failed_pkgs = Vec::new();

            match command {
                Command::Install { names } => {
                    println!("Attempting to install {} package(s)...", names.len());
                    for name in names {
                        match handle_install(name) {
                            Ok(_) => {}
                            Err(e) => {
                                eprintln!("Failed to install package '{name}': {e}");
                                failed_pkgs.push(name);
                            }
                        }
                    }
                }
                Command::Update { names } => {
                    if names.is_empty() {
                        println!("Updating all packages...");
                        handle_update_all()?;
                    } else {
                        println!("Attempting to update {} package(s)...", names.len());
                        for name in names {
                            match handle_update_one(name) {
                                Ok(_) => {}
                                Err(e) => {
                                    eprintln!("Failed to update package '{name}': {e}");
                                    failed_pkgs.push(name);
                                }
                            }
                        }
                    }
                }
                _ => println!("Not implemented!"),
            }

            if !failed_pkgs.is_empty() {
                eprintln!("\nSummary: The following packages failed to install: {failed_pkgs:?}");
                anyhow::bail!("One or more packages failed to install.");
            }
        }
        None => {
            let _ = Args::command().print_help();
        }
    }

    Ok(())
}

fn handle_install(name: &str) -> anyhow::Result<()> {
    println!("Installing {name}...");
    Ok(())
}

fn handle_update_all() -> anyhow::Result<()> {
    println!("Updating all packages...");
    Ok(())
}

fn handle_update_one(name: &str) -> anyhow::Result<()> {
    println!("Updating {name}...");
    Ok(())
}
