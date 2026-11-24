use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Result as AnyResult, anyhow};
use tokio::time::sleep;
use tracing::{debug, info, instrument, warn};

use crate::config::{Config, Llm};
use crate::domain::{Article, Candidate};
use crate::report::RunMetrics;

pub const SYSTEM_PROMPT: &str = include_str!("../prompts/system.txt");

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
    match llm {
        Llm::Gemini { model, .. } => vec![model.clone()],
        Llm::OpenRouter {
            model, fallbacks, ..
        } => prefixed(model, fallbacks, "open_router"),
        Llm::Groq {
            model, fallbacks, ..
        } => prefixed(model, fallbacks, "groq"),
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
    match llm {
        Llm::Groq {
            model, fallbacks, ..
        } => {
            let mut names = fallbacks.to_vec();
            if !names.contains(model) {
                names.push(model.clone());
            }
            names
                .into_iter()
                .map(|name| format!("groq::{name}"))
                .collect()
        }
        _ => models_for(llm),
    }
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

#[instrument(skip_all, fields(total = items.len(), model = %config.llm.model()))]
pub(crate) async fn extract(config: &Config, items: &[Article]) -> AnyResult<Extraction> {
    let structured = config.llm.structured_output();
    let client = genai::client(&config.llm)?;
    let (articles, mut metrics) =
        triage(&client, &models_for(&config.llm), items, structured).await?;
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
    use super::{extract, extraction_models, models_for, verdict_article};
    use crate::config::{Config, Llm, Persistence, SecretString};
    use crate::domain::Article;
    use crate::llm::triage::Verdict;

    fn config() -> Config {
        Config {
            persistence: Persistence::Postgres {
                url: SecretString::new(String::new()),
            },
            llm: Llm::Gemini {
                key: SecretString::new("key".into()),
                model: "model".into(),
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
        Llm::Groq {
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
