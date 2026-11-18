use std::collections::HashSet;

use anyhow::{Result, anyhow};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::domain::{Article, Candidate};
use crate::report::RunMetrics;

pub const SYSTEM_PROMPT: &str = include_str!("../prompts/system.txt");

/// Large batches respect the ~5 requests/min free tier; crowding is handled
/// downstream by the prompt's 16k-token output ceiling.
pub(crate) const BATCH_SIZE: usize = 20;
// ponytail: concurrent batches hit 429s together, so they run sequentially.

/// Kept as a compatibility alias; the canonical default now lives in `config`.
pub const DEFAULT_GEMINI_MODEL: &str = crate::config::DEFAULT_GEMINI_MODEL;

mod genai;
mod quality;
mod triage;

/// Strict relevance pass. Returns the absolute indices the extraction stage
/// should process. A failed batch passes its items through and is counted, so
/// a degraded run never silently drops the whole chunk.
pub(crate) async fn triage(
    api_key: &str,
    model: &str,
    items: &[Article],
    metrics: &mut RunMetrics,
) -> Result<Vec<usize>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let client = genai::client(api_key, None)?;
    let mut kept = Vec::new();
    let mut seen = HashSet::new();

    for (batch_no, chunk) in items.chunks(BATCH_SIZE).enumerate() {
        let base = batch_no * BATCH_SIZE;
        match genai::triage_batch(&client, model, chunk, metrics).await {
            Ok(verdicts) => {
                for verdict in verdicts {
                    if verdict.source_index >= chunk.len() {
                        warn!(
                            index = verdict.source_index,
                            "triage verdict index out of range"
                        );
                        continue;
                    }
                    let index = base + verdict.source_index;
                    if seen.insert(index) {
                        debug!(index, reason = ?verdict.reason, "triage kept item");
                        kept.push(index);
                    }
                }
            }
            Err(error) => {
                metrics.triage_failures += 1;
                warn!(
                    batch = batch_no,
                    error = %format!("{error:#}"),
                    "triage failed; passing all items through"
                );
                for offset in 0..chunk.len() {
                    let index = base + offset;
                    if seen.insert(index) {
                        kept.push(index);
                    }
                }
            }
        }
    }
    Ok(kept)
}

/// Extract and validate articles into candidates ready for persistence. All
/// configuration comes from `config`; no environment reads happen here.
pub async fn extract(
    config: &Config,
    items: &[Article],
    metrics: &mut RunMetrics,
) -> Result<Vec<Candidate>> {
    let indices = triage(&config.gemini_key, &config.gemini_model, items, metrics).await?;
    info!(relevant = indices.len(), total = items.len(), "llm triage");
    if indices.is_empty() {
        return Ok(Vec::new());
    }

    let openrouter = config.openrouter.as_ref();
    let openrouter_key = openrouter.map(|options| options.key.as_str());
    let openrouter_model = openrouter
        .map(|options| options.model.as_str())
        .unwrap_or(crate::config::DEFAULT_OPENROUTER_MODEL);
    let openrouter_fallbacks: &[String] = match openrouter {
        Some(options) => options.fallbacks.as_slice(),
        None => &[],
    };
    let client = genai::client(&config.gemini_key, openrouter_key)?;

    let mut jobs: Vec<triage::Job<'_>> = indices
        .into_iter()
        .filter_map(|index| {
            items
                .get(index)
                .map(|item| triage::Job { orig: index, item })
        })
        .collect();
    jobs.sort_by_key(|job| job.orig);

    let mut candidates = Vec::new();
    let mut failed = 0;
    for (batch_no, chunk) in jobs.chunks(BATCH_SIZE).enumerate() {
        match genai::extract_batch(
            &client,
            &config.gemini_model,
            chunk,
            openrouter_key,
            openrouter_model,
            openrouter_fallbacks,
            metrics,
        )
        .await
        {
            Ok(batch) => {
                for (index, action) in batch {
                    // Bound untrusted LLM strings before they reach the DB or
                    // the dedup prompt: no new logic, reuse the existing clamps.
                    let action = genai::sanitize_action(action);
                    let Some(article) = items.get(index) else {
                        warn!(index, "extraction result index out of range");
                        continue;
                    };
                    match Candidate::try_new(action, article)
                        .and_then(|candidate| quality::check(&candidate).map(|()| candidate))
                    {
                        Ok(candidate) => candidates.push(candidate),
                        Err(rejection) => {
                            metrics.rejected += 1;
                            warn!(reason = %rejection, index, "dropping rejected LLM record");
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
    if failed > 0 && failed == jobs.len().div_ceil(BATCH_SIZE) {
        return Err(anyhow!("all llm batches failed"));
    }

    info!(
        candidates = candidates.len(),
        failed_batches = failed,
        "llm extraction done"
    );
    for candidate in &candidates {
        debug!(
            establishment = %candidate.establishment,
            city = candidate.city.as_str(),
            action_type = %candidate.action_type,
            action_date = %candidate.action_date,
            "record"
        );
    }
    Ok(candidates)
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
    use super::extract;
    use crate::config::Config;
    use crate::report::RunMetrics;

    fn config() -> Config {
        Config {
            database_url: String::new(),
            gemini_key: "key".into(),
            gemini_model: "model".into(),
            openrouter: None,
        }
    }

    #[tokio::test]
    async fn empty_items_skip_llm() {
        let mut metrics = RunMetrics::default();
        let candidates = extract(&config(), &[], &mut metrics).await.unwrap();
        assert!(candidates.is_empty());
        assert_eq!(metrics.total_calls(), 0);
    }
}
