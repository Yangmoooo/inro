use anyhow::{Result, bail};

use super::CommandHandler;
use crate::package::{PackageError, PackgeReceipt};
use crate::{report, reporter::MsgType};

pub struct InstallCommand {
    pub names: Vec<String>,
}

impl CommandHandler for InstallCommand {
    fn handle(&self) -> Result<()> {
        let names = super::unique(&self.names);
        report!(
            MsgType::Info,
            "Starting installation of {} package(s)...",
            names.len()
        );

        let mut success_receipts = Vec::new();
        let mut failed_packages = Vec::new();

        for name in &names {
            match do_install(name) {
                Ok(receipt) => success_receipts.push(receipt),
                Err(e) => {
                    report!(MsgType::Error, "Failed to install '{name}': {e:?}");
                    failed_packages.push(name.clone());
                }
            }
        }

        if !success_receipts.is_empty() {
            let success_packages: Vec<_> = success_receipts.into_iter().map(|r| r.name).collect();
            report!(
                MsgType::Info,
                "Installed {} package(s): {success_packages:?}",
                success_packages.len()
            )
        }

        if !failed_packages.is_empty() {
            report!(
                MsgType::Error,
                "\nDone with errors. Failed to install: {failed_packages:?}"
            );
            bail!("One or more packages failed to install.");
        } else {
            report!(MsgType::Success, "\nAll packages installed successfully.")
        }

        Ok(())
    }
}

fn do_install(name: &str) -> Result<PackgeReceipt, PackageError> {
    report!(MsgType::Step, "Processing package '{name}'...");
    report!(MsgType::Detail, "Fetching from GitHub Releases...");
    Ok(PackgeReceipt {
        name: name.to_string(),
        version: "v0.1.0".to_string(),
        bins: vec![name.to_string()],
    })
}
