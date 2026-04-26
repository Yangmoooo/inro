use std::env;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;

static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

/// Bound how long establishing a connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound how long a single read may stall, so a hung server cannot wedge
/// the command. The total request length is intentionally unbounded so
/// large package downloads on slow links are not killed.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Get or initialize the shared async HTTP client.
pub fn get() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(format!("inro/{}", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .expect("Failed to build HTTP client")
    })
}
