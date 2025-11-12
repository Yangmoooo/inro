mod remote;

#[derive(Debug)]
pub struct PackgeReceipt {
    pub name: String,
    pub version: String,
    pub bins: Vec<String>,
}


#[derive(thiserror::Error, Debug)]
pub enum PackageError {
    #[error("Package '{name}' not found in any sources")]
    NotFound { name: String },

    #[error("Failed to fetch from '{name}'")]
    Remote {
        name: String,
        #[source]
        source: remote::Error,
    },

    #[error("Download failed for '{url}'")]
    Download {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Checksum validation failed for downloaded file")]
    ChecksumMismatch,

    #[error("Failed to extract archive '{filename}'")]
    Extraction {
        filename: String,
        #[source]
        source: std::io::Error, // TODO replace from archive library
    },

    #[error("Could not find the binary '{binary_name}' inside the extracted archive")]
    BinaryNotFoundInArchive { binary_name: String },

    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),

}
