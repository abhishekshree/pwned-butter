use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;

use crate::db::{self, ActionInsert};
use crate::llm;
use crate::models::{canonical_outlet_type, coerce_action_date, nonempty, LlmAction, NewsItem};
use crate::news;

#[derive(Debug, Clone)]
pub struct ScrapeReport {
    pub articles_seen: usize,
    pub articles_new: usize,
    pub actions_upserted: usize,
    pub llm_calls: usize,
}

pub async fn run_scrape(gemini_key: &str, model: &str) -> Result<(i64, ScrapeReport)> {
    run_with_window(gemini_key, model, "when:1d", 14, news::MAX_ITEMS, false).await
}

pub async fn run_with_window(
    gemini_key: &str,
    model: &str,
    window: &str,
    seen_days: i64,
    max_items: usize,
    delivery: bool,
) -> Result<(i64, ScrapeReport)> {
    let pool = db::pool().await?;
    let run_id = db::begin_run(pool).await?;

    match scrape_once(
        pool, gemini_key, model, window, seen_days, max_items, delivery,
    )
    .await
    {
        Ok(report) => {
            db::finish_run(
                pool,
                run_id,
                report.articles_seen,
                report.articles_new,
                report.actions_upserted,
                report.llm_calls,
                &json!({"model": model, "window": window, "delivery": delivery}),
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
    delivery: bool,
) -> Result<ScrapeReport> {
    let client = crate::http_client();
    let seen = Utc::now() - ChronoDuration::days(seen_days);

    let items = news::fetch_items(client, window).await?;
    let already_seen = db::seen_urls(pool, seen).await?;
    let articles_seen = items.len();
    let fresh = news::enrich(client, items, &already_seen, max_items).await;

    let (actions, llm_calls) = llm::extract(gemini_key, model, &fresh, delivery).await?;
    let rows = build_rows(&fresh, &actions, delivery);
    let upserted = db::upsert_actions(pool, &rows).await?;

    Ok(ScrapeReport {
        articles_seen,
        articles_new: fresh.len(),
        actions_upserted: upserted,
        llm_calls,
    })
}

fn build_rows(items: &[NewsItem], actions: &[LlmAction], delivery: bool) -> Vec<ActionInsert> {
    actions
        .iter()
        .filter_map(|a| {
            if delivery && a.platforms.is_empty() {
                return None;
            }
            let item = items.get(a.source_index)?;
            let establishment = nonempty(Some(a.establishment.clone()))?;
            Some(ActionInsert {
                establishment,
                area: nonempty(a.area.clone()),
                city: nonempty(a.city.clone()),
                brand: nonempty(a.brand.clone()),
                operator: nonempty(a.operator.clone()),
                outlet_type: a.outlet_type.as_deref().map(canonical_outlet_type),
                action_type: a.action_type.to_string(),
                action_date: coerce_action_date(a.action_date.clone(), item.published),
                violations: a.violations.clone(),
                compliance_score: a.compliance_score.filter(|s| (0..=100).contains(s)),
                fssai_number: nonempty(a.fssai_number.clone()),
                details: nonempty(a.details.clone()),
                platforms: a.platforms.clone(),
                source_url: item.url.clone(),
                source_publisher: nonempty(item.source.clone()),
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
            operator: None,
            outlet_type: Some("restaurant".into()),
            action_type: ActionType::LicenceSuspension,
            action_date: Some("2026-08-11".into()),
            violations: vec!["pest control lapses".into()],
            compliance_score: Some(54),
            fssai_number: None,
            details: None,
            platforms: vec!["zomato".into()],
            source_index: 0,
        }];
        let rows = build_rows(&items, &actions, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].brand.as_deref(), Some("Domino's"));
        assert_eq!(rows[0].action_type, "licence_suspension");
        assert_eq!(rows[0].outlet_type.as_deref(), Some("restaurant"));
        assert_eq!(rows[0].source_url, "https://a.test/1");
    }

    #[test]
    fn unknown_outlet_type_maps_to_other() {
        let items = vec![item("a", "b")];
        let action = LlmAction {
            establishment: "X".into(),
            area: None,
            city: None,
            brand: None,
            operator: None,
            outlet_type: Some("pavement stand".into()),
            action_type: ActionType::Inspection,
            action_date: None,
            violations: vec![],
            compliance_score: None,
            fssai_number: None,
            details: None,
            platforms: vec![],
            source_index: 0,
        };
        let rows = build_rows(&items, &[action], false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outlet_type.as_deref(), Some("other"));
    }

    #[test]
    fn drops_orphan_source_index() {
        let items = vec![item("a", "b")];
        let action = LlmAction {
            establishment: "X".into(),
            area: None,
            city: None,
            brand: None,
            operator: None,
            outlet_type: None,
            action_type: ActionType::Inspection,
            action_date: None,
            violations: vec![],
            compliance_score: None,
            fssai_number: None,
            details: None,
            platforms: vec![],
            source_index: 5,
        };
        assert!(build_rows(&items, &[action], false).is_empty());
    }

    #[test]
    fn delivery_mode_drops_unlisted_outlets() {
        let items = vec![item("a", "b")];
        let mut action = LlmAction {
            establishment: "X".into(),
            area: None,
            city: Some("Mumbai".into()),
            brand: None,
            operator: None,
            outlet_type: Some("restaurant".into()),
            action_type: ActionType::Inspection,
            action_date: None,
            violations: vec![],
            compliance_score: None,
            fssai_number: None,
            details: None,
            platforms: vec![],
            source_index: 0,
        };
        assert!(build_rows(&items, &[action.clone()], true).is_empty());
        action.platforms = vec!["zomato".into(), "swiggy".into()];
        assert_eq!(build_rows(&items, &[action], true).len(), 1);
    }
}
