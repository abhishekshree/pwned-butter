use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::models::{LlmAction, NewsItem};

pub const SYSTEM_PROMPT: &str = "You are a structured-data extractor for a tracker of Maharashtra \
FDA (Food and Drug Administration) food-safety enforcement. Given JSON news items (news \
articles and X/Twitter posts) from India, extract one record per RETAIL food business that \
faced a concrete regulatory action: licence suspension, stop business, improvement notice, \
sealing, seizure, or an inspection/raid with cited violations.

Scope rules:
- Only extract places a consumer could order food from or eat at: restaurants, hotels, \
cafés, cloud kitchens, fast-food outlets, bakeries, sweet shops, dhabas, messes, and \
quick-commerce stores (Blinkit, Zepto, Instamart, BigBasket) that sell food.
- Skip manufacturers, food-processing plants, warehouses, wholesale/B2B suppliers, dairy \
plants, farms, slaughterhouses, and any non-food business.
- If one item lists several named outlets under a single action, emit a separate record per \
named outlet. Never invent establishments that are not named in the source.

Output JSON uses camelCase keys exactly as listed: establishment, area, city, brand, operator, \
outletType, actionType, actionDate, violations, complianceScore, platforms, details, \
sourceIndex.

Field rules:
- establishment: the outlet name as reported (e.g. \"Noor Mohammadi Hotel\", \"Blink Commerce Malad\").
- brand: national/chain brand if applicable (Domino's, Pizza Hut, Burger King, KFC, Blinkit, Zepto), else omit.
- area: locality within the city (e.g. Vile Parle West), else omit.
- city: city/locality name (Mumbai, Navi Mumbai, Thane, Pune, Nashik...).
- outletType: one of restaurant, cloud_kitchen, quick_commerce, dhaba, hotel, bakery, club, mess, dairy, street_vendor, other.
- actionType: one of licence_suspension, stop_business, improvement_notice, sealing, seizure, inspection, reopened.
- actionDate: inspection or order date in YYYY-MM-DD when stated, otherwise the source publication date.
- violations: array of up to 5 short phrases summarising the cited violations (hygiene, pest \
infestation, expired stock, missing records, unhygienic storage). Omit when none cited.
- complianceScore: the reported percentage score (integer) only when the source states one, else omit.
- platforms: lowercased delivery apps the outlet operates on, from the source OR from your own \
knowledge (zomato, swiggy, blinkit, zepto, instamart, bigbasket). Omit when none applies.
- details: one sentence of crucial context (e.g. reopened after compliance, appeal filed), else omit.
- sourceIndex: the index of the source item this record came from (required).

Return a JSON array only. If an item reports no concrete action against a named retail food \
outlet, skip it entirely. Optional fields may be null.";

const DELIVERY_MODE: &str = "\n\nThis is a Mumbai consumer-delivery run. Additional hard rules:\n\
- Include ONLY establishments in the Mumbai metropolitan region: Mumbai, Navi Mumbai, Thane, \
and neighbouring suburbs (Kalyan, Dombivli, Mulund, Goregaon, Andheri, Bandra, Worli...).\n\
- Every included record MUST list at least one of platforms: zomato, swiggy, blinkit, \
instamart, zepto, bigbasket.\n\
- Drop any record outside the Mumbai region or with no delivery-app presence.";

fn system_prompt(delivery: bool) -> String {
    if delivery {
        format!("{SYSTEM_PROMPT}{DELIVERY_MODE}")
    } else {
        SYSTEM_PROMPT.to_string()
    }
}

const MAX_ATTEMPTS: usize = 3;
const BATCH_SIZE: usize = 20;
const MAX_CONCURRENT: usize = 2;

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

    for attempt in 0..MAX_ATTEMPTS {
        requests.fetch_add(1, Ordering::Relaxed);
        let resp = match client.post(&url).json(&payload).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("gemini request error: {e}");
                tokio::time::sleep(Duration::from_secs(10 * (attempt as u64 + 1))).await;
                continue;
            }
        };
        if resp.status().is_success() {
            let body: Value = resp.json().await.context("gemini json")?;
            return parse_response(&body);
        }
        let status = resp.status();
        let text = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
        if status.as_u16() == 429 || status.is_server_error() {
            eprintln!("gemini http {status}, attempt {attempt}; {text}");
            tokio::time::sleep(Duration::from_secs(15 * (attempt as u64 + 1))).await;
            continue;
        }
        return Err(anyhow!("gemini http {status}: {text}"));
    }
    Err(anyhow!("gemini API failed after {MAX_ATTEMPTS} attempts"))
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

    let text = strip_code_fences(text.as_str());
    let parsed: Value = serde_json::from_str(&text).map_err(|e| {
        anyhow!(
            "gemini returned invalid JSON: {e}; body: {}",
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
        _ => return Err(anyhow!("unexpected gemini response shape")),
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
    fn applies_batch_offset_to_source_index() {
        let mut a = LlmAction {
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
