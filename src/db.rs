use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::{PgArguments, PgPoolOptions};
use sqlx::query::Query;
use sqlx::{PgPool, Postgres, Row};

use crate::config::SecretString;
use crate::domain::{Candidate, EventKey};
use crate::report::{RunReport, StoreOutcome};

pub async fn pool(url: &SecretString) -> AnyResult<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(15))
        .connect(url.expose())
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

const INSERT_TAIL: &str = "establishment, area, city, brand, operator, outlet_type, action_type,
                action_date, violations, compliance_score, fssai_number, details,
                platforms, source_url, source_publisher, source_headline, published_at";

const INSERT_CONFLICT_TAIL: &str = "DO UPDATE SET
                area = EXCLUDED.area,
                city = EXCLUDED.city,
                brand = EXCLUDED.brand,
                operator = EXCLUDED.operator,
                outlet_type = EXCLUDED.outlet_type,
                action_type = EXCLUDED.action_type,
                violations = EXCLUDED.violations,
                compliance_score = EXCLUDED.compliance_score,
                fssai_number = EXCLUDED.fssai_number,
                details = EXCLUDED.details,
                platforms = EXCLUDED.platforms,
                source_publisher = EXCLUDED.source_publisher,
                source_headline = EXCLUDED.source_headline,
                published_at = EXCLUDED.published_at,
                updated_at = now()";

const UPDATE_BY_ID: &str = "UPDATE actions SET
                establishment=$1, area=$2, city=$3, brand=$4, operator=$5,
                outlet_type=$6, action_type=$7, action_date=$8, violations=$9,
                compliance_score=$10, fssai_number=$11, details=$12, platforms=$13,
                source_url=$14, source_publisher=$15, source_headline=$16,
                published_at=$17, updated_at=now()
                WHERE id=$18";

type BoundCandidate<'q> = Query<'q, Postgres, PgArguments>;

fn bind_candidate<'q>(query: BoundCandidate<'q>, r: &'q Candidate) -> BoundCandidate<'q> {
    query
        .bind(r.establishment())
        .bind(r.area())
        .bind(Some(r.city().as_str()))
        .bind(r.brand())
        .bind(r.operator())
        .bind(r.outlet_type().map(|t| t.to_string()))
        .bind(r.action_type().to_string())
        .bind(r.action_date())
        .bind(r.violations().to_vec())
        .bind(r.compliance_score())
        .bind(r.fssai_number())
        .bind(r.details())
        .bind(r.platforms().to_vec())
        .bind(r.source_url())
        .bind(r.source_publisher())
        .bind(r.source_headline())
        .bind(r.published_at())
}

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
    let res = bind_candidate(sqlx::query(sqlx::AssertSqlSafe(sql)), r)
        .execute(conn)
        .await?;
    Ok(res.rows_affected())
}

async fn execute_update(conn: &mut sqlx::PgConnection, r: &Candidate, id: i64) -> AnyResult<()> {
    bind_candidate(sqlx::query(sqlx::AssertSqlSafe(UPDATE_BY_ID)), r)
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

struct StoredRow {
    id: i64,
    name: String,
    city: Option<String>,
    area: Option<String>,
    action_type: String,
}

/// Same-source rows are disambiguated by exact name, then action type, then
/// lowest id. All candidates for this pick share the exact action date, so a
/// different event 5 days apart can never be rewritten by a re-store.
fn pick_target(r: &Candidate, stored: &[StoredRow]) -> Option<i64> {
    let key = EventKey::from_row(r.establishment(), Some(r.city().as_str()), r.area());
    stored
        .iter()
        .filter(|row| {
            key.matches(&EventKey::from_row(
                &row.name,
                row.city.as_deref(),
                row.area.as_deref(),
            ))
        })
        .min_by_key(|row| {
            (
                u8::from(row.name != r.establishment()),
                u8::from(row.action_type != r.action_type().to_string()),
                row.id,
            )
        })
        .map(|row| row.id)
}

/// Check the two database constraints before writing, so a collision skips one
/// row instead of aborting the whole transaction.
async fn collision_exists(
    conn: &mut sqlx::PgConnection,
    r: &Candidate,
    exclude_id: Option<i64>,
) -> AnyResult<bool> {
    let type_name = r.action_type().to_string();
    let exclude = |ids: Vec<i64>| ids.into_iter().any(|id| Some(id) != exclude_id);

    // actions_no_dupe_events uses daterange(d-5, d+5, '[]'), which Postgres
    // canonicalizes to [d-5, d+6); two such ranges overlap up to 10 days apart.
    let same_event: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM actions
         WHERE lower(establishment) = lower($1)
           AND coalesce(lower(city), '') = lower($2)
           AND action_type = $3
           AND action_date BETWEEN $4 - 10 AND $4 + 10",
    )
    .bind(r.establishment())
    .bind(r.city().as_str())
    .bind(type_name)
    .bind(r.action_date())
    .fetch_all(&mut *conn)
    .await?;
    if exclude(same_event) {
        return Ok(true);
    }

    // UNIQUE (source_url, establishment, action_date).
    let same_identity: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM actions
         WHERE source_url = $1 AND establishment = $2 AND action_date = $3",
    )
    .bind(r.source_url())
    .bind(r.establishment())
    .bind(r.action_date())
    .fetch_all(&mut *conn)
    .await?;
    Ok(exclude(same_identity))
}

