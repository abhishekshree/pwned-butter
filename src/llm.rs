use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::models::{LlmAction, NewsItem};

pub const SYSTEM_PROMPT: &str = include_str!("prompts/system.txt");

const DELIVERY_MODE: &str = include_str!("prompts/delivery.txt");

fn system_prompt(delivery: bool) -> String {
    if delivery {
        format!("{SYSTEM_PROMPT}{DELIVERY_MODE}")
    } else {
        SYSTEM_PROMPT.to_string()
    }
}

const MAX_ATTEMPTS: usize = 6;

fn retry_delay(attempt: usize, retry_after: Option<u64>) -> Duration {
    let secs = retry_after
        .map(|s| s.min(120))
        .unwrap_or_else(|| (15u64 * (1u64 << attempt.min(4))).min(60));
    Duration::from_secs(secs)
}
const BATCH_SIZE: usize = 20;
const MAX_CONCURRENT: usize = 2;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_MAX_ATTEMPTS: usize = 3;
const DEFAULT_OPENROUTER_MODEL: &str = "nvidia/nemotron-3-ultra-550b-a55b:free";

pub async fn extract(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
    delivery: bool,
) -> Result<(Vec<LlmAction>, usize)> {
    if items.is_empty() {
        return Ok((Vec::new(), 0));
    }

    let api_key = api_key.to_owned();
    let model = model.to_owned();
    let requests = Arc::new(AtomicUsize::new(0));
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut tasks = Vec::new();

    for (offset, chunk) in items.chunks(BATCH_SIZE).enumerate() {
        let sem = Arc::clone(&sem);
        let requests = Arc::clone(&requests);
        let api_key = api_key.clone();
        let model = model.clone();
        let chunk = chunk.to_vec();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let batch = extract_batch(&api_key, &model, &chunk, &requests, delivery).await;
            (offset, batch)
        }));
    }

    let batch_count = tasks.len();
    let mut actions: Vec<LlmAction> = Vec::new();
    let mut failed = 0usize;
    for task in tasks {
        let (offset, result) = task.await.context("llm batch task join")?;
        match result {
            Ok(mut batch) => {
                apply_offset(&mut batch, offset * BATCH_SIZE);
                actions.append(&mut batch);
            }
            Err(e) => {
                failed += 1;
                eprintln!("llm batch {offset} failed: {e}");
            }
        }
    }

    let calls = requests.load(Ordering::Relaxed);
    if failed == batch_count {
        return Err(anyhow!("all {batch_count} llm batches failed"));
    }

    let mut seen = std::collections::HashSet::new();
    actions = actions
        .into_iter()
        .filter(|a| !a.establishment.trim().is_empty())
        .filter(|a| {
            let key = format!(
                "{}|{}|{}",
                a.source_index,
                a.establishment.to_lowercase(),
                a.action_type
            );
            seen.insert(key)
        })
        .map(sanitize_action)
        .collect();

    eprintln!(
        "llm: {calls} calls, {failed} failed batches, {} extracted actions",
        actions.len()
    );
    for a in &actions {
        eprintln!(
            "  record: {} | {} | {} | {} | violations={} | details={}",
            a.establishment,
            a.city.as_deref().unwrap_or("-"),
            a.action_type,
            a.action_date.as_deref().unwrap_or("-"),
            if a.violations.is_empty() {
                "-".to_string()
            } else {
                a.violations.join("; ")
            },
            a.details.as_deref().unwrap_or("-"),
        );
    }

    Ok((actions, calls))
}

fn apply_offset(actions: &mut [LlmAction], offset: usize) {
    for a in actions.iter_mut() {
        a.source_index += offset;
    }
}

