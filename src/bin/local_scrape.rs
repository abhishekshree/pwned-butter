use fda_mumbai_tracker::config::Config;
use fda_mumbai_tracker::scrape::run_scrape;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    fda_mumbai_tracker::init_logging();
    let config = Config::from_env()?;

    let report = run_scrape(&config).await?;
    println!(
        "run ok: seen={} new={} upserted={} llm_calls={} triage_failures={} extraction_failures={} rejected={}",
        report.articles_seen,
        report.articles_new,
        report.actions_upserted,
        report.metrics.total_calls(),
        report.metrics.triage_failures,
        report.metrics.extraction_failures,
        report.metrics.rejected,
    );
    Ok(())
}
