pub mod config;
pub mod context;
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

use std::io::stderr;

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
        .with_writer(stderr)
        .with_target(false)
        .try_init();
}
