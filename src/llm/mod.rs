use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result as AnyResult, anyhow};
use serde_json::Value;
use tokio::time::sleep;
use tracing::{debug, info, instrument, warn};

use crate::config::{Config, Llm, Provider};
use crate::domain::{Article, Candidate, area_in_name};
use crate::report::RunMetrics;

pub const SYSTEM_PROMPT: &str = include_str!("../prompts/system.txt");
/// Best-effort alias resolution: the Rust matcher only knows consecutive initials.
pub(crate) const COLLAPSE_PROMPT: &str = include_str!("../prompts/collapse.txt");

/// Free models truncate long JSON output; keep extraction responses short.
const EXTRACT_BATCH_SIZE: usize = 4;
/// One window per call keeps both the input and output token limits clear.
const TPM_PAUSE: Duration = Duration::from_secs(60);
// ponytail: serial calls avoid concurrent bursts tripping the same window.

mod genai;
mod triage;

use genai::ExtractOptions;
use triage::Verdict;

/// The result of an LLM run, including degradation the caller must not ignore.
pub struct Extraction {
    pub candidates: Vec<Candidate>,
    pub metrics: RunMetrics,
    pub degraded: bool,
}

/// Each chunk addresses only itself, so no absolute index survives batching.
fn verdict_article<'a>(chunk: &'a [Article], verdict: &Verdict) -> Option<&'a Article> {
    chunk.get(verdict.source_index)
}

/// Namespaced model IDs for the active vendor, primary first.
fn models_for(llm: &Llm) -> Vec<String> {
    match llm.provider {
        Provider::Gemini => vec![llm.model.clone()],
        Provider::OpenRouter => prefixed(&llm.model, &llm.fallbacks, "open_router"),
        Provider::Groq => prefixed(&llm.model, &llm.fallbacks, "groq"),
    }
}

fn prefixed(model: &str, fallbacks: &[String], namespace: &str) -> Vec<String> {
    let mut names = vec![format!("{namespace}::{model}")];
    names.extend(
        fallbacks
            .iter()
            .filter(|fallback| *fallback != model)
            .map(|fallback| format!("{namespace}::{fallback}")),
    );
    names
}

/// Extraction needs a large output budget, which Groq's reasoning model has;
/// the smaller model stays first for the compact triage call.
fn extraction_models(llm: &Llm) -> Vec<String> {
    if llm.provider != Provider::Groq {
        return models_for(llm);
    }
    let mut names = llm.fallbacks.clone();
    if !names.contains(&llm.model) {
        names.push(llm.model.clone());
    }
    names
        .into_iter()
        .map(|name| format!("groq::{name}"))
        .collect()
}

/// One compact call filters every article; no provider verdict drops the run.
#[instrument(skip_all, fields(total = items.len()))]
async fn triage<'a>(
    client: &genai::Client,
    models: &[String],
    items: &'a [Article],
    structured: bool,
) -> AnyResult<(Vec<&'a Article>, RunMetrics)> {
    if items.is_empty() {
        return Ok((Vec::new(), RunMetrics::default()));
    }
    let mut metrics = RunMetrics::default();
    let mut verdicts = None;
    for model in models {
        let (result, calls) = genai::triage_batch(client, model, items, structured).await;
        metrics.triage_calls += calls;
        match result {
            Ok(found) => {
                verdicts = Some(found);
                break;
            }
            Err(error) => warn!(
                model = %model,
                error = %format!("{error:#}"),
                "triage model failed"
            ),
        }
    }
    let Some(verdicts) = verdicts else {
        metrics.triage_failures += 1;
        warn!("dropping triage after all providers failed");
        return Ok((Vec::new(), metrics));
    };

    let mut kept = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for verdict in verdicts {
        let Some(item) = verdict_article(items, &verdict) else {
            warn!(
                index = verdict.source_index,
                "triage verdict index out of range"
            );
            continue;
        };
        if seen.insert(item.url.as_str()) {
            debug!(url = %item.url, reason = ?verdict.reason, "triage kept item");
            kept.push(item);
        }
    }
    Ok((kept, metrics))
}

