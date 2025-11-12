#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("The provided URL template is invalid: {0}")]
    InvalidUrlTemplate(String),

    #[error("Failed to render URL template: {0}")]
    TemplateRender(String), // Placeholder for a real template engine error
}

pub type Result<T> = std::result::Result<T, Error>;
