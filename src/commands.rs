pub mod clean;
pub mod generate;
pub mod info;
pub mod install;
pub mod list;
pub mod search;
pub mod source;
pub mod uninstall;
pub mod unlink;
pub mod update;
pub mod usecmd;

pub use clean::CleanCommand;
pub use generate::GenerateCommand;
pub use info::InfoCommand;
pub use install::InstallCommand;
pub use list::ListCommand;
pub use search::SearchCommand;
pub use source::SourceCommand;
pub use uninstall::UninstallCommand;
pub use unlink::UnlinkCommand;
pub use update::UpdateCommand;
pub use usecmd::UseCommand;

pub trait CommandHandler {
    fn handle(&self) -> anyhow::Result<()>;
}
