use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use tracing::{debug, info, warn};

use crate::db::{ActionInsert, RecentEvent};
use crate::models::{ActionType, LlmAction, NewsItem};

use triage::Verdict;

pub const SYSTEM_PROMPT: &str = include_str!("../prompts/system.txt");

/// Large batches respect the ~5 requests/min free tier; crowding is handled
/// downstream by the prompt's 16k-token output ceiling.
pub(crate) const BATCH_SIZE: usize = 20;
// ponytail: concurrent batches hit 429s together, so they run sequentially.

/// Other Gemini aliases return empty 404s with this key — they moved to the
/// Interactions API — and would burn a call per batch before failing over.
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";

mod genai;
mod quality;
mod triage;

/// Strict relevance pass. On a model/parse failure every item passes through
/// rather than dropping the run: extraction and the quality gate still decide.
pub(crate) async fn triage(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
) -> Result<(Vec<Verdict>, usize)> {
    if items.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let client = genai::client(api_key, None)?;
    let mut calls = 0;
    let mut verdicts = Vec::new();
    let mut seen = HashSet::new();

    for (batch_no, chunk) in items.chunks(BATCH_SIZE).enumerate() {
        let base = batch_no * BATCH_SIZE;
        let (batch, used) = genai::triage_batch(&client, model, chunk).await;
        calls += used;
        match batch {
            Ok(batch) => {
                for mut verdict in batch {
                    if verdict.source_index >= chunk.len() {
                        warn!(
                            index = verdict.source_index,
                            "triage verdict index out of range"
                        );
                        continue;
                    }
                    verdict.source_index += base;
                    if seen.insert(verdict.source_index) {
                        debug!(
                            index = verdict.source_index,
                            reason = verdict.reason.as_deref().unwrap_or("-"),
                            "triage kept item"
                        );
                        verdicts.push(verdict);
                    }
                }
            }
            Err(error) => {
                warn!(error = %format!("{error:#}"), "triage failed; passing all items through");
                for (i, _) in chunk.iter().enumerate() {
                    let index = base + i;
                    if seen.insert(index) {
                        verdicts.push(Verdict {
                            source_index: index,
                            // Placeholder: only used if extraction also fails,
                            // and the minimal record has no name or city.
                            action_type: ActionType::Inspection,
                            city: None,
                            establishment: None,
                            reason: None,
                        });
                    }
                }
            }
        }
    }
    Ok((verdicts, calls))
}

pub async fn extract(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
) -> Result<(Vec<LlmAction>, usize)> {
    let (verdicts, triage_calls) = triage(api_key, model, items).await?;
    let verdicts: HashMap<usize, Verdict> =
        verdicts.into_iter().map(|v| (v.source_index, v)).collect();
    let mut jobs: Vec<triage::Job<'_>> = verdicts
        .keys()
        .filter_map(|&index| {
            items
                .get(index)
                .map(|item| triage::Job { orig: index, item })
        })
        .collect();
    jobs.sort_by_key(|job| job.orig);
    info!(relevant = jobs.len(), total = items.len(), "llm triage");
    if jobs.is_empty() {
        return Ok((Vec::new(), triage_calls));
    }

    // ponytail: resolve config once at the boundary; deeper fns stay pure of env reads.
    let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();
    let openrouter_model = std::env::var("OPENROUTER_MODEL")
        .unwrap_or_else(|_| genai::DEFAULT_OPENROUTER_MODEL.to_string());
    let openrouter_fallbacks: Vec<String> = std::env::var("OPENROUTER_FALLBACKS")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|_| {
            genai::OPENROUTER_FALLBACKS
                .iter()
                .map(|m| m.to_string())
                .collect()
        });

    let client = genai::client(api_key, openrouter_key.as_deref())?;
    let mut calls = triage_calls;
    let mut actions = Vec::new();
    let mut failed = 0;
    for (batch_no, chunk) in jobs.chunks(BATCH_SIZE).enumerate() {
        let (batch, used) = genai::extract_batch(
            &client,
            model,
            chunk,
            openrouter_key.as_deref(),
            &openrouter_model,
            &openrouter_fallbacks,
        )
        .await;
        calls += used;
        match batch {
            // Gate LLM output only: salvage rows come from triage verdicts,
            // not from this model's free-form generation.
            Ok(batch) => actions.extend(batch.into_iter().filter(|a| match quality::check(a) {
                Ok(()) => true,
                Err(reason) => {
                    warn!(
                        reason,
                        establishment = %truncate(&a.establishment, 80),
                        "dropping low-quality LLM record"
                    );
                    false
                }
            })),
            Err(e) => actions.extend(salvaged(batch_no, &e, chunk, &verdicts, &mut failed)),
        }
    }
    let batch_count = jobs.len().div_ceil(BATCH_SIZE);
    if failed == batch_count {
        return Err(anyhow!("all {batch_count} llm batches failed"));
    }

    let mut seen = HashSet::new();
    actions = actions
        .into_iter()
        .filter(|a| {
            seen.insert((
                a.source_index,
                a.establishment.to_lowercase(),
                a.action_type,
            ))
        })
        .map(genai::sanitize_action)
        .collect();

    info!(
        calls,
        failed_batches = failed,
        actions = actions.len(),
        "llm extraction done"
    );
    for a in &actions {
        debug!(
            establishment = %a.establishment,
            city = a.city.as_deref().unwrap_or("-"),
            action_type = %a.action_type,
            action_date = a.action_date.as_deref().unwrap_or("-"),
            violations = %a.violations.join("; "),
            details = a.details.as_deref().unwrap_or("-"),
            "record"
        );
    }

    Ok((actions, calls))
}

