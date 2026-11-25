use std::env::var;

use anyhow::{Context, Result as AnyResult, bail};
use fda_mumbai_tracker::config::SecretString;
use fda_mumbai_tracker::db;
use fda_mumbai_tracker::init;
use tracing::info;

#[tokio::main]
async fn main() -> AnyResult<()> {
    init();
    let raw = var("DATABASE_URL").context("DATABASE_URL is not set")?;
    if raw.trim().is_empty() {
        bail!("DATABASE_URL must not be empty");
    }
    let url = SecretString::new(raw);
    let pool = db::pool(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("migrations applied");
    Ok(())
}
