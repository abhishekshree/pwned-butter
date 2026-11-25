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

/// Static statement: no fragments, no runtime SQL assembly. The conflict key
/// mirrors `actions_source_identity` in migration 0004.
const INSERT_SQL: &str = "INSERT INTO actions (
                 establishment, area, city, brand, operator, outlet_type, action_type,
                 action_date, violations, compliance_score, fssai_number, details,
                 platforms, source_url, source_publisher, source_headline, published_at
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
                 ON CONFLICT (source_url, establishment, action_date,
                              (coalesce(city, '')), (coalesce(area, '')))
                 DO UPDATE SET
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

async fn execute_insert(conn: &mut sqlx::PgConnection, r: &Candidate) -> AnyResult<u64> {
    let res = bind_candidate(sqlx::query(INSERT_SQL), r)
        .execute(conn)
        .await?;
    Ok(res.rows_affected())
}

async fn execute_update(conn: &mut sqlx::PgConnection, r: &Candidate, id: i64) -> AnyResult<()> {
    bind_candidate(sqlx::query(UPDATE_BY_ID), r)
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

/// Mirrors both constraints in migration 0004: the city/area/type ±10 day
/// exclusion and the per-source identity unique key. `exclude_id` lets an
/// update ignore the row it is about to rewrite.
async fn collision_exists(
    conn: &mut sqlx::PgConnection,
    r: &Candidate,
    exclude_id: Option<i64>,
) -> AnyResult<bool> {
    // daterange(d-5, d+5, '[]') canonicalizes to [d-5, d+6), so two rows can
    // conflict up to 10 days apart — the window checked here.
    let collision: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM actions
             WHERE lower(establishment) = lower($1)
               AND coalesce(lower(city), '') = lower($2)
               AND coalesce(lower(area), '') = lower($3)
               AND action_type = $4
               AND action_date BETWEEN $5 - 10 AND $5 + 10
               AND ($6::bigint IS NULL OR id <> $6)
         ) OR EXISTS (
             SELECT 1 FROM actions
             WHERE source_url = $7
               AND establishment = $8
               AND action_date = $9
               AND lower(coalesce(city, '')) = lower($2)
               AND lower(coalesce(area, '')) = lower($3)
               AND ($6::bigint IS NULL OR id <> $6)
         )",
    )
    .bind(r.establishment())
    .bind(r.city().as_str())
    .bind(r.area().unwrap_or(""))
    .bind(r.action_type().to_string())
    .bind(r.action_date())
    .bind(exclude_id)
    .bind(r.source_url())
    .bind(r.establishment())
    .bind(r.action_date())
    .fetch_one(&mut *conn)
    .await?;
    Ok(collision)
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

        outcome.inserted += usize::try_from(execute_insert(&mut tx, r).await?)?;
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

    fn db_candidate(
        name: &str,
        city: &str,
        area: Option<&str>,
        date: &str,
        url: &str,
        details: &str,
    ) -> Candidate {
        let action = LlmAction {
            establishment: name.into(),
            city: Some(city.into()),
            area: area.map(str::to_string),
            action_type: ActionType::LicenceSuspension,
            action_date: Some(date.into()),
            details: Some(details.into()),
            ..Default::default()
        };
        let article = Article {
            title: "source headline".into(),
            url: url.into(),
            source: None,
            published: None,
            snippet: None,
        };
        Candidate::try_new(SanitizedAction::new(action), &article).expect("valid candidate")
    }

    async fn row_count(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM actions")
            .fetch_one(pool)
            .await
            .expect("count rows")
    }

    /// Real Postgres coverage for the identity policy. Skipped by default so
    /// the ordinary unit suite stays offline; run with RUN_DB_TESTS=1.
    #[tokio::test]
    #[ignore = "requires RUN_DB_TESTS=1 and DATABASE_URL"]
    async fn postgres_enforces_the_identity_policy() {
        if std::env::var("RUN_DB_TESTS").ok().as_deref() != Some("1") {
            return;
        }
        let raw_url = std::env::var("DATABASE_URL").expect("DATABASE_URL for db tests");
        let schema = format!("it_{}", std::process::id());
        let schema_for_pool = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(move |conn, _| {
                let schema = schema_for_pool.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('search_path', $1, false)")
                        .bind(schema)
                        .execute(conn)
                        .await
                        .map(|_| ())
                })
            })
            .connect(SecretString::new(raw_url.clone()).expose())
            .await
            .expect("connect to test database");
        // The identifier is generated from the process id, never user input.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
            .execute(&pool)
            .await
            .expect("create isolated schema");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("apply migrations in test schema");

        const DETAILS: &str =
            "licence suspended over cockroach infestation found during inspection";
        let borivali = db_candidate(
            "Domino's Pizza",
            "Mumbai",
            Some("Borivali West"),
            "2026-11-23",
            "https://news.test/a",
            DETAILS,
        );
        let ghatkopar = db_candidate(
            "Domino's Pizza",
            "Mumbai",
            Some("Ghatkopar West"),
            "2026-11-23",
            "https://news.test/b",
            DETAILS,
        );
        let out = upsert_actions(&pool, &[borivali, ghatkopar])
            .await
            .expect("upsert two branches");
        assert_eq!(
            (out.inserted, out.updated, out.skipped),
            (2, 0, 0),
            "two branches of one name in one city are distinct events"
        );
        assert_eq!(row_count(&pool).await, 2);

        let pune = db_candidate(
            "Domino's Pizza",
            "Pune",
            Some("Borivali West"),
            "2026-11-23",
            "https://news.test/c",
            DETAILS,
        );
        let out = upsert_actions(&pool, &[pune])
            .await
            .expect("upsert other city");
        assert_eq!(
            (out.inserted, out.updated, out.skipped),
            (1, 0, 0),
            "the same outlet in a different city is a distinct event"
        );
        assert_eq!(row_count(&pool).await, 3);

        let revised = db_candidate(
            "Domino's Pizza",
            "Mumbai",
            Some("Borivali West"),
            "2026-11-23",
            "https://news.test/a",
            "licence suspended after a fresh inspection found cockroach infestation",
        );
        let out = upsert_actions(&pool, &[revised])
            .await
            .expect("re-store same source URL");
        assert_eq!(
            (out.inserted, out.updated, out.skipped),
            (0, 1, 0),
            "a second copy from one URL updates the row instead of inserting"
        );
        assert_eq!(row_count(&pool).await, 3);
        let details: String = sqlx::query_scalar(
            "SELECT details FROM actions WHERE source_url = 'https://news.test/a'",
        )
        .fetch_one(&pool)
        .await
        .expect("stored details");
        assert!(
            details.contains("fresh inspection"),
            "the re-store refreshed the details column"
        );

        let re_report = db_candidate(
            "Domino's Pizza",
            "Mumbai",
            Some("Borivali West"),
            "2026-11-26",
            "https://news.test/d",
            DETAILS,
        );
        let out = upsert_actions(&pool, &[re_report])
            .await
            .expect("re-report inside the window");
        assert_eq!(
            (out.inserted, out.updated, out.skipped),
            (0, 0, 1),
            "a re-report three days later is one event"
        );
        assert_eq!(row_count(&pool).await, 3);

        let later = db_candidate(
            "Domino's Pizza",
            "Mumbai",
            Some("Borivali West"),
            "2026-12-04",
            "https://news.test/e",
            DETAILS,
        );
        let out = upsert_actions(&pool, &[later])
            .await
            .expect("event outside the window");
        assert_eq!(
            (out.inserted, out.updated, out.skipped),
            (1, 0, 0),
            "eleven days later is outside the ±10 day window"
        );
        assert_eq!(row_count(&pool).await, 4);

        pool.close().await;
        let admin = super::pool(&SecretString::new(raw_url))
            .await
            .expect("admin pool");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"
        )))
        .execute(&admin)
        .await
        .expect("drop isolated schema");
        admin.close().await;
    }
}
