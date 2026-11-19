use anyhow::Result as AnyResult;
use fda_mumbai_tracker::config::Config;
use fda_mumbai_tracker::context::AppContext;
use fda_mumbai_tracker::init;
use fda_mumbai_tracker::scrape::run_scrape;
use tracing::info;

#[tokio::main]
async fn main() -> AnyResult<()> {
    init();
    let config = Config::from_env()?;

    let ctx = AppContext::from_config(&config).await?;
    let report = run_scrape(&config, &ctx).await?;
    info!(
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