/// Fallback for a failed extraction batch, built only from triage verdicts.
/// An empty vec means the batch is unrecoverable; the caller counts it.
fn salvaged(
    batch_no: usize,
    err: &anyhow::Error,
    chunk: &[triage::Job<'_>],
    verdicts: &HashMap<usize, Verdict>,
    failed: &mut usize,
) -> Vec<LlmAction> {
    let kept: Vec<LlmAction> = chunk
        .iter()
        .filter_map(|job| {
            verdicts
                .get(&job.orig)
                .map(|verdict| triage::minimal_from_verdict(verdict, job.item))
        })
        .collect();
    let n = chunk.len();
    if kept.is_empty() {
        *failed += 1;
        warn!(batch = batch_no, items = n, error = %format!("{err:#}"), "llm batch failed");
    } else {
        warn!(
            batch = batch_no,
            items = n,
            kept = kept.len(),
            error = %format!("{err:#}"),
            "llm batch failed; triage fallback kept records"
        );
    }
    kept
}

/// Ask Gemini which rows describe an event already covered by another row or
/// a recent DB row. Returns indices to drop; on failure returns none so the
/// name heuristics in `upsert_actions` stay the safety net.
pub async fn collapse_dupes(
    api_key: &str,
    model: &str,
    rows: &[ActionInsert],
    recent: &[RecentEvent],
) -> (Vec<usize>, usize) {
    // ponytail: collapse has no OpenRouter fallback, so its own client is fine.
    let (outcome, calls) = match genai::client(api_key, None) {
        Ok(client) => genai::collapse_once(&client, model, rows, recent).await,
        Err(error) => (Err(error), 0),
    };
    match outcome {
        Ok(drops) => (drops, calls),
        Err(e) => {
            warn!(error = %format!("{e:#}"), "dupe-collapse llm failed; using name heuristics only");
            (Vec::new(), calls)
        }
    }
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    let out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{out}…")
    } else {
        out
    }
}

pub(crate) fn strip_code_fences(s: &str) -> String {
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
    use strum::IntoEnumIterator;

    use super::{SYSTEM_PROMPT, extract};
    use crate::models::{ActionType, OutletType};
    use crate::scrape;

    #[test]
    fn prompt_names_every_code_and_covered_city() {
        let prompt = SYSTEM_PROMPT.to_lowercase();
        for code in ActionType::iter().map(|t| t.to_string()) {
            assert!(prompt.contains(&code), "prompt missing action code {code}");
        }
        for code in OutletType::iter().map(|t| t.to_string()) {
            assert!(prompt.contains(&code), "prompt missing outlet code {code}");
        }
        for city in scrape::IN_SCOPE_CITIES {
            assert!(prompt.contains(city), "prompt missing covered city {city}");
        }
    }

    #[tokio::test]
    async fn empty_items_skip_llm() {
        let (actions, calls) = extract("key", "model", &[]).await.unwrap();
        assert!(actions.is_empty());
        assert_eq!(calls, 0);
    }
}