#[instrument(skip_all, fields(total = items.len(), model = %config.llm.model))]
pub(crate) async fn extract(config: &Config, items: &[Article]) -> AnyResult<Extraction> {
    let structured = config.llm.structured_output();
    let client = genai::client(&config.llm)?;
    let triage_models = models_for(&config.llm);
    let (articles, mut metrics) = triage(&client, &triage_models, items, structured).await?;
    info!(relevant = articles.len(), total = items.len(), "llm triage");
    if articles.is_empty() {
        let degraded = metrics.triage_failures > 0;
        return Ok(Extraction {
            candidates: Vec::new(),
            metrics,
            degraded,
        });
    }

    // Triage just spent the minute's token budget; let it reset before details.
    sleep(TPM_PAUSE).await;

    let models = extraction_models(&config.llm);
    let opts = ExtractOptions {
        models: &models,
        structured,
    };

    let mut candidates = Vec::new();
    let mut failed = 0;
    for (batch_no, chunk) in articles.chunks(EXTRACT_BATCH_SIZE).enumerate() {
        if batch_no > 0 {
            sleep(TPM_PAUSE).await;
        }
        let (result, calls) = genai::extract_batch(&client, chunk, &opts).await;
        metrics.extraction_calls += calls;
        match result {
            Ok(batch) => {
                for (article, action) in batch {
                    let action = genai::sanitize_action(action);
                    match Candidate::try_new(action, article) {
                        Ok(candidate) => candidates.push(candidate),
                        Err(rejection) => {
                            metrics.rejected += 1;
                            warn!(reason = %rejection, url = %article.url, "dropping rejected LLM record");
                        }
                    }
                }
            }
            Err(error) => {
                failed += 1;
                metrics.extraction_failures += 1;
                warn!(
                    batch = batch_no,
                    error = %format!("{error:#}"),
                    "llm batch failed"
                );
            }
        }
    }
    if failed > 0 && failed == articles.len().div_ceil(EXTRACT_BATCH_SIZE) {
        return Err(anyhow!("all llm batches failed"));
    }

    if candidates.len() > 1
        && let Some(model) = triage_models.first()
    {
        let (text, calls) = genai::collapse_batch(&client, model, &candidates).await;
        metrics.extraction_calls += calls;
        match text.and_then(|text| parse_collapse(&text)) {
            Ok(groups) => apply_collapse(&mut candidates, &groups),
            Err(error) => {
                metrics.extraction_failures += 1;
                warn!(error = %format!("{error:#}"), "collapse failed; keeping extracted names");
            }
        }
    }

    info!(
        candidates = candidates.len(),
        failed_batches = failed,
        "llm extraction done"
    );
    for candidate in &candidates {
        debug!(
            establishment = %candidate.establishment(),
            city = candidate.city().as_str(),
            action_type = %candidate.action_type(),
            action_date = %candidate.action_date(),
            "record"
        );
    }
    let degraded = metrics.triage_failures > 0 || metrics.extraction_failures > 0;
    Ok(Extraction {
        candidates,
        metrics,
        degraded,
    })
}

/// A proposed rename; ids are candidate indices validated before use.
pub(crate) struct CollapseGroup {
    pub ids: Vec<usize>,
    pub canonical: String,
}

/// Duplicate ids and singleton groups are dropped; range and shape are checked at apply time.
fn parse_collapse(text: &str) -> AnyResult<Vec<CollapseGroup>> {
    let text = strip_code_fences(text);
    let parsed: Value = serde_json::from_str(&text).context("parse collapse response")?;
    let raw = match &parsed {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("groups")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("collapse response missing \"groups\" array"))?,
        _ => return Err(anyhow!("unexpected collapse response shape")),
    };
    let mut groups = Vec::new();
    for value in raw {
        let Some(object) = value.as_object() else {
            continue;
        };
        let Some(canonical) = object.get("canonical").and_then(Value::as_str) else {
            continue;
        };
        let canonical = canonical.trim();
        if canonical.is_empty() || canonical.chars().count() > 200 {
            continue;
        }
        let mut ids: Vec<usize> = object
            .get("ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_u64().and_then(|id| usize::try_from(id).ok()))
            .collect();
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(*id));
        if ids.len() >= 2 {
            groups.push(CollapseGroup {
                ids,
                canonical: canonical.to_string(),
            });
        }
    }
    Ok(groups)
}

