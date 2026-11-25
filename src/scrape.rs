use anyhow::{Result as AnyResult, bail};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use tracing::{instrument, warn};

use crate::config::Config;
use crate::context::AppContext;
use crate::llm;
use crate::news;
use crate::report::{RunMetrics, RunReport, StoreOutcome};

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
            if let Err(error) = store
                .finish_run(run_id, &report, &json!({"model": config.llm.model}))
                .await
            {
                let _ = store
                    .fail_run(run_id, &format!("finish_run failed: {error:#}"))
                    .await;
                return Err(error);
            }
            Ok(report)
        }
        Err(error) => {
            if let Err(fail_error) = store.fail_run(run_id, &format!("{error:#}")).await {
                warn!(
                    error = %format!("{fail_error:#}"),
                    "fail_run failed; preserving the scrape error"
                );
            }
            Err(error)
        }
    }
}

fn is_degraded(llm_degraded: bool, metrics: &RunMetrics) -> bool {
    llm_degraded || metrics.rss_failures > 0 || metrics.article_failures > 0
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

    let fetch = news::fetch_items(&ctx.http).await?;
    if fetch.successful_queries == 0 {
        bail!("all {} rss queries failed", fetch.failed_queries);
    }
    let already_seen = store.seen_urls(seen).await?;
    let articles_seen = fetch.items.len();
    let (articles, article_failures) =
        news::enrich(&ctx.http, fetch.items, &already_seen, max_items).await;
    let articles_new = articles.len();

    let mut extraction = llm::extract(config, &articles).await?;
    extraction.metrics.rss_failures = fetch.failed_queries;
    extraction.metrics.article_failures = article_failures;
    let StoreOutcome {
        inserted, updated, ..
    } = store.upsert_actions(&extraction.candidates).await?;
    let degraded = is_degraded(extraction.degraded, &extraction.metrics);

    Ok(RunReport {
        articles_seen,
        articles_new,
        actions_upserted: inserted + updated,
        metrics: extraction.metrics,
        degraded,
    })
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use test_case::test_case;

    use super::is_degraded;
    use crate::domain::{Article, Candidate, CoveredCity};
    use crate::error::Rejection;
    use crate::models::{ActionType, LlmAction, SanitizedAction};
    use crate::report::RunMetrics;

    #[test]
    fn partial_rss_or_article_failures_mark_the_run_degraded() {
        let clean = RunMetrics::default();
        assert!(!is_degraded(false, &clean));
        let rss = RunMetrics {
            rss_failures: 1,
            ..RunMetrics::default()
        };
        let articles = RunMetrics {
            article_failures: 1,
            ..RunMetrics::default()
        };
        assert!(is_degraded(false, &rss));
        assert!(is_degraded(false, &articles));
        assert!(is_degraded(true, &clean));
    }

    fn article(published: Option<chrono::DateTime<Utc>>) -> Article {
        Article {
            title: "Domino's licence suspended in Mumbai".into(),
            url: "https://a.test/1".into(),
            source: Some("Test Press".into()),
            published,
            snippet: Some("licence suspended over cockroach infestation".into()),
        }
    }

    fn action() -> LlmAction {
        LlmAction {
            establishment: "Domino's Vile Parle".into(),
            city: Some("Mumbai".into()),
            action_type: ActionType::LicenceSuspension,
            action_date: Some("2026-08-11".into()),
            details: Some(
                "licence suspended over cockroach infestation found during inspection".into(),
            ),
            ..Default::default()
        }
    }

    fn try_new(action: LlmAction, source: &Article) -> Result<Candidate, Rejection> {
        Candidate::try_new(SanitizedAction::new(action), source)
    }

    fn published() -> Option<chrono::DateTime<Utc>> {
        Some(
            Utc.with_ymd_and_hms(2026, 11, 18, 9, 0, 0)
                .single()
                .expect("valid test date"),
        )
    }

    #[test]
    fn candidate_keeps_its_source_provenance() {
        let mut action = action();
        action.brand = Some("Domino's".into());
        action.outlet_type = Some("restaurant".into());
        let source = article(published());
        let candidate = try_new(action, &source).expect("valid");

        assert_eq!(candidate.source_url(), "https://a.test/1");
        assert_eq!(candidate.source_publisher(), Some("Test Press"));
        assert_eq!(
            candidate.source_headline(),
            Some("Domino's licence suspended in Mumbai")
        );
        assert_eq!(candidate.brand(), Some("Domino's"));
        assert_eq!(candidate.action_type(), ActionType::LicenceSuspension);
        assert_eq!(candidate.city(), CoveredCity::Mumbai);
    }

    #[test]
    fn rejects_out_of_scope_city() {
        let mut action = action();
        action.city = Some("Nashik".into());
        assert_eq!(
            try_new(action, &article(published())).err(),
            Some(Rejection::OutOfScopeCity),
            "uncovered city is rejected at the boundary"
        );
    }

    #[test]
    fn rejects_missing_action_date() {
        let mut action = action();
        action.action_date = None;
        assert_eq!(
            try_new(action, &article(None)).err(),
            Some(Rejection::NoActionDate),
            "no reported or published date means no fabricated date"
        );
    }

    #[test]
    fn unknown_outlet_code_becomes_none() {
        let mut action = action();
        action.outlet_type = Some("pavement stand".into());
        let candidate = try_new(action, &article(published())).expect("valid candidate");
        assert_eq!(
            candidate.outlet_type(),
            None,
            "unknown codes are not an error sink"
        );
    }

    #[test_case(Some(54), Some(54); "in range")]
    #[test_case(Some(999), None; "too high")]
    #[test_case(Some(-5), None; "negative")]
    #[test_case(Some(101), None; "just out of range")]
    #[test_case(None, None; "absent")]
    fn compliance_score_keeps_only_0_to_100(score: Option<i32>, expected: Option<i32>) {
        let mut action = action();
        action.compliance_score = score;
        let candidate = try_new(action, &article(published())).expect("valid candidate");
        assert_eq!(candidate.compliance_score(), expected, "{score:?}");
    }
}
