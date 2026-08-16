use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::models::{LlmAction, NewsItem};

const SYSTEM_PROMPT: &str = "You are a structured-data extractor for a tracker of Maharashtra \
FDA (Food and Drug Administration) food-safety enforcement. Given JSON news items from India, \
extract one record per food establishment that faced a concrete regulatory action.

Output JSON uses camelCase keys exactly as listed: establishment, area, city, brand, \
operator, outletType, actionType, actionDate, violations, complianceScore, platforms, \
details, sourceIndex.

Field rules:
- establishment: the business or establishment name as reported (e.g. \"Noor Mohammadi Hotel\", \"Blink Commerce\").
- brand: the national/brand name if applicable (Domino's, Pizza Hut, Burger King, KFC, Starbucks, Blinkit, Zepto, Swiggy Instamart), otherwise null.
- area: locality within the city (e.g. Vile Parle West) if reported, else null.
- city: city/locality name (Mumbai, Pune, Nashik, Satara, Karad, Palghar...).
- actionType: one of licence_suspension, stop_business, improvement_notice, sealing, seizure, inspection, reopened.
- actionDate: the inspection or order date in YYYY-MM-DD when stated, otherwise the article publication date.
- violations: up to 5 short phrases summarising the cited violations (hygiene, pest infestation, expired stock, missing records, unhygienic storage...).
- complianceScore: the reported percentage score (integer) only when the article states one, else omit.
- platforms: delivery/quick-commerce platforms named in the article, lowercase (zomato, swiggy, blinkit, zepto, instamart, bigbasket...).
- details: one sentence of crucial context (e.g. reopened after compliance, appeal filed), else null.
- sourceIndex: the index of the source item this record came from (required).

Return a JSON array only. If an item reports no concrete enforcement record against a named \
establishment, skip it entirely. Optional string fields may be null.";

const MAX_ATTEMPTS: usize = 3;

pub async fn extract(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
) -> Result<(Vec<LlmAction>, usize)> {
    if items.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let payload = json!({
        "system_instruction": {"parts": [{"text": SYSTEM_PROMPT}]},
        "contents": [{"parts": [{"text": serde_json::to_string(&json!({ "items": items })).context("serialize news batch")?}]}],
        "generationConfig": {
            "temperature": 0.0,
            "responseMimeType": "application/json"
        }
    });
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
    );
    let client = crate::http_client();

    for attempt in 0..MAX_ATTEMPTS {
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
            let actions = parse_response(&body)?;
            return Ok((actions, 1));
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

    let mut seen = std::collections::HashSet::new();
    Ok(actions
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
        .collect())
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
    fn drops_unknown_action_type() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "[{\"establishment\":\"X\",\"actionType\":\"bogus\",\"sourceIndex\":0}]"}]}
            }]
        });
        assert_eq!(parse_response(&body).unwrap().len(), 0);
    }
}
