use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::config::Config;
use crate::domain::{Candidate, EventKey};
use crate::report::{RunReport, StoreOutcome};

pub async fn pool(config: &Config) -> AnyResult<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(15))
        .connect(config.database_url.expose())
        .await
        .context("connect to neon")
}

pub(crate) async fn seen_urls(pool: &PgPool, cutoff: DateTime<Utc>) -> AnyResult<HashSet<String>> {
    // Seen-window is when we first stored the URL, not when the article was published.
    Ok(
        sqlx::query_scalar::<_, String>("SELECT source_url FROM actions WHERE created_at > $1")
            .bind(cutoff)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect(),
    )
}

const UPSERT_SQL: &str = "INSERT INTO actions (
            establishment, area, city, brand, operator, outlet_type, action_type,
            action_date, violations, compliance_score, fssai_number, details,
            platforms, source_url, source_publisher, source_headline, published_at
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
         ON CONFLICT (source_url, establishment, action_date) DO UPDATE SET
            violations = EXCLUDED.violations,
            platforms = EXCLUDED.platforms,
            details = EXCLUDED.details,
            updated_at = now()";

/// `EventKey` is the dedup authority; the SQL EXCLUDE constraint is the backstop.
pub(crate) async fn upsert_actions(pool: &PgPool, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
    let mut tx = pool.begin().await?;
    let mut outcome = StoreOutcome::default();
    for r in rows {
        // One event per outlet AND action type, so a reopening is not swallowed.
        // ±10 pre-check vs SQL's ±5 daterange: a narrower check misses conflicts and aborts the tx.
        let key = EventKey::from_candidate(r);
        let candidates: Vec<String> = sqlx::query(
            "SELECT establishment FROM actions
                 WHERE action_date BETWEEN $1 - 10 AND $1 + 10
                   AND action_type = $2",
        )
        .bind(r.action_date)
        .bind(r.action_type.to_string())
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get(0).ok())
        .collect();
        if candidates
            .iter()
            .any(|name| key.matches(&EventKey::from_name(name)))
        {
            outcome.skipped += 1;
            continue;
        }
        outcome.inserted += usize::try_from(
            sqlx::query(UPSERT_SQL)
                .bind(&r.establishment)
                .bind(&r.area)
                .bind(r.city.as_str())
                .bind(&r.brand)
                .bind(&r.operator)
                .bind(r.outlet_type.map(|t| t.to_string()))
                .bind(r.action_type.to_string())
                .bind(r.action_date)
                .bind(&r.violations)
                .bind(r.compliance_score)
                .bind(&r.fssai_number)
                .bind(&r.details)
                .bind(&r.platforms)
                .bind(&r.source_url)
                .bind(&r.source_publisher)
                .bind(&r.source_headline)
                .bind(r.published_at)
                .execute(&mut *tx)
                .await?
                .rows_affected(),
        )?;
    }
    tx.commit().await?;
    Ok(outcome)
}

pub(crate) async fn begin_run(pool: &PgPool) -> AnyResult<i64> {
    let row = sqlx::query("INSERT INTO fetch_runs (status) VALUES ('running') RETURNING id")
        .fetch_one(pool)
        .await?;
    row.try_get("id").map_err(Into::into)
}

pub(crate) async fn finish_run(
    pool: &PgPool,
    run_id: i64,
    report: &RunReport,
    model: &str,
) -> AnyResult<()> {
    let mut details_json = report.to_details();
    details_json["model"] = json!(model);

    sqlx::query(
        "UPDATE fetch_runs SET finished_at = now(), status = 'ok',
         articles_seen = $1, articles_new = $2, actions_upserted = $3, llm_calls = $4, details = $5
         WHERE id = $6",
    )
    .bind(i64::try_from(report.articles_seen)?)
    .bind(i64::try_from(report.articles_new)?)
    .bind(i64::try_from(report.actions_upserted)?)
    .bind(i64::try_from(report.metrics.total_calls())?)
    .bind(&details_json)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn fail_run(pool: &PgPool, run_id: i64, error: &str) -> AnyResult<()> {
    sqlx::query(
        "UPDATE fetch_runs SET finished_at = now(), status = 'error', error = $1 WHERE id = $2",
    )
    .bind(error)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}
