use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use reqwest::Client;

use crate::config::{Config, Persistence};
use crate::db;
use crate::local::LocalWriter;
use crate::news;
use crate::store::Store;

/// Dependencies built once at startup and passed explicitly through the pipeline.
pub struct AppContext {
    pub http: Client,
    pub store: Box<dyn Store>,
}

impl AppContext {
    pub async fn from_config(config: &Config) -> AnyResult<Self> {
        let http = Client::builder()
            .user_agent(news::USER_AGENT)
            .timeout(Duration::from_secs(60))
            .build()
            .context("build http client")?;
        let store: Box<dyn Store> = match &config.persistence {
            Persistence::Postgres { .. } => Box::new(db::pool(config).await?),
            Persistence::Local { path } => Box::new(LocalWriter::new(path.clone())),
        };
        Ok(Self { http, store })
    }
}
