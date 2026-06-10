pub mod clean;
pub mod doctor;
pub mod env;
pub mod generate;
pub mod install;
pub mod list;
pub mod pin;
pub mod search;
pub mod show;
pub mod source;
pub mod uninstall;
pub mod unlink;
pub mod update;
pub mod usecmd;

pub use clean::CleanCommand;
pub use doctor::DoctorCommand;
pub use env::EnvCommand;
pub use generate::GenerateCommand;
pub use install::InstallCommand;
pub use list::ListCommand;
pub use pin::{PinCommand, UnpinCommand};
pub use search::SearchCommand;
pub use show::ShowCommand;
pub use source::SourceCommand;
pub use uninstall::UninstallCommand;
pub use unlink::UnlinkCommand;
pub use update::UpdateCommand;
pub use usecmd::UseCommand;

pub trait CommandHandler {
    fn handle(&self) -> anyhow::Result<()>;
}
