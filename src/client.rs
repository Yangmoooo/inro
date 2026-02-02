use std::env;
use std::sync::OnceLock;

use reqwest::Client;

static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

/// Get or initialize the shared async HTTP client
pub fn get() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(format!("inro/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("Failed to build HTTP client")
    })
}
