use std::collections::HashSet;

use anyhow::{Result as AnyResult, anyhow};
use tracing::{debug, info, instrument, warn};

use crate::config::{Config, DEFAULT_OPENROUTER_MODEL};
use crate::domain::{Article, Candidate};
use crate::report::RunMetrics;

pub const SYSTEM_PROMPT: &str = include_str!("../prompts/system.txt");

/// Large batches respect the ~5 requests/min free tier.
const BATCH_SIZE: usize = 20;
// ponytail: concurrent batches hit 429s together, so they run sequentially.

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

/// A failed batch passes its items through and is counted as degradation.
#[instrument(skip_all, fields(total = items.len(), model = %model))]
async fn triage<'a>(
    api_key: &str,
    model: &str,
    items: &'a [Article],
) -> AnyResult<(Vec<&'a Article>, RunMetrics)> {
    if items.is_empty() {
        return Ok((Vec::new(), RunMetrics::default()));
    }
    let client = genai::client(api_key, None)?;
    let mut metrics = RunMetrics::default();
    let mut kept = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();

    for (batch_no, chunk) in items.chunks(BATCH_SIZE).enumerate() {
        let (result, calls) = genai::triage_batch(&client, model, chunk).await;
        metrics.triage_calls += calls;
        match result {
            Ok(verdicts) => {
                for verdict in verdicts {
                    let Some(item) = verdict_article(chunk, &verdict) else {
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
            }
            Err(error) => {
                metrics.triage_failures += 1;
                warn!(
                    batch = batch_no,
                    error = %format!("{error:#}"),
                    "triage failed; passing all items through"
                );
                for item in chunk {
                    if seen.insert(item.url.as_str()) {
                        kept.push(item);
                    }
                }
            }
        }
    }
    Ok((kept, metrics))
}

#[instrument(skip_all, fields(total = items.len(), model = %config.gemini_model))]
pub(crate) async fn extract(config: &Config, items: &[Article]) -> AnyResult<Extraction> {
    let (articles, mut metrics) =
        triage(config.gemini_key.expose(), &config.gemini_model, items).await?;
    info!(relevant = articles.len(), total = items.len(), "llm triage");
    if articles.is_empty() {
        let degraded = metrics.triage_failures > 0;
        return Ok(Extraction {
            candidates: Vec::new(),
            metrics,
            degraded,
        });
    }

    let openrouter = config.openrouter.as_ref();
    let openrouter_key = openrouter.map(|options| options.key.expose());
    let openrouter_model = openrouter
        .map(|options| options.model.as_str())
        .unwrap_or(DEFAULT_OPENROUTER_MODEL);
    let openrouter_fallbacks: &[String] = match openrouter {
        Some(options) => options.fallbacks.as_slice(),
        None => &[],
    };
    let client = genai::client(config.gemini_key.expose(), openrouter_key)?;
    let opts = ExtractOptions {
        model: &config.gemini_model,
        openrouter_key,
        openrouter_model,
        openrouter_fallbacks,
    };

    let mut candidates = Vec::new();
    let mut failed = 0;
    for (batch_no, chunk) in articles.chunks(BATCH_SIZE).enumerate() {
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
    if failed > 0 && failed == articles.len().div_ceil(BATCH_SIZE) {
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
    let degraded = metrics.triage_failures > 0 || metrics.extraction_failures > 0;
    Ok(Extraction {
        candidates,
        metrics,
        degraded,
    })
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
    use super::{BATCH_SIZE, extract, verdict_article};
    use crate::config::{Config, Persistence, SecretString};
    use crate::domain::Article;
    use crate::llm::triage::Verdict;

    fn config() -> Config {
        Config {
            persistence: Persistence::Postgres {
                url: SecretString::new(String::new()),
            },
            gemini_key: SecretString::new("key".into()),
            gemini_model: "model".into(),
            openrouter: None,
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
    fn each_chunk_maps_local_index_zero_to_its_own_article() {
        let items: Vec<Article> = (0..2 * BATCH_SIZE)
            .map(|index| article(&format!("item-{index}")))
            .collect();
        let verdict = Verdict {
            source_index: 0,
            reason: None,
        };
        let chunks: Vec<&[Article]> = items.chunks(BATCH_SIZE).collect();

        assert_eq!(chunks.len(), 2, "two distinct chunks expected");
        assert_eq!(
            verdict_article(chunks[0], &verdict).map(|item| item.title.as_str()),
            Some("item-0"),
            "first chunk's local index 0 is its own first item"
        );
        let expected = format!("item-{BATCH_SIZE}");
        assert_eq!(
            verdict_article(chunks[1], &verdict).map(|item| item.title.as_str()),
            Some(expected.as_str()),
            "second chunk's local index 0 is its own first item"
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
