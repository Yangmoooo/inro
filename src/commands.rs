pub mod info;
pub mod install;
pub mod list;
pub mod search;
pub mod source;
pub mod uninstall;
pub mod usecmd;

use anyhow::Result;

pub use info::InfoCommand;
pub use install::InstallCommand;
pub use list::ListCommand;
pub use search::SearchCommand;
pub use source::SourceCommand;
pub use uninstall::UninstallCommand;
pub use usecmd::UseCommand;

pub trait CommandHandler {
    fn handle(&self) -> Result<()>;
}
