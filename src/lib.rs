pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod llm;
mod local;
pub mod models;
pub mod news;
pub mod report;
pub mod rules;
pub mod scrape;
mod store;

use std::time::Duration;

use reqwest::Client;

/// The single startup boundary for binaries.
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

pub(crate) fn http_client() -> Client {
    Client::builder()
        .user_agent(news::USER_AGENT)
        .timeout(Duration::from_secs(60))
        .build()
        .expect("build shared reqwest client")
}
