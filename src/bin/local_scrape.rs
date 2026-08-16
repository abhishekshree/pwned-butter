use fda_mumbai_tracker::scrape::run_scrape;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let gemini_key = std::env::var("GEMINI_API_KEY")?;
    let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-latest".into());

    let (run_id, report) = run_scrape(&gemini_key, &model).await?;
    println!(
        "run {run_id} ok: seen={} new={} upserted={} llm_calls={}",
        report.articles_seen, report.articles_new, report.actions_upserted, report.llm_calls
    );
    Ok(())
}
