use std::path::PathBuf;

use anyhow::Result as AnyResult;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use tracing::instrument;

use crate::artifacts;
use crate::config::Config;
use crate::context::AppContext;
use crate::db;
use crate::llm;
use crate::news;
use crate::report::{RunReport, StoreOutcome};

const SEEN_DAYS: i64 = 21;

pub async fn run_scrape(config: &Config, ctx: &AppContext) -> AnyResult<RunReport> {
    let run_id = db::begin_run(&ctx.db).await?;

    match scrape_once(ctx, config).await {
        Ok(report) => {
            db::finish_run(&ctx.db, run_id, &report, &config.llm.model).await?;
            Ok(report)
        }
        Err(error) => {
            db::fail_run(&ctx.db, run_id, &format!("{error:#}")).await?;
            Err(error)
        }
    }
}

fn dump_artifact<T: serde::Serialize>(
    dir: Option<&PathBuf>,
    name: &str,
    value: &T,
) -> AnyResult<()> {
    let Some(dir) = dir else {
        return Ok(());
    };
    artifacts::write_json(dir, name, value)
}

#[instrument(skip_all, fields(seen_days = SEEN_DAYS, max_items = news::MAX_ITEMS))]
async fn scrape_once(ctx: &AppContext, config: &Config) -> AnyResult<RunReport> {
    let seen = Utc::now() - ChronoDuration::days(SEEN_DAYS);
    let artifact_dir = std::env::var_os("ARTIFACT_DIR").map(PathBuf::from);

    let items = news::fetch_items(&ctx.http).await?;
    dump_artifact(artifact_dir.as_ref(), "items.json", &items)?;
    let already_seen = db::seen_urls(&ctx.db, seen).await?;
    let articles_seen = items.len();
    let articles = news::enrich(&ctx.http, items, &already_seen, news::MAX_ITEMS).await;
    dump_artifact(artifact_dir.as_ref(), "articles.json", &articles)?;
    let articles_new = articles.len();

    let extraction = llm::extract(config, &articles).await?;
    let StoreOutcome { inserted, skipped } =
        db::upsert_actions(&ctx.db, &extraction.candidates).await?;
    let run = json!({
        "key": Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string(),
        "articles_seen": articles_seen,
        "articles_new": articles_new,
        "actions_upserted": inserted,
        "actions_skipped": skipped,
        "triage_calls": extraction.metrics.triage_calls,
        "extraction_calls": extraction.metrics.extraction_calls,
        "triage_failures": extraction.metrics.triage_failures,
        "extraction_failures": extraction.metrics.extraction_failures,
        "rejected": extraction.metrics.rejected,
        "degraded": extraction.degraded,
        "model": config.llm.model,
        "candidates": &extraction.candidates,
    });
    dump_artifact(artifact_dir.as_ref(), "run.json", &run)?;

    Ok(RunReport {
        articles_seen,
        articles_new,
        actions_upserted: inserted,
        metrics: extraction.metrics,
        degraded: extraction.degraded,
    })
}

#[cfg(test)]
#[path = "scrape_test.rs"]
mod tests;