async fn extract_batch(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
    requests: &AtomicUsize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    let payload = json!({
        "system_instruction": {"parts": [{"text": system_prompt(delivery)}]},
        "contents": [{"parts": [{"text": serde_json::to_string(&json!({ "items": items })).context("serialize news batch")?}]}],
        "generationConfig": {
            "temperature": 0.0,
            "responseMimeType": "application/json",
            "maxOutputTokens": 8192
        }
    });
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
    );
    let client = crate::http_client();
    let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();

    for attempt in 0..MAX_ATTEMPTS {
        requests.fetch_add(1, Ordering::Relaxed);
        let resp = match client.post(&url).json(&payload).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("gemini request error: {e}");
                tokio::time::sleep(retry_delay(attempt, None)).await;
                continue;
            }
        };
        if resp.status().is_success() {
            let body: Value = resp.json().await.context("gemini json")?;
            return parse_response(&body);
        }
        let status = resp.status();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let text = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
        if status.as_u16() == 429 || status.is_server_error() {
            eprintln!("gemini http {status}, attempt {attempt}; {text}");
            if is_quota_exhausted(&text) {
                return extract_openrouter(openrouter_key.as_deref(), items, requests, delivery)
                    .await;
            }
            tokio::time::sleep(retry_delay(attempt, retry_after)).await;
            continue;
        }
        return Err(anyhow!("gemini http {status}: {text}"));
    }
    eprintln!("gemini API failed after {MAX_ATTEMPTS} attempts; falling back to openrouter");
    extract_openrouter(openrouter_key.as_deref(), items, requests, delivery).await
}

fn is_quota_exhausted(text: &str) -> bool {
    text.contains("Quota exceeded") || text.contains("RESOURCE_EXHAUSTED")
}

async fn extract_openrouter(
    api_key: Option<&str>,
    items: &[NewsItem],
    requests: &AtomicUsize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    let Some(api_key) = api_key else {
        return Err(anyhow!(
            "gemini API failed and no OPENROUTER_API_KEY fallback configured"
        ));
    };
    openrouter_with_model(api_key, DEFAULT_OPENROUTER_MODEL, items, requests, delivery).await
}

async fn openrouter_with_model(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
    requests: &AtomicUsize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    eprintln!("openrouter attempt: model={model}, items={}", items.len());
    let mut payload = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt(delivery)},
            {"role": "user", "content": serde_json::to_string(&json!({ "items": items })).context("serialize news batch")?}
        ],
        "tools": [{"type": "openrouter:web_search"}],
        "temperature": 0.0,
        "max_tokens": 32768
    });
    let client = crate::http_client();

    for attempt in 0..OPENROUTER_MAX_ATTEMPTS {
        requests.fetch_add(1, Ordering::Relaxed);
        let resp = match client
            .post(OPENROUTER_URL)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("openrouter request error: {e}");
                tokio::time::sleep(retry_delay(attempt, None)).await;
                continue;
            }
        };
        if resp.status().is_success() {
            let body: Value = resp.json().await.context("openrouter json")?;
            let responded = body["model"].as_str().unwrap_or(model);
            let citations = body["citations"].as_array().map_or(0, Vec::len);
            eprintln!("openrouter ok: model={responded}, web_searches={citations}");
            let text = body["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| anyhow!("no text in openrouter response"))?;
            return parse_llm_text(text);
        }
        let status = resp.status();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let text = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
        eprintln!("openrouter http {status}, attempt {attempt}; {text}");
        if status.as_u16() == 402 {
            if text.contains("Insufficient credits") || text.contains("never purchased credits") {
                return Err(anyhow!("openrouter 402 insufficient credits: {text}"));
            }
            let cur = payload["max_tokens"].as_u64().unwrap_or(32768);
            if cur > 8192 {
                let lower = cur / 2;
                payload["max_tokens"] = json!(lower);
                eprintln!("openrouter 402 (low balance); retrying with max_tokens={lower}");
                continue;
            }
            return Err(anyhow!("openrouter low balance (402): {text}"));
        }
        tokio::time::sleep(retry_delay(attempt, retry_after)).await;
    }
    Err(anyhow!("openrouter {model} failed after {OPENROUTER_MAX_ATTEMPTS} attempts"))
}

fn parse_response(body: &Value) -> Result<Vec<LlmAction>> {
    let text: String = body
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .ok_or_else(|| anyhow!("no text in gemini response"))?;

    parse_llm_text(&text)
}

fn parse_llm_text(text: &str) -> Result<Vec<LlmAction>> {
    let text = strip_code_fences(text);
    let parsed: Value = serde_json::from_str(&text).map_err(|e| {
        anyhow!(
            "LLM returned invalid JSON: {e}; body: {}",
            truncate(text.as_str(), 300)
        )
    })?;

    let raw = match parsed {
        Value::Array(arr) => arr,
        Value::Object(map) => map
            .get("actions")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or_else(|| anyhow!("expected JSON array or object with \"actions\" array"))?,
        _ => return Err(anyhow!("unexpected LLM response shape")),
    };

    let mut actions = Vec::new();
    for v in raw {
        match serde_json::from_value::<LlmAction>(v.clone()) {
            Ok(a) => actions.push(a),
            Err(e) => eprintln!(
                "dropping invalid LLM record: {e}: {}",
                truncate(&v.to_string(), 200)
            ),
        }
    }

    Ok(actions)
}

