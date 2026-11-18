use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use tracing::{info, warn};

use crate::db::{self, ActionInsert};
use crate::llm;
use crate::models::{LlmAction, NewsItem, OutletType, coerce_action_date, nonempty};
use crate::news;

#[derive(Debug, Clone)]
pub struct ScrapeReport {
    pub articles_seen: usize,
    pub articles_new: usize,
    pub actions_upserted: usize,
    pub llm_calls: usize,
}

pub async fn run_scrape(gemini_key: &str, model: &str) -> Result<(i64, ScrapeReport)> {
    // Google News indexes regional enforcement reports hours after publication
    // surprisingly often. Search an overlapping window and rely on source and
    // event deduplication rather than making each daily run a one-shot.
    run_with_window(gemini_key, model, "when:3d", 21, news::MAX_ITEMS).await
}

pub async fn run_with_window(
    gemini_key: &str,
    model: &str,
    window: &str,
    seen_days: i64,
    max_items: usize,
) -> Result<(i64, ScrapeReport)> {
    let pool = db::pool().await?;
    let run_id = db::begin_run(pool).await?;

    match scrape_once(pool, gemini_key, model, window, seen_days, max_items).await {
        Ok(report) => {
            db::finish_run(
                pool,
                run_id,
                &report,
                &json!({"model": model, "window": window}),
            )
            .await?;
            Ok((run_id, report))
        }
        Err(e) => {
            db::fail_run(pool, run_id, &format!("{e:#}")).await?;
            Err(e)
        }
    }
}

async fn scrape_once(
    pool: &sqlx::PgPool,
    gemini_key: &str,
    model: &str,
    window: &str,
    seen_days: i64,
    max_items: usize,
) -> Result<ScrapeReport> {
    let client = crate::http_client();
    let seen = Utc::now() - ChronoDuration::days(seen_days);

    let items = news::fetch_items(client, window).await?;
    let already_seen = db::seen_urls(pool, seen).await?;
    let articles_seen = items.len();
    let fresh = news::enrich(client, items, &already_seen, max_items).await;

    let (actions, mut llm_calls) = llm::extract(gemini_key, model, &fresh).await?;
    let rows = build_rows(&fresh, &actions);
    let (rows, collapse_calls) = collapse_dupes(pool, gemini_key, model, rows).await;
    llm_calls += collapse_calls;
    let upserted = db::upsert_actions(pool, &rows).await?;

    Ok(ScrapeReport {
        articles_seen,
        articles_new: fresh.len(),
        actions_upserted: upserted,
        llm_calls,
    })
}

/// LLM pass that collapses re-reported events before insert; falls back to
/// the db.rs name heuristics alone when the DB lookup or the call fails.
async fn collapse_dupes(
    pool: &sqlx::PgPool,
    gemini_key: &str,
    model: &str,
    rows: Vec<ActionInsert>,
) -> (Vec<ActionInsert>, usize) {
    if rows.is_empty() {
        return (rows, 0);
    }
    let recent = match db::recent_events(pool, 10).await {
        Ok(recent) => recent,
        Err(e) => {
            warn!("recent_events unavailable ({e:#}); skipping dupe-collapse pass");
            return (rows, 0);
        }
    };
    let (drops, calls) = llm::collapse_dupes(gemini_key, model, &rows, &recent).await;
    if drops.is_empty() {
        return (rows, calls);
    }
    info!(
        "dupe-collapse dropping {} record(s): {drops:?}",
        drops.len()
    );
    let drop_set: std::collections::HashSet<usize> = drops.into_iter().collect();
    let kept = rows
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !drop_set.contains(i))
        .map(|(_, r)| r)
        .collect();
    (kept, calls)
}

/// Cities the tracker covers. The FDA brief is statewide, so a roundup that
/// names outlets in Nashik or Mahabaleshwar still parses — drop them at the
/// insert boundary instead of showing places the site has no section for.
const IN_SCOPE_CITIES: &[&str] = &["mumbai", "pune", "thane", "navi mumbai", "lonavala"];

