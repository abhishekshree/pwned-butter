use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

static POOL: OnceLock<PgPool> = OnceLock::new();

pub async fn pool() -> Result<&'static PgPool> {
    if let Some(pool) = POOL.get() {
        return Ok(pool);
    }
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&database_url)
        .await
        .context("connect to neon")?;
    Ok(POOL.get_or_init(|| pool))
}

pub async fn seen_urls(pool: &PgPool, cutoff: DateTime<Utc>) -> Result<HashSet<String>> {
    let rows = sqlx::query(
        "SELECT source_url FROM actions WHERE published_at > $1 OR published_at IS NULL",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| r.try_get::<String, _>("source_url").ok())
        .collect())
}

#[derive(Debug, Clone)]
pub struct ActionInsert {
    pub establishment: String,
    pub area: Option<String>,
    pub city: Option<String>,
    pub brand: Option<String>,
    pub operator: Option<String>,
    pub outlet_type: Option<String>,
    pub action_type: String,
    pub action_date: NaiveDate,
    pub violations: Vec<String>,
    pub compliance_score: Option<i32>,
    pub fssai_number: Option<String>,
    pub details: Option<String>,
    pub platforms: Vec<String>,
    pub source_url: String,
    pub source_publisher: Option<String>,
    pub source_headline: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

pub async fn upsert_actions(pool: &PgPool, rows: &[ActionInsert]) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let mut affected: usize = 0;
    for r in rows {
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM actions
             WHERE lower(establishment) = lower($1) AND action_date = $2
               AND ($3::text IS NULL OR lower(coalesce(city, '')) = lower(coalesce($3, '')))
             ORDER BY id LIMIT 1",
        )
        .bind(&r.establishment)
        .bind(r.action_date)
        .bind(&r.city)
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_some() {
            continue;
        }
        let res = sqlx::query(
            "INSERT INTO actions (
                establishment, area, city, brand, operator, outlet_type, action_type,
                action_date, violations, compliance_score, fssai_number, details,
                platforms, source_url, source_publisher, source_headline, published_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
            ON CONFLICT (source_url, establishment, action_date) DO UPDATE SET
                violations = EXCLUDED.violations,
                platforms = EXCLUDED.platforms,
                details = EXCLUDED.details,
                updated_at = now()",
        )
        .bind(&r.establishment)
        .bind(&r.area)
        .bind(&r.city)
        .bind(&r.brand)
        .bind(&r.operator)
        .bind(&r.outlet_type)
        .bind(&r.action_type)
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
        .await?;
        affected += usize::try_from(res.rows_affected())?;
    }
    tx.commit().await?;
    Ok(affected)
}

/// Insert rows, skipping any that already exist on
/// (source_url, establishment, action_date). Zero overwrite.
pub async fn insert_actions(pool: &PgPool, rows: &[ActionInsert]) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let mut inserted: usize = 0;
    for r in rows {
        let res = sqlx::query(
            "INSERT INTO actions (
                establishment, area, city, brand, operator, outlet_type, action_type,
                action_date, violations, compliance_score, fssai_number, details,
                platforms, source_url, source_publisher, source_headline, published_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
            ON CONFLICT (source_url, establishment, action_date) DO NOTHING",
        )
        .bind(&r.establishment)
        .bind(&r.area)
        .bind(&r.city)
        .bind(&r.brand)
        .bind(&r.operator)
        .bind(&r.outlet_type)
        .bind(&r.action_type)
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
        .await?;
        inserted += usize::try_from(res.rows_affected())?;
    }
    tx.commit().await?;
    Ok(inserted)
}

pub async fn begin_run(pool: &PgPool) -> Result<i64> {
    let row = sqlx::query("INSERT INTO fetch_runs (status) VALUES ('running') RETURNING id")
        .fetch_one(pool)
        .await?;
    row.try_get("id").map_err(Into::into)
}

pub async fn finish_run(
    pool: &PgPool,
    run_id: i64,
    articles_seen: usize,
    articles_new: usize,
    actions_upserted: usize,
    llm_calls: usize,
    details: &Value,
) -> Result<()> {
    sqlx::query(
        "UPDATE fetch_runs SET finished_at = now(), status = 'ok',
         articles_seen = $1, articles_new = $2, actions_upserted = $3, llm_calls = $4, details = $5
         WHERE id = $6",
    )
    .bind(articles_seen as i64)
    .bind(articles_new as i64)
    .bind(actions_upserted as i64)
    .bind(llm_calls as i64)
    .bind(details)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fail_run(pool: &PgPool, run_id: i64, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE fetch_runs SET finished_at = now(), status = 'error', error = $1 WHERE id = $2",
    )
    .bind(error)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_stale_runs(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "UPDATE fetch_runs SET finished_at = now(), status = 'error',
         error = 'stale run marked by backfill (no heartbeat)'
         WHERE status = 'running' AND started_at < now() - interval '30 minutes'",
    )
    .execute(pool)
    .await?;
    Ok(())
}