fn sanitize_action(mut a: LlmAction) -> LlmAction {
    a.establishment = clamp(a.establishment, 200);
    clamp_opt(&mut a.area, 120);
    clamp_opt(&mut a.city, 120);
    clamp_opt(&mut a.brand, 120);
    clamp_opt(&mut a.operator, 200);
    clamp_opt(&mut a.fssai_number, 64);
    clamp_opt(&mut a.details, 2000);
    a.violations = a
        .violations
        .into_iter()
        .map(|v| clamp(v, 300))
        .filter(|v| !v.is_empty())
        .collect();
    a.violations.truncate(5);
    a.platforms = a
        .platforms
        .into_iter()
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    a.platforms.truncate(6);
    a
}

fn clamp(s: String, max: usize) -> String {
    let trimmed = s.trim().to_string();
    if trimmed.chars().count() > max {
        trimmed.chars().take(max).collect()
    } else {
        trimmed
    }
}

fn clamp_opt(field: &mut Option<String>, max: usize) {
    if let Some(v) = field.take() {
        *field = Some(clamp(v, max));
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
    let body = body.trim_end_matches('`').trim();
    match body.split_once('\n') {
        Some((_lang, rest)) => rest.trim().to_string(),
        None => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActionType;

    #[test]
    fn strips_fences() {
        assert_eq!(strip_code_fences("```json\n[1,2]\n```"), "[1,2]");
        assert_eq!(strip_code_fences("[1,2]"), "[1,2]");
    }

    #[test]
    fn parses_minimal_action() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "[{\"establishment\":\"Domino's\",\"actionType\":\"licence_suspension\",\"sourceIndex\":0}]"}]}
            }]
        });
        let actions = parse_response(&body).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].establishment, "Domino's");
        assert_eq!(actions[0].action_type, ActionType::LicenceSuspension);
    }

    #[test]
    fn accepts_snake_case_keys() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "[{\"establishment\":\"X\",\"action_type\":\"inspection\",\"source_index\":0}]"}]}
            }]
        });
        let actions = parse_response(&body).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, ActionType::Inspection);
    }

    #[test]
    fn tolerates_null_arrays() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "[{\"establishment\":\"X\",\"actionType\":\"sealing\",\"violations\":null,\"platforms\":null,\"source_index\":0}]"}]}
            }]
        });
        let actions = parse_response(&body).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(actions[0].violations.is_empty());
        assert!(actions[0].platforms.is_empty());
    }

    #[test]
    fn drops_unknown_action_type() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "[{\"establishment\":\"X\",\"actionType\":\"bogus\",\"sourceIndex\":0}]"}]}
            }]
        });
        assert_eq!(parse_response(&body).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn empty_items_skip_llm() {
        let (actions, calls) = extract("key", "model", &[], false).await.unwrap();
        assert!(actions.is_empty());
        assert_eq!(calls, 0);
    }

    #[test]
    fn retry_delay_backs_off_and_caps() {
        assert_eq!(retry_delay(0, None).as_secs(), 15);
        assert_eq!(retry_delay(1, None).as_secs(), 30);
        assert_eq!(retry_delay(2, None).as_secs(), 60);
        assert_eq!(retry_delay(5, None).as_secs(), 60);
        assert_eq!(retry_delay(0, Some(5)).as_secs(), 5);
        assert_eq!(retry_delay(0, Some(300)).as_secs(), 120);
    }

    #[test]
    fn detects_quota_exhaustion() {
        assert!(is_quota_exhausted("Quota exceeded for metric: ..."));
        assert!(is_quota_exhausted("\"status\": \"RESOURCE_EXHAUSTED\""));
        assert!(!is_quota_exhausted("high demand, try again later"));
    }

    #[test]
    fn parses_fenced_text() {
        let text = "```json\n[{\"establishment\":\"X\",\"actionType\":\"inspection\",\"source_index\":0}]\n```";
        let actions = parse_llm_text(text).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].establishment, "X");
    }

    #[test]
    fn applies_batch_offset_to_source_index() {        let mut a = LlmAction {
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
            source_index: 3,
        };
        apply_offset(std::slice::from_mut(&mut a), 40);
        assert_eq!(a.source_index, 43);
    }
}