/// Rejects groups that are ambiguous or span different real-world events.
fn valid_collapse(
    candidates: &[Candidate],
    group: &CollapseGroup,
    shared: &HashSet<usize>,
) -> bool {
    if group.ids.is_empty()
        || group
            .ids
            .iter()
            .any(|&id| id >= candidates.len() || shared.contains(&id))
    {
        return false;
    }
    let Some(members) = group
        .ids
        .iter()
        .map(|&id| candidates.get(id))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let Some(first) = members.first() else {
        return false;
    };
    if members
        .iter()
        .any(|c| c.action_type() != first.action_type() || c.city() != first.city())
    {
        return false;
    }
    let (mut earliest, mut latest) = (first.action_date(), first.action_date());
    for candidate in &members {
        earliest = earliest.min(candidate.action_date());
        latest = latest.max(candidate.action_date());
    }
    if latest.signed_duration_since(earliest).num_days() > 10 {
        return false;
    }
    for (index, a) in members.iter().enumerate() {
        for b in members.iter().skip(index + 1) {
            match (a.area(), b.area()) {
                (Some(x), Some(y)) if x != y && !x.contains(y) && !y.contains(x) => return false,
                (Some(x), None) if !area_in_name(x, b.establishment()) => return false,
                (None, Some(y)) if !area_in_name(y, a.establishment()) => return false,
                _ => {}
            }
        }
    }
    true
}

