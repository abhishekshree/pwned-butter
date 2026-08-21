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

const INSERT_TAIL: &str = "establishment, area, city, brand, operator, outlet_type, action_type,
                action_date, violations, compliance_score, fssai_number, details,
                platforms, source_url, source_publisher, source_headline, published_at";

async fn execute_insert(
    conn: &mut sqlx::PgConnection,
    r: &ActionInsert,
    conflict_tail: &str,
) -> Result<u64> {
    let sql = format!(
        "INSERT INTO actions ({INSERT_TAIL}) VALUES \
         ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) \
         ON CONFLICT (source_url, establishment, action_date) {conflict_tail}"
    );
    let res = sqlx::query(&sql)
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
        .execute(conn)
        .await?;
    Ok(res.rows_affected())
}

pub async fn upsert_actions(pool: &PgPool, rows: &[ActionInsert]) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let mut affected: usize = 0;
    for r in rows {
        // Same outlet + same action within ±5 days = one event, re-reported by
        // another outlet. SQL narrows to the window; name matching tolerates
        // qualifier variants ("Otters Club" vs "Otters Club, Carter Road").
        // ponytail: containment match, so "KFC" vs "KFC Andheri" still slips
        // through; switch to pg_trgm similarity if variants show up in practice.
        let candidates: Vec<String> = sqlx::query(
            "SELECT establishment FROM actions
             WHERE action_type = $1
               AND action_date BETWEEN $2 - 5 AND $2 + 5",
        )
        .bind(&r.action_type)
        .bind(r.action_date)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get(0).ok())
        .collect();
        if candidates
            .iter()
            .any(|name| same_event(name, &r.establishment))
        {
            continue;
        }
        let affected_rows = execute_insert(
            &mut tx,
            r,
            "DO UPDATE SET
                violations = EXCLUDED.violations,
                platforms = EXCLUDED.platforms,
                details = EXCLUDED.details,
                updated_at = now()",
        )
        .await?;
        affected += usize::try_from(affected_rows)?;
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
        let inserted_rows = execute_insert(&mut tx, r, "DO NOTHING").await?;
        inserted += usize::try_from(inserted_rows)?;
    }
    tx.commit().await?;
    Ok(inserted)
}

/// Replace a date window of rows with a fresh, deduped extraction. Rows are
/// deleted by published_at/action_date (Google News URLs rotate between
/// fetches, so source_url is not a stable key), then inserted with
/// within-batch dedup on (action_date, city, establishment-name overlap).
/// Returns (deleted, inserted).
pub async fn replace_actions(
    pool: &PgPool,
    since: DateTime<Utc>,
    rows: &[ActionInsert],
) -> Result<(usize, usize)> {
    let mut tx = pool.begin().await?;
    let deleted =
        sqlx::query("DELETE FROM actions WHERE published_at >= $1 OR action_date >= $1::date")
            .bind(since)
            .execute(&mut *tx)
            .await?;

    let mut inserted = 0usize;
    let mut kept: Vec<&ActionInsert> = Vec::new();
    for r in rows {
        if kept.iter().any(|k| {
            k.action_date == r.action_date
                && k.city == r.city
                && name_overlap(&k.establishment, &r.establishment) > 0
        }) {
            continue;
        }
        inserted += usize::try_from(execute_insert(&mut tx, r, "DO NOTHING").await?)?;
        kept.push(r);
    }

    tx.commit().await?;
    Ok((usize::try_from(deleted.rows_affected())?, inserted))
}

fn name_overlap(a: &str, b: &str) -> usize {
    let tokens = |s: &str| -> HashSet<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    };
    tokens(a).intersection(&tokens(b)).count()
}

/// True when two establishment names refer to the same outlet: exact
/// (case/punctuation-insensitive) or one name containing the other, so a bare
/// name and its qualified variant collapse to one event.
fn same_event(a: &str, b: &str) -> bool {
    let norm = |s: &str| -> String {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (a, b) = (norm(a), norm(b));
    if a == b {
        return true;
    }
    // containment of a very short name is too loose ("KFC" everywhere)
    a.len().min(b.len()) >= 6 && (a.contains(&b) || b.contains(&a))
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

#[cfg(test)]
mod tests {
    use super::same_event;

    #[test]
    fn same_event_matches_variants_not_branches() {
        assert!(same_event("Otters Club", "otters club"));
        assert!(same_event(
            "Otters Club",
            "Otters Club, Carter Road, Bandra West"
        ));
        assert!(same_event(
            "Blinkit Dark Store (Malad West)",
            "Blinkit Dark Store Malad West"
        ));
        assert!(!same_event(
            "Domino's Pizza (Borivali West)",
            "Domino's Pizza (Ghatkopar West)"
        ));
        assert!(!same_event("Domino's Pizza", "Pizza Hut Borivali"));
        // short names only match exactly
        assert!(!same_event("KFC", "KFC Andheri"));
        assert!(same_event("KFC", "kfc"));
    }
}
