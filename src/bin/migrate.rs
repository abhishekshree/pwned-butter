use fda_mumbai_tracker::config::Config;
use fda_mumbai_tracker::db;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    fda_mumbai_tracker::init_logging();
    let config = Config::from_env()?;
    let pool = db::pool(&config).await?;
    sqlx::migrate!("./migrations").run(pool).await?;
    println!("migrations applied");
    Ok(())
}