fn apply_collapse(candidates: &mut [Candidate], groups: &[CollapseGroup]) {
    // An index claimed by two groups is ambiguous, so neither group is applied.
    let mut seen = HashSet::new();
    let mut shared = HashSet::new();
    for id in groups.iter().flat_map(|group| group.ids.iter()) {
        if !seen.insert(*id) {
            shared.insert(*id);
        }
    }

    for group in groups {
        if !valid_collapse(candidates, group, &shared) {
            warn!(ids = ?group.ids, "skipping invalid collapse group");
            continue;
        }
        // Stage on clones so a rejected canonical cannot partially rename a group.
        let mut staged: Vec<(usize, Candidate)> = group
            .ids
            .iter()
            .filter_map(|&id| candidates.get(id).cloned().map(|c| (id, c)))
            .collect();
        if staged
            .iter_mut()
            .any(|(_, c)| c.set_establishment(group.canonical.clone()).is_err())
        {
            warn!(ids = ?group.ids, "collapse canonical rejected; keeping original names");
            continue;
        }
        for (id, candidate) in staged {
            if let Some(slot) = candidates.get_mut(id) {
                *slot = candidate;
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    let out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{out}…")
    } else {
        out
    }
}

fn strip_code_fences(s: &str) -> String {
    let s = s.trim();
    let Some(body) = s.strip_prefix("```") else {
        return s.to_string();
    };
    let body = body.trim_suffix("```").trim();
    match body.split_once('\n') {
        Some((_lang, rest)) => rest.trim().to_string(),
        None => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CollapseGroup, apply_collapse, extract, extraction_models, models_for, parse_collapse,
        verdict_article,
    };
    use crate::config::{Config, Llm, Persistence, Provider, SecretString};
    use crate::domain::{Article, Candidate};
    use crate::llm::triage::Verdict;
    use crate::models::{ActionType, LlmAction, SanitizedAction};

    fn config() -> Config {
        Config {
            persistence: Persistence::Postgres {
                url: SecretString::new(String::new()),
            },
            llm: Llm {
                provider: Provider::Gemini,
                key: SecretString::new("key".into()),
                model: "model".into(),
                fallbacks: Vec::new(),
            },
        }
    }

    fn article(title: &str) -> Article {
        Article {
            title: title.into(),
            url: format!("https://example.test/{title}"),
            source: None,
            published: None,
            snippet: None,
        }
    }

    #[test]
    fn verdict_index_resolves_against_the_whole_item_list() {
        let items: Vec<Article> = (0..6)
            .map(|index| article(&format!("item-{index}")))
            .collect();
        let last = Verdict {
            source_index: 5,
            reason: None,
        };

        assert_eq!(
            verdict_article(&items, &last).map(|item| item.title.as_str()),
            Some("item-5"),
            "a single triage call uses absolute indices"
        );
        assert!(
            verdict_article(
                &items,
                &Verdict {
                    source_index: 6,
                    reason: None,
                }
            )
            .is_none()
        );
    }

    fn groq() -> Llm {
        Llm {
            provider: Provider::Groq,
            key: SecretString::new("key".into()),
            model: "qwen/qwen3.8-27b".into(),
            fallbacks: vec!["openai/gpt-oss-120b".into()],
        }
    }

    #[test]
    fn groq_models_use_the_groq_namespace() {
        assert_eq!(
            models_for(&groq()),
            ["groq::qwen/qwen3.8-27b", "groq::openai/gpt-oss-120b"]
        );
    }

    #[test]
    fn groq_extraction_leads_with_the_high_output_model() {
        assert_eq!(
            extraction_models(&groq()),
            ["groq::openai/gpt-oss-120b", "groq::qwen/qwen3.8-27b"]
        );
    }

    #[test]
    fn groq_disables_strict_json_schema() {
        assert!(!groq().structured_output());
        assert!(config().llm.structured_output());
    }

    const DETAILS: &str = "licence suspended over cockroach infestation found during inspection";

    fn candidate(
        name: &str,
        city: &str,
        action_type: ActionType,
        date: &str,
        area: Option<&str>,
    ) -> Candidate {
        let action = LlmAction {
            establishment: name.into(),
            city: Some(city.into()),
            area: area.map(str::to_string),
            action_type,
            action_date: Some(date.into()),
            details: Some(DETAILS.into()),
            ..Default::default()
        };
        Candidate::try_new(SanitizedAction::new(action), &article("headline"))
            .expect("valid candidate")
    }

    fn group(ids: &[usize], canonical: &str) -> CollapseGroup {
        CollapseGroup {
            ids: ids.to_vec(),
            canonical: canonical.into(),
        }
    }

    fn parsed(groups: &[CollapseGroup]) -> Vec<(Vec<usize>, String)> {
        groups
            .iter()
            .map(|g| (g.ids.clone(), g.canonical.clone()))
            .collect()
    }

    #[test]
    fn collapse_ignores_bad_and_singleton_groups() {
        let groups = parse_collapse(
            r#"{"groups":[
                {"ids":[0,2],"canonical":" National Sports Club of India "},
                {"ids":[4],"canonical":"singleton"},
                {"ids":[99,100],"canonical":"out of range"},
                {"ids":[-1],"canonical":"negative"}
            ]}"#,
        )
        .expect("parse collapse response");
        assert_eq!(
            parsed(&groups),
            vec![
                (vec![0, 2], "National Sports Club of India".to_string()),
                (vec![99, 100], "out of range".to_string()),
            ],
            "singletons and negative ids are dropped; range is checked at apply time"
        );
        assert!(parse_collapse("not json").is_err());
    }

    #[test]
    fn collapse_deduplicates_ids_and_needs_two_distinct_members() {
        let groups = parse_collapse(
            r#"{"groups":[
                {"ids":[0,0],"canonical":"repeated only"},
                {"ids":[1,1,2],"canonical":"deduped group"}
            ]}"#,
        )
        .expect("parse collapse response");
        assert_eq!(
            parsed(&groups),
            vec![(vec![1, 2], "deduped group".to_string())],
            "[0,0] is not a group of two distinct records"
        );
    }

    #[test]
    fn collapse_renames_every_record_in_a_group() {
        let mut candidates = vec![
            candidate(
                "NSCI",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                Some("Worli"),
            ),
            candidate(
                "National Sports Club",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-30",
                Some("NSCI Worli"),
            ),
        ];

        apply_collapse(
            &mut candidates,
            &[group(&[0, 1], "National Sports Club of India")],
        );

        assert!(
            candidates
                .iter()
                .all(|c| c.establishment() == "National Sports Club of India"),
            "both aliases become the canonical name so the store can dedup them"
        );
    }

    #[test]
    fn collapse_skips_a_group_with_an_out_of_range_id() {
        let mut candidates = vec![
            candidate(
                "NSCI",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                None,
            ),
            candidate(
                "National Sports Club",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                None,
            ),
        ];

        apply_collapse(
            &mut candidates,
            &[group(&[0, 1, 99], "National Sports Club of India")],
        );

        assert!(
            candidates
                .iter()
                .all(|c| c.establishment() != "National Sports Club of India"),
            "one bad id rejects the whole group, not just that id"
        );
    }

    #[test]
    fn collapse_rejects_overlapping_groups() {
        let mut candidates: Vec<Candidate> = (0..3)
            .map(|_| {
                candidate(
                    "NSCI",
                    "Mumbai",
                    ActionType::LicenceSuspension,
                    "2026-11-23",
                    None,
                )
            })
            .collect();

        apply_collapse(
            &mut candidates,
            &[
                group(&[0, 1], "First Canonical"),
                group(&[1, 2], "Second Canonical"),
            ],
        );

        assert!(
            candidates.iter().all(|c| c.establishment() == "NSCI"),
            "an id claimed twice is ambiguous, so neither group is applied"
        );
    }

    #[test]
    fn collapse_requires_same_type_city_and_nearby_dates() {
        let cases: Vec<(&str, Vec<Candidate>)> = vec![
            (
                "mixed action types",
                vec![
                    candidate(
                        "NSCI",
                        "Mumbai",
                        ActionType::LicenceSuspension,
                        "2026-11-23",
                        None,
                    ),
                    candidate("NSCI", "Mumbai", ActionType::Inspection, "2026-11-23", None),
                ],
            ),
            (
                "mixed cities",
                vec![
                    candidate(
                        "NSCI",
                        "Mumbai",
                        ActionType::LicenceSuspension,
                        "2026-11-23",
                        None,
                    ),
                    candidate(
                        "NSCI",
                        "Pune",
                        ActionType::LicenceSuspension,
                        "2026-11-23",
                        None,
                    ),
                ],
            ),
            (
                "dates more than ten days apart",
                vec![
                    candidate(
                        "NSCI",
                        "Mumbai",
                        ActionType::LicenceSuspension,
                        "2026-11-23",
                        None,
                    ),
                    candidate(
                        "NSCI",
                        "Mumbai",
                        ActionType::LicenceSuspension,
                        "2026-12-04",
                        None,
                    ),
                ],
            ),
        ];

        for (label, mut rows) in cases {
            apply_collapse(
                &mut rows,
                &[group(&[0, 1], "National Sports Club of India")],
            );
            assert!(
                rows.iter().all(|c| c.establishment() == "NSCI"),
                "{label} must not be collapsed"
            );
        }
    }

    #[test]
    fn collapse_accepts_exactly_ten_days_between_dates() {
        let mut candidates = vec![
            candidate(
                "NSCI",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                None,
            ),
            candidate(
                "National Sports Club",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-12-03",
                None,
            ),
        ];

        apply_collapse(
            &mut candidates,
            &[group(&[0, 1], "National Sports Club of India")],
        );

        assert!(
            candidates
                .iter()
                .all(|c| c.establishment() == "National Sports Club of India"),
            "ten days is within the window"
        );
    }

    #[test]
    fn collapse_rejects_conflicting_areas() {
        let mut candidates = vec![
            candidate(
                "NSCI",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                Some("Worli"),
            ),
            candidate(
                "NSCI",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                Some("Bandra"),
            ),
        ];

        apply_collapse(
            &mut candidates,
            &[group(&[0, 1], "National Sports Club of India")],
        );

        assert!(
            candidates.iter().all(|c| c.establishment() == "NSCI"),
            "unrelated localities are different events"
        );
    }

    #[test]
    fn collapse_rejects_an_invalid_canonical() {
        let mut candidates = vec![
            candidate(
                "NSCI",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                None,
            ),
            candidate(
                "National Sports Club",
                "Mumbai",
                ActionType::LicenceSuspension,
                "2026-11-23",
                None,
            ),
        ];

        apply_collapse(&mut candidates, &[group(&[0, 1], "'Quoted Outlet")]);

        assert!(
            candidates
                .iter()
                .all(|c| c.establishment() != "'Quoted Outlet"),
            "the canonical must pass Candidate::set_establishment"
        );
    }

    #[tokio::test]
    async fn empty_items_skip_llm() {
        let extraction = extract(&config(), &[]).await.unwrap();
        assert!(extraction.candidates.is_empty());
        assert_eq!(
            extraction.metrics.triage_calls + extraction.metrics.extraction_calls,
            0
        );
    }
}
