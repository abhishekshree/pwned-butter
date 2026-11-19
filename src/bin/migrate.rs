use anyhow::Result as AnyResult;
use fda_mumbai_tracker::config::Config;
use fda_mumbai_tracker::db;
use fda_mumbai_tracker::init;
use tracing::info;

#[tokio::main]
async fn main() -> AnyResult<()> {
    init();
    let config = Config::from_env()?;
    let pool = db::pool(&config).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("migrations applied");
    Ok(())
}
