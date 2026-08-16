use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{FromRow, PgPool, QueryBuilder, Row};

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

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRow {
    pub id: i64,
    pub establishment: String,
    pub area: Option<String>,
    pub city: Option<String>,
    pub state: String,
    pub brand: Option<String>,
    pub operator: Option<String>,
    pub outlet_type: Option<String>,
    pub action_type: String,
    pub action_date: NaiveDate,
    pub status: String,
    pub violations: Vec<String>,
    pub compliance_score: Option<i32>,
    pub fssai_number: Option<String>,
    pub details: Option<String>,
    pub platforms: Vec<String>,
    pub source_url: String,
    pub source_publisher: Option<String>,
    pub source_headline: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(default)]
    #[serde(skip)]
    pub total_count: i64,
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

#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    pub brand: Option<String>,
    pub city: Option<String>,
    pub status: Option<String>,
    pub action_type: Option<String>,
    pub outlet_type: Option<String>,
    pub q: Option<String>,
    #[serde(default)]
    pub from: Option<NaiveDate>,
    #[serde(default)]
    pub to: Option<NaiveDate>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

pub async fn list_actions(pool: &PgPool, p: &ListParams) -> Result<(i64, Vec<ActionRow>)> {
    let (limit, offset) = (
        p.limit.unwrap_or(50).clamp(1, 200),
        p.offset.unwrap_or(0).max(0),
    );

    let mut qb: QueryBuilder<'_, sqlx::Postgres> = QueryBuilder::new(
        "SELECT actions.*, COUNT(*) OVER() AS total_count FROM actions WHERE true",
    );
    push_eq(&mut qb, "brand", &p.brand);
    push_eq(&mut qb, "city", &p.city);
    push_eq(&mut qb, "status", &p.status);
    push_eq(&mut qb, "action_type", &p.action_type);
    push_eq(&mut qb, "outlet_type", &p.outlet_type);

    if let Some(q) = p.q.trimmed() {
        let like = format!("%{q}%");
        qb.push(" AND (establishment ILIKE ")
            .push_bind(like.clone())
            .push(" OR brand ILIKE ")
            .push_bind(like.clone())
            .push(" OR area ILIKE ")
            .push_bind(like)
            .push(")");
    }
    if let Some(d) = p.from {
        qb.push(" AND action_date >= ").push_bind(d);
    }
    if let Some(d) = p.to {
        qb.push(" AND action_date <= ").push_bind(d);
    }

    qb.push(" ORDER BY action_date DESC, id DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let rows = qb.build_query_as::<ActionRow>().fetch_all(pool).await?;
    let total = rows.first().map_or(0, |r| r.total_count);
    Ok((total, rows))
}

trait Trimmed {
    fn trimmed(&self) -> Option<String>;
}

impl Trimmed for Option<String> {
    fn trimmed(&self) -> Option<String> {
        self.as_ref().map(|s| s.trim().to_string()).filter(|s| {
            !s.is_empty() && !s.eq_ignore_ascii_case("all") && !s.eq_ignore_ascii_case("any")
        })
    }
}

fn push_eq(qb: &mut QueryBuilder<'_, sqlx::Postgres>, col: &str, val: &Option<String>) {
    if let Some(v) = val.trimmed() {
        qb.push(" AND ").push(col).push(" = ").push_bind(v);
    }
}

#[derive(Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunSummary {
    id: i64,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    status: String,
    articles_seen: i64,
    articles_new: i64,
    actions_upserted: i64,
    llm_calls: i64,
    error: Option<String>,
}

pub async fn stats(pool: &PgPool) -> Result<Value> {
    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM actions")
        .fetch_one(pool)
        .await?;
    let latest: NaiveDate =
        sqlx::query_scalar("SELECT COALESCE(MAX(action_date), '1970-01-01'::date) FROM actions")
            .fetch_one(pool)
            .await?;

    let last_run = sqlx::query_as::<_, RunSummary>(
        "SELECT id, started_at, finished_at, status, articles_seen, articles_new,
         actions_upserted, llm_calls, error
         FROM fetch_runs ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    .map(|r| json!(r))
    .unwrap_or(Value::Null);

    Ok(json!({
        "totalActions": total,
        "latestActionDate": latest,
        "byStatus": dims_to_value(dimension_counts(pool, "status").await?),
        "byActionType": dims_to_value(dimension_counts(pool, "action_type").await?),
        "byCity": dims_to_value(dimension_counts(pool, "city").await?),
        "lastRun": last_run,
    }))
}

pub async fn dimension_counts(pool: &PgPool, col: &str) -> Result<Vec<(String, i64)>> {
    let q = format!(
        "SELECT COALESCE(NULLIF({col}, ''), 'unknown') AS k, COUNT(*)::bigint AS n
         FROM actions GROUP BY k ORDER BY n DESC"
    );
    let rows = sqlx::query(&q).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push((r.try_get::<String, _>("k")?, r.try_get::<i64, _>("n")?));
    }
    Ok(out)
}

fn dims_to_value(rows: Vec<(String, i64)>) -> Value {
    Value::Object(
        rows.into_iter()
            .map(|(k, n)| (k, json!(n)))
            .collect::<serde_json::Map<String, Value>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_filters_all() {
        assert!(None.trimmed().is_none());
        assert!(Some("  ".into()).trimmed().is_none());
        assert!(Some("all".into()).trimmed().is_none());
        assert!(Some("any".into()).trimmed().is_none());
        assert_eq!(Some("Mumbai".into()).trimmed(), Some("Mumbai".into()));
    }
}
