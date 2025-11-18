pub mod github;

use std::path::{Path, PathBuf};

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("An error occurred while fetching from GitHub")]
    GitHub(#[from] github::Error),

    #[error("The source type '{0}' is not supported")]
    UnsupportedSourceType(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[async_trait]
pub trait RemoteProvider {
    async fn download_asset(&self, dest_dir: &Path) -> Result<PathBuf>;
}