/// Case-insensitive: "Mumbai Suburban" is still Mumbai, "Navi Mumbai" needs
/// its own entry because the prefix does not match.
fn in_scope(city: Option<&str>) -> bool {
    city.is_some_and(|city| {
        let city = city.trim().to_lowercase();
        IN_SCOPE_CITIES.iter().any(|keep| {
            city.strip_prefix(keep)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
    })
}

pub fn build_rows(items: &[NewsItem], actions: &[LlmAction]) -> Vec<ActionInsert> {
    actions
        .iter()
        .filter_map(|a| {
            let item = items.get(a.source_index)?;
            let establishment = nonempty(Some(a.establishment.as_str()))?;
            let city = nonempty(a.city.as_deref());
            if !in_scope(city.as_deref()) {
                return None;
            }
            Some(ActionInsert {
                establishment,
                area: nonempty(a.area.as_deref()),
                city,
                brand: nonempty(a.brand.as_deref()),
                operator: nonempty(a.operator.as_deref()),
                outlet_type: a.outlet_type.as_deref().map(OutletType::from),
                action_type: a.action_type,
                action_date: coerce_action_date(a.action_date.as_deref(), item.published),
                violations: a.violations.clone(),
                compliance_score: a.compliance_score.filter(|s| (0..=100).contains(s)),
                fssai_number: nonempty(a.fssai_number.as_deref()),
                details: nonempty(a.details.as_deref()),
                platforms: a.platforms.clone(),
                source_url: item.url.clone(),
                source_publisher: nonempty(item.source.as_deref()),
                source_headline: Some(item.title.clone()),
                published_at: item.published,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActionType;
    use chrono::Utc;

    fn item(title: &str, url: &str) -> NewsItem {
        NewsItem {
            title: title.into(),
            url: url.into(),
            source: Some("Test Press".into()),
            published: Some(Utc::now()),
            snippet: None,
        }
    }

    #[test]
    fn builds_rows_from_sources() {
        let items = vec![item(
            "Domino's licence suspended in Mumbai",
            "https://a.test/1",
        )];
        let actions = vec![LlmAction {
            establishment: "Domino's Vile Parle".into(),
            area: Some("Vile Parle West".into()),
            city: Some("Mumbai".into()),
            brand: Some("Domino's".into()),
            outlet_type: Some("restaurant".into()),
            action_type: ActionType::LicenceSuspension,
            action_date: Some("2026-08-11".into()),
            violations: vec!["pest control lapses".into()],
            compliance_score: Some(54),
            platforms: vec!["zomato".into()],
            source_index: 0,
            ..Default::default()
        }];
        let rows = build_rows(&items, &actions);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].brand.as_deref(), Some("Domino's"));
        assert_eq!(rows[0].action_type, ActionType::LicenceSuspension);
        assert_eq!(rows[0].outlet_type, Some(OutletType::Restaurant));
        assert_eq!(rows[0].source_url, "https://a.test/1");
    }

    #[test]
    fn unknown_outlet_type_maps_to_other() {
        let items = vec![item("a", "b")];
        let action = LlmAction {
            establishment: "X".into(),
            city: Some("Mumbai".into()),
            outlet_type: Some("pavement stand".into()),
            action_type: ActionType::Inspection,
            source_index: 0,
            ..Default::default()
        };
        let rows = build_rows(&items, &[action]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outlet_type, Some(OutletType::Other));
    }

    fn with_city(city: &str) -> LlmAction {
        LlmAction {
            establishment: "X".into(),
            city: Some(city.into()),
            action_type: ActionType::LicenceSuspension,
            source_index: 0,
            ..Default::default()
        }
    }

    #[test]
    fn keeps_only_the_covered_cities() {
        let items = vec![item("a", "b")];
        let kept = build_rows(
            &items,
            &[
                with_city("Mumbai"),
                with_city("pune"),
                with_city("Pune City"),
                with_city("Thane"),
                with_city("Thane West"),
                with_city("Navi Mumbai"),
                with_city("Lonavala"),
            ],
        );
        assert_eq!(kept.len(), 7, "all covered cities are kept");

        let dropped = build_rows(
            &items,
            &[
                with_city("Mahabaleshwar"),
                with_city("Nashik"),
                with_city("Panvel"),
                with_city("Bhiwandi"),
            ],
        );
        assert!(dropped.is_empty(), "uncovered cities are out of scope");
    }

    #[test]
    fn drops_outlet_with_no_city() {
        let items = vec![item("a", "b")];
        let action = LlmAction {
            establishment: "X".into(),
            action_type: ActionType::Inspection,
            source_index: 0,
            ..Default::default()
        };
        assert!(build_rows(&items, &[action]).is_empty());
    }

    fn scored(score: Option<i32>) -> LlmAction {
        LlmAction {
            establishment: "X".into(),
            city: Some("Mumbai".into()),
            action_type: ActionType::Inspection,
            compliance_score: score,
            source_index: 0,
            ..Default::default()
        }
    }

    #[test]
    fn compliance_score_keeps_only_0_to_100() {
        let items = vec![item("a", "b")];
        let cases = [
            (Some(54), Some(54)),
            (Some(999), None),
            (Some(-5), None),
            (Some(101), None),
            (None, None),
        ];
        for (score, expected) in cases {
            assert_eq!(
                build_rows(&items, &[scored(score)])[0].compliance_score,
                expected,
                "{score:?}"
            );
        }
    }
}
