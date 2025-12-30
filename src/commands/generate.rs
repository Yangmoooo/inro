use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Command, CommandFactory};
use clap_complete::{Shell, generate_to};
use clap_mangen::Man;

use super::CommandHandler;
use crate::cli::{Cli, Generator};

pub struct GenerateCommand {
    pub generator: Generator,
    pub out: PathBuf,
}

impl CommandHandler for GenerateCommand {
    fn handle(&self) -> Result<()> {
        let mut cmd = Cli::command();
        let bin_name = "inro";

        fs::create_dir_all(&self.out)?;

        match self.generator {
            Generator::Man => {
                generate_man(cmd.clone(), bin_name, &self.out)?;
                println!("Man page generated at: {}", self.out.display());
            }
            Generator::Bash => generate_shell(Shell::Bash, &mut cmd, bin_name, &self.out)?,
            Generator::Zsh => generate_shell(Shell::Zsh, &mut cmd, bin_name, &self.out)?,
            Generator::Fish => generate_shell(Shell::Fish, &mut cmd, bin_name, &self.out)?,
            Generator::PowerShell => {
                generate_shell(Shell::PowerShell, &mut cmd, bin_name, &self.out)?
            }
        }

        Ok(())
    }
}

fn generate_man(cmd: Command, name: &str, out_dir: &Path) -> Result<()> {
    let man = Man::new(cmd.clone());
    let mut buffer: Vec<u8> = Default::default();
    man.render(&mut buffer)?;

    let filename = format!("{name}.1");
    let path = out_dir.join(&filename);
    fs::write(&path, buffer)?;

    for subcommand in cmd.get_subcommands() {
        if subcommand.is_hide_set() {
            continue;
        }
        let sub_name = format!("{name}-{}", subcommand.get_name());
        generate_man(subcommand.clone(), &sub_name, out_dir)?;
    }
    Ok(())
}

fn generate_shell(shell: Shell, cmd: &mut Command, bin_name: &str, out_dir: &Path) -> Result<()> {
    let path = generate_to(shell, cmd, bin_name, out_dir)?;
    println!("{shell} completion generated at: {}", path.display());
    Ok(())
}
