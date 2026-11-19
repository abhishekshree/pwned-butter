use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use crate::config::Config;
use crate::domain::Candidate;
use crate::report::{RunReport, StoreOutcome};

static POOL: OnceLock<PgPool> = OnceLock::new();
static POOL_LOCK: Mutex<()> = Mutex::const_new(());

pub async fn pool(config: &Config) -> AnyResult<&'static PgPool> {
    if let Some(pool) = POOL.get() {
        return Ok(pool);
    }
    // Serialize the first connect so concurrent callers cannot each build one.
    let _guard = POOL_LOCK.lock().await;
    if let Some(pool) = POOL.get() {
        return Ok(pool);
    }
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&config.database_url)
        .await
        .context("connect to neon")?;
    Ok(POOL.get_or_init(|| pool))
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

const INSERT_TAIL: &str = "establishment, area, city, brand, operator, outlet_type, action_type,
                action_date, violations, compliance_score, fssai_number, details,
                platforms, source_url, source_publisher, source_headline, published_at";

async fn execute_insert(
    conn: &mut sqlx::PgConnection,
    r: &Candidate,
    conflict_tail: &str,
) -> AnyResult<u64> {
    let sql = format!(
        "INSERT INTO actions ({INSERT_TAIL}) VALUES \
         ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) \
         ON CONFLICT (source_url, establishment, action_date) {conflict_tail}"
    );
    // conflict_tail is a compile-time constant, not user input.
    let res = sqlx::query(sqlx::AssertSqlSafe(sql))
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
        .execute(conn)
        .await?;
    Ok(res.rows_affected())
}

/// `same_event` is the Rust dedup authority; the SQL EXCLUDE constraint is the backstop.
pub(crate) async fn upsert_actions(pool: &PgPool, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
    let mut tx = pool.begin().await?;
    let mut outcome = StoreOutcome::default();
    for r in rows {
        // One event per outlet AND action type, so a reopening is not swallowed.
        // ±10 pre-check vs SQL's ±5 daterange: a narrower check misses conflicts and aborts the tx.
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
            .any(|name| same_event(name, &r.establishment))
        {
            outcome.skipped += 1;
            continue;
        }
        outcome.inserted += usize::try_from(
            execute_insert(
                &mut tx,
                r,
                "DO UPDATE SET
                    violations = EXCLUDED.violations,
                    platforms = EXCLUDED.platforms,
                    details = EXCLUDED.details,
                    updated_at = now()",
            )
            .await?,
        )?;
    }
    tx.commit().await?;
    Ok(outcome)
}

/// Venue-type words that vary between reports of the same outlet.
const GENERIC_WORDS: [&str; 7] = [
    "club",
    "facility",
    "canteen",
    "restaurant",
    "hotel",
    "outlet",
    "kitchen",
];

fn tokens(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && !GENERIC_WORDS.contains(t))
        .collect()
}

/// "mca" = initials of the consecutive run "mumbai cricket association".
fn is_acronym_of(acro: &str, words: &[&str]) -> bool {
    !acro.is_empty()
        && acro.len() <= words.len()
        && acro.chars().zip(words).all(|(c, w)| w.starts_with(c))
}

fn token_matches(t: &str, words: &[&str]) -> bool {
    words.contains(&t)
        || (0..=words.len().saturating_sub(t.len())).any(|i| is_acronym_of(t, &words[i..]))
}

/// Exact, containment for 6+ char names, or token/acronym matching with a 3+ char word in common.
pub(crate) fn same_event(a: &str, b: &str) -> bool {
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
    // Short-name containment is too loose ("KFC" everywhere).
    if a.len().min(b.len()) >= 6 && (a.contains(&b) || b.contains(&a)) {
        return true;
    }
    let side_matches = |from: &[&str], to: &[&str]| {
        from.iter().map(|t| t.len()).sum::<usize>() >= 6
            && from.iter().all(|t| token_matches(t, to))
            && from.iter().any(|t| t.len() >= 3 && to.contains(t))
    };
    let (ta, tb) = (tokens(&a), tokens(&b));
    side_matches(&ta, &tb) || side_matches(&tb, &ta)
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
    details: &Value,
) -> AnyResult<()> {
    let mut details_json = json!({
        "triage_calls": report.metrics.triage_calls,
        "extraction_calls": report.metrics.extraction_calls,
        "triage_failures": report.metrics.triage_failures,
        "extraction_failures": report.metrics.extraction_failures,
        "rejected": report.metrics.rejected,
    });
    // Typed metrics win on collision, so only add caller keys we do not already have.
    if let (Some(obj), Some(extra)) = (details_json.as_object_mut(), details.as_object()) {
        for (key, value) in extra {
            obj.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

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

#[cfg(test)]
mod tests {
    use super::same_event;

    #[test]
    fn same_event_matches_case_and_containment() {
        assert!(
            same_event("Otters Club", "otters club"),
            "case-insensitive exact"
        );
        assert!(
            same_event("Otters Club", "Otters Club, Carter Road, Bandra West"),
            "locality qualifier still matches"
        );
        assert!(
            same_event(
                "Blinkit Dark Store (Malad West)",
                "Blinkit Dark Store Malad West"
            ),
            "punctuation-insensitive match"
        );
    }

    #[test]
    fn same_event_rejects_different_outlets() {
        assert!(
            !same_event(
                "Domino's Pizza (Borivali West)",
                "Domino's Pizza (Ghatkopar West)"
            ),
            "different localities are different events"
        );
        assert!(
            !same_event("Domino's Pizza", "Pizza Hut Borivali"),
            "different brands never match"
        );
    }

    #[test]
    fn same_event_short_names_match_exactly_only() {
        assert!(
            !same_event("KFC", "KFC Andheri"),
            "short name containment is too loose"
        );
        assert!(same_event("KFC", "kfc"), "exact short names match");
    }

    #[test]
    fn same_event_matches_acronym_and_venue_drift() {
        assert!(
            same_event("MCA BKC Club", "Mumbai Cricket Association (BKC Facility)"),
            "acronym plus venue-word drift matches (Aug 2026 dupes)"
        );
        assert!(
            !same_event(
                "Cricket Club of India Canteen",
                "Mumbai Cricket Association (BKC Facility)"
            ),
            "shared generic words alone do not match"
        );
    }
}