/// `EventKey` is the dedup authority; the SQL constraints are the backstop.
pub(crate) async fn upsert_actions(pool: &PgPool, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
    let mut tx = pool.begin().await?;
    let mut outcome = StoreOutcome::default();
    for r in rows {
        // Same source + same event = updated representation (every column refreshed).
        // Different source + same event + same action type = one event, skipped.
        let key = EventKey::from_row(r.establishment(), Some(r.city().as_str()), r.area());

        // Exact date: a second event from the same article on a nearby date is
        // a different row, never an update target.
        let stored: Vec<StoredRow> =
            sqlx::query_as::<_, (i64, String, Option<String>, Option<String>, String)>(
                "SELECT id, establishment, city, area, action_type FROM actions
             WHERE source_url = $1 AND action_date = $2 ORDER BY id",
            )
            .bind(r.source_url())
            .bind(r.action_date())
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(|(id, name, city, area, action_type)| StoredRow {
                id,
                name,
                city,
                area,
                action_type,
            })
            .collect();

        if let Some(id) = pick_target(r, &stored) {
            if collision_exists(&mut tx, r, Some(id)).await? {
                outcome.skipped += 1;
                continue;
            }
            execute_update(&mut tx, r, id).await?;
            outcome.updated += 1;
            continue;
        }

        // Another outlet reporting the same event is not a new card.
        let others: Vec<(String, Option<String>, Option<String>)> =
            sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT establishment, city, area FROM actions
                 WHERE action_date BETWEEN $1 - 10 AND $1 + 10
                   AND action_type = $2
                   AND source_url <> $3",
            )
            .bind(r.action_date())
            .bind(r.action_type().to_string())
            .bind(r.source_url())
            .fetch_all(&mut *tx)
            .await?;
        if others.iter().any(|(name, city, area)| {
            key.matches(&EventKey::from_row(name, city.as_deref(), area.as_deref()))
        }) || collision_exists(&mut tx, r, None).await?
        {
            outcome.skipped += 1;
            continue;
        }

        outcome.inserted +=
            usize::try_from(execute_insert(&mut tx, r, INSERT_CONFLICT_TAIL).await?)?;
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
    details: &Value,
) -> AnyResult<()> {
    let details_json = report.to_details(details);

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
    use super::*;
    use crate::domain::Article;
    use crate::models::{ActionType, LlmAction, SanitizedAction};

    fn candidate(name: &str, action_type: ActionType) -> Candidate {
        let action = LlmAction {
            establishment: name.into(),
            city: Some("Mumbai".into()),
            action_type,
            action_date: Some("2026-11-23".into()),
            details: Some(
                "licence suspended over cockroach infestation found during inspection".into(),
            ),
            ..Default::default()
        };
        let article = Article {
            title: "source headline".into(),
            url: "https://example.test/1".into(),
            source: None,
            published: None,
            snippet: None,
        };
        Candidate::try_new(SanitizedAction::new(action), &article).expect("valid candidate")
    }

    fn stored(id: i64, name: &str, action_type: &str) -> StoredRow {
        StoredRow {
            id,
            name: name.into(),
            city: Some("Mumbai".into()),
            area: None,
            action_type: action_type.into(),
        }
    }

    #[test]
    fn target_prefers_exact_name_and_type_over_an_earlier_alias() {
        let r = candidate(
            "National Sports Club of India",
            ActionType::LicenceSuspension,
        );
        let rows = [
            stored(1, "NSCI", "licence_suspension"),
            stored(2, "National Sports Club of India", "inspection"),
        ];
        assert_eq!(
            pick_target(&r, &rows),
            Some(2),
            "a same-source re-store must update the exact-name row, not the alias"
        );
    }

    #[test]
    fn target_falls_back_to_the_lowest_matching_alias() {
        let r = candidate(
            "Mumbai Cricket Association (BKC Facility)",
            ActionType::LicenceSuspension,
        );
        let rows = [
            stored(4, "MCA BKC Club", "licence_suspension"),
            stored(7, "Mumbai Cricket Association Club", "licence_suspension"),
        ];
        assert_eq!(
            pick_target(&r, &rows),
            Some(4),
            "with no exact name, the lowest matching alias wins"
        );
    }
}
