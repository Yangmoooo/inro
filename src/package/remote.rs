pub mod direct;
pub mod github;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("An error occurred while fetching from GitHub")]
    GitHub(#[from] github::Error),

    #[error("An error occurred while processing a direct URL source")]
    Direct(#[from] direct::Error),

    #[error("The source type '{0}' is not supported")]
    UnsupportedSourceType(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct InstallCandidate {
    pub version: String,    // e.g., "v1.2.3"
    pub asset_name: String, // e.g., "ripgrep-v1.2.3-linux-musl.tar.gz"
    pub download_url: String,
}
