use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::models::LlmAction;

use super::gemini::{parse_llm_text, post, retry_after, retry_delay};
use super::system_prompt;
use super::triage::Job;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
/// openrouter/free auto-routes to whatever free model is currently healthy
/// and conforms with the account data policies (including zero data retention).
/// Fallbacks include specific free models and user-supplied paid models if set.
pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
pub(crate) const OPENROUTER_FALLBACKS: &[&str] = &["inclusionai/ling-3.2-flash:free"];
const OPENROUTER_MAX_ATTEMPTS: usize = 2;

/// Single OpenRouter escalation path: fall back, then readdress positions
/// to the caller's item indices; orphan indices the LLM invented are dropped.
pub(crate) async fn escalate(
    openrouter_key: Option<&str>,
    chunk: &[Job<'_>],
    calls: &mut usize,
    delivery: bool,
    openrouter_model: &str,
    openrouter_fallbacks: &[String],
) -> Result<Vec<LlmAction>> {
    Ok(remap(
        fallback(
            openrouter_key,
            chunk,
            calls,
            delivery,
            openrouter_model,
            openrouter_fallbacks,
        )
        .await?,
        chunk,
    ))
}

pub(crate) fn remap(actions: Vec<LlmAction>, chunk: &[Job<'_>]) -> Vec<LlmAction> {
    actions
        .into_iter()
        .filter_map(|mut a| {
            let job = chunk.get(a.source_index)?;
            a.source_index = job.orig;
            Some(a)
        })
        .collect()
}

async fn fallback(
    openrouter_key: Option<&str>,
    chunk: &[Job<'_>],
    calls: &mut usize,
    delivery: bool,
    openrouter_model: &str,
    openrouter_fallbacks: &[String],
) -> Result<Vec<LlmAction>> {
    let Some(key) = openrouter_key else {
        return Err(anyhow::anyhow!(
            "gemini API failed and no OPENROUTER_API_KEY fallback configured"
        ));
    };

    let mut candidate_models = vec![openrouter_model.to_string()];
    for m in openrouter_fallbacks {
        if m != openrouter_model {
            candidate_models.push(m.clone());
        }
    }

    let mut last_err = None;
    for (idx, m) in candidate_models.iter().enumerate() {
        match openrouter_with_model(key, m, chunk, calls, delivery).await {
            Ok(actions) => return Ok(actions),
            Err(e) => {
                eprintln!("openrouter model {m} failed: {e:#}");
                let err_str = e.to_string();
                if err_str.contains("http 401") {
                    return Err(e);
                }
                if idx + 1 < candidate_models.len() {
                    eprintln!(
                        "trying next openrouter fallback: {}",
                        candidate_models[idx + 1]
                    );
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all openrouter models failed")))
}

/// OpenRouter 404s/4xx on guardrails never succeed on retry — fail fast
/// instead of burning backoffs per batch. Returns the error to abort with.
fn openrouter_fatal(status: u16, text: &str) -> Option<anyhow::Error> {
    // Free endpoints do not offer zero-data-retention, so an account with ZDR
    // enforced gets "0 endpoints available" for every model. Keep the log line
    // short and actionable instead of dumping the routing metadata per batch.
    if status == 404 && text.contains("zdr-violation") {
        return Some(anyhow::anyhow!(
            "openrouter: ZDR-only routing excludes every free endpoint; enabling \
             'Allow free endpoints that train on request data' (and 'publish prompts' \
             if needed) at https://openrouter.ai/settings/privacy fixes it, or fund a \
             ZDR-capable model"
        ));
    }
    match status {
        400 | 401 | 403 | 404 => Some(anyhow::anyhow!(
            "openrouter http {status} (non-retryable): {text}"
        )),
        402 if text.contains("Insufficient credits")
            || text.contains("never purchased credits") =>
        {
            Some(anyhow::anyhow!(
                "openrouter 402 insufficient credits: {text}"
            ))
        }
        _ => None,
    }
}

/// One OpenRouter round-trip. Transport failures yield None (the caller backs
/// off and retries); anything the server answered yields status/body/wait.
async fn openrouter_round(
    api_key: &str,
    payload: &Value,
    calls: &mut usize,
) -> Option<(reqwest::StatusCode, String, Option<u64>)> {
    let resp = match post(OPENROUTER_URL, Some(api_key), payload, calls).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("openrouter request error: {e}");
            return None;
        }
    };
    let wait = retry_after(&resp);
    let status = resp.status();
    let text = resp
        .text()
        .await
        .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
    Some((status, text, wait))
}

/// Parse a 200 OpenRouter body into actions.
fn openrouter_success(text: &str, model: &str) -> Result<Vec<LlmAction>> {
    let body: Value = serde_json::from_str(text).context("openrouter json")?;
    let responded = body["model"].as_str().unwrap_or(model);
    let citations = body["citations"].as_array().map_or(0, Vec::len);
    eprintln!("openrouter ok: model={responded}, web_searches={citations}");
    let text = body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no text in openrouter response"))?;
    parse_llm_text(text)
}

/// Handle an OpenRouter 402 by halving the token budget. Non-402 statuses
/// yield false so the caller falls through to the normal backoff. Bails when
/// the budget is already at the floor.
fn shrink_budget_on_402(status: u16, text: &str, payload: &mut Value) -> Result<bool> {
    if status != 402 {
        return Ok(false);
    }
    let cur = payload["max_tokens"].as_u64().unwrap_or(32768);
    if cur <= 8192 {
        return Err(anyhow::anyhow!("openrouter low balance (402): {text}"));
    }
    let lower = cur / 2;
    payload["max_tokens"] = json!(lower);
    eprintln!("openrouter 402 (low balance); retrying with max_tokens={lower}");
    Ok(true)
}

async fn openrouter_with_model(
    api_key: &str,
    model: &str,
    chunk: &[Job<'_>],
    calls: &mut usize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    eprintln!("openrouter attempt: model={model}, items={}", chunk.len());
    let mut payload = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt(delivery)},
            {"role": "user", "content": serde_json::to_string(
                &json!({ "items": chunk.iter().map(|j| j.item).collect::<Vec<_>>() })
            ).context("serialize news batch")?}
        ],
        "response_format": {"type": "json_object"},
        "temperature": 0.0,
        "max_tokens": 16384
    });
    for attempt in 0..OPENROUTER_MAX_ATTEMPTS {
        let Some((status, text, wait)) = openrouter_round(api_key, &payload, calls).await else {
            tokio::time::sleep(retry_delay(attempt, None)).await;
            continue;
        };
        if status.is_success() {
            return openrouter_success(&text, model);
        }
        eprintln!("openrouter http {status}, attempt {attempt}; {text}");
        if let Some(e) = openrouter_fatal(status.as_u16(), &text) {
            return Err(e);
        }
        if shrink_budget_on_402(status.as_u16(), &text, &mut payload)? {
            continue;
        }
        tokio::time::sleep(retry_delay(attempt, wait)).await;
    }
    Err(anyhow::anyhow!(
        "openrouter {model} failed after {OPENROUTER_MAX_ATTEMPTS} attempts"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fatal_statuses_abort_without_retry() {
        assert!(openrouter_fatal(404, "no such model").is_some());
        assert!(openrouter_fatal(401, "bad key").is_some());
        assert!(openrouter_fatal(429, "slow down").is_none());
        assert!(openrouter_fatal(500, "boom").is_none());
    }

    #[test]
    fn zdr_404_names_the_account_setting() {
        let e = openrouter_fatal(
            404,
            r#"{"ineligibility_reasons":[{"reason":"zdr-violation-by-account"}]}"#,
        )
        .expect("zdr 404 is fatal")
        .to_string();
        assert!(e.contains("settings/privacy"), "{e}");
    }

    #[test]
    fn budget_halves_on_402() {
        let mut payload = json!({"max_tokens": 32768});
        assert!(shrink_budget_on_402(402, "low", &mut payload).unwrap());
        assert_eq!(payload["max_tokens"], json!(16384));
        assert!(!shrink_budget_on_402(429, "slow", &mut payload).unwrap());
    }

    #[test]
    fn budget_floor_errors_instead_of_halving() {
        let mut floor = json!({"max_tokens": 8192});
        assert!(shrink_budget_on_402(402, "low", &mut floor).is_err());
    }

    #[test]
    fn success_parses_first_choice_content() {
        let body = json!({
            "model": "m",
            "choices": [{"message": {"content": "[{\"establishment\":\"X\",\"actionType\":\"inspection\",\"source_index\":0}]"}}]
        });
        let actions = openrouter_success(&body.to_string(), "m").unwrap();
        assert_eq!(actions.len(), 1, "one action parsed");
    }
}
