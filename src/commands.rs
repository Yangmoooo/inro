pub mod install;
pub mod list;
pub mod source;
pub mod uninstall;

use anyhow::Result;

pub use install::InstallCommand;
pub use list::ListCommand;
pub use source::SourceCommand;
pub use uninstall::UninstallCommand;

pub trait CommandHandler {
    fn handle(&self) -> Result<()>;
}

pub fn unique(strs: &[String]) -> Vec<String> {
    let mut vec = strs.to_owned();
    vec.sort_unstable();
    vec.dedup();
    vec
}
