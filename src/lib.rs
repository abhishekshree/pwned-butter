pub mod artifacts;
pub mod config;
pub mod context;
pub mod db;
pub mod domain;
pub mod error;
pub mod ingest;
pub mod llm;
mod local;
pub mod models;
pub mod report;
mod store;

pub use ingest::{news, rules, scrape};

/// The single startup boundary for binaries.
pub fn init() {
    dotenvy::dotenv().ok();
    init_logging();
}

fn init_logging() {
    use std::fs::OpenOptions;
    use std::io::stderr;

    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::prelude::*;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr_layer = fmt::layer()
        .with_writer(stderr)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .with_filter(filter);
    let registry = tracing_subscriber::registry().with(stderr_layer);

    let Some(dir) = std::env::var_os("ARTIFACT_DIR") else {
        let _ = tracing::subscriber::set_global_default(registry);
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot create artifact dir {}: {error}", dir.display());
        let _ = tracing::subscriber::set_global_default(registry);
        return;
    }
    let path = dir.join("llm.jsonl");
    match OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
    {
        Ok(file) => {
            let json_layer = fmt::layer()
                .json()
                .with_writer(file)
                .with_filter(EnvFilter::new("artifact=debug"));
            let _ = tracing::subscriber::set_global_default(registry.with(json_layer));
        }
        Err(error) => {
            eprintln!("cannot open artifact log {}: {error}", path.display());
            let _ = tracing::subscriber::set_global_default(registry);
        }
    }
}
