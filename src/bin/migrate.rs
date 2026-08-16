use fda_mumbai_tracker::db;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let pool = db::pool().await?;
    sqlx::migrate!("./migrations").run(pool).await?;
    println!("migrations applied");
    Ok(())
}
