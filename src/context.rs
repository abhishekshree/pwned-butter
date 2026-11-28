use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use reqwest::Client;
use sqlx::PgPool;

use crate::config::Config;
use crate::db;
use crate::news;

/// Dependencies built once at startup and passed explicitly through the pipeline.
pub struct AppContext {
    pub http: Client,
    pub db: PgPool,
}

impl AppContext {
    pub async fn from_config(config: &Config) -> AnyResult<Self> {
        let http = Client::builder()
            .user_agent(news::USER_AGENT)
            .timeout(Duration::from_secs(60))
            .build()
            .context("build http client")?;
        let db = db::pool(config).await?;
        Ok(Self { http, db })
    }
}
