pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod llm;
pub mod models;
pub mod news;
pub mod report;
pub mod rules;
pub mod scrape;

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;

static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

/// The single startup boundary for binaries: load `.env` into the process
/// environment, then configure logging. `std::env::var` is the only reader,
/// used by `Config::from_env` and by the cache-bust tool's own variables.
pub fn init() {
    dotenvy::dotenv().ok();
    init_logging();
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

pub fn http_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(news::USER_AGENT)
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build shared reqwest client")
    })
}
