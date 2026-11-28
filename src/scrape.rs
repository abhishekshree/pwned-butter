use std::path::PathBuf;

use anyhow::Result as AnyResult;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use tracing::instrument;

use crate::artifacts;
use crate::config::Config;
use crate::context::AppContext;
use crate::llm;
use crate::news;
use crate::report::{RunReport, StoreOutcome};

pub async fn run_scrape(config: &Config, ctx: &AppContext) -> AnyResult<RunReport> {
    run(config, ctx, 21, news::MAX_ITEMS).await
}

async fn run(
    config: &Config,
    ctx: &AppContext,
    seen_days: i64,
    max_items: usize,
) -> AnyResult<RunReport> {
    let store = ctx.store.as_ref();
    let run_id = store.begin_run().await?;

    match scrape_once(ctx, config, seen_days, max_items).await {
        Ok(report) => {
            store
                .finish_run(run_id, &report, &json!({"model": config.llm.model}))
                .await?;
            Ok(report)
        }
        Err(error) => {
            store.fail_run(run_id, &format!("{error:#}")).await?;
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

#[instrument(skip_all, fields(seen_days = seen_days, max_items = max_items))]
async fn scrape_once(
    ctx: &AppContext,
    config: &Config,
    seen_days: i64,
    max_items: usize,
) -> AnyResult<RunReport> {
    let store = ctx.store.as_ref();
    let seen = Utc::now() - ChronoDuration::days(seen_days);
    let artifact_dir = std::env::var_os("ARTIFACT_DIR").map(PathBuf::from);

    let items = news::fetch_items(&ctx.http).await?;
    dump_artifact(artifact_dir.as_ref(), "items.json", &items)?;
    let already_seen = store.seen_urls(seen).await?;
    let articles_seen = items.len();
    let articles = news::enrich(&ctx.http, items, &already_seen, max_items).await;
    dump_artifact(artifact_dir.as_ref(), "articles.json", &articles)?;
    let articles_new = articles.len();

    let extraction = llm::extract(config, &articles).await?;
    let StoreOutcome { inserted, skipped } = store.upsert_actions(&extraction.candidates).await?;
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
#[path = "../tests/unit/scrape.rs"]
mod tests;
