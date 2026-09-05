use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::db::{ActionInsert, RecentEvent};
use crate::models::{LlmAction, NewsItem};

pub const SYSTEM_PROMPT: &str = include_str!("prompts/system.txt");

const DELIVERY_MODE: &str = include_str!("prompts/delivery.txt");
const DEDUPE_PROMPT: &str = include_str!("prompts/dedupe.txt");

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
        .unwrap_or_else(|| (15u64 * (1u64 << attempt.min(4))).min(120));
    // full jitter (Google/AWS rec): shave up to half so concurrent batches
    // don't re-hit the API in lockstep; nanos are enough entropy for 2 workers
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.subsec_nanos() as u64)
        .unwrap_or(0);
    Duration::from_secs(secs - nanos % (secs / 2 + 1))
}
const BATCH_SIZE: usize = 20;
// ponytail: free-tier quota is 5 RPM / 20 RPD — 2 concurrent workers re-hit
// 429s in lockstep, so run batches single-file and honor RetryInfo delays.
const MAX_CONCURRENT: usize = 1;

/// Floating alias: always tracks the newest Flash. Deliberate — pinned
/// 3.5-flash underperformed, so we ride latest instead of a version.
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_MAX_ATTEMPTS: usize = 3;
/// Verified live against the OpenRouter /models API. lfm-2.5 is built for
/// data extraction with structured-output support; z-ai stays last as the
/// privacy-safe resort under no-training settings.
const DEFAULT_OPENROUTER_MODEL: &str = "liquid/lfm-2.5-2.6b:free";
const OPENROUTER_FALLBACKS: &[&str] = &[
    "cohere/north-mini-code:free",
    "nvidia/nemotron-3-super-120b-a12b:free",
    "z-ai/glm-5.2:free",
];

pub async fn extract(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
    delivery: bool,
) -> Result<(Vec<LlmAction>, usize)> {
    if items.is_empty() {
        return Ok((Vec::new(), 0));
    }

    // Algorithmic pre-filter: only spend quota on items with an FDA/enforcement
    // signal. Drops obvious non-matches before batching; original indices are
    // carried alongside so source_index still addresses `items` for build_rows.
    let indexed: Vec<(usize, NewsItem)> = items
        .iter()
        .cloned()
        .enumerate()
        .filter(|(_, it)| is_likely_relevant(it))
        .collect();
    eprintln!(
        "llm pre-filter: {}/{} items relevant",
        indexed.len(),
        items.len()
    );
    if indexed.is_empty() {
        return Ok((Vec::new(), 0));
    }

    let api_key = api_key.to_owned();
    let model = model.to_owned();
    let requests = Arc::new(AtomicUsize::new(0));
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut tasks = Vec::new();

    for (batch_no, chunk) in indexed.chunks(BATCH_SIZE).enumerate() {
        let sem = Arc::clone(&sem);
        let requests = Arc::clone(&requests);
        let api_key = api_key.clone();
        let model = model.clone();
        let idx_map: Vec<usize> = chunk.iter().map(|(i, _)| *i).collect();
        let chunk_items: Vec<NewsItem> = chunk.iter().map(|(_, it)| it.clone()).collect();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let batch = extract_batch(&api_key, &model, &chunk_items, &requests, delivery).await;
            (batch_no, idx_map, chunk_items, batch)
        }));
    }

    let batch_count = tasks.len();
    let mut actions: Vec<LlmAction> = Vec::new();
    let mut failed = 0usize;
    for task in tasks {
        let (batch_no, idx_map, chunk_items, result) = task.await.context("llm batch task join")?;
        match result {
            Ok(mut batch) => {
                // Remap filtered-chunk positions back to original item indices.
                let mut remapped = Vec::with_capacity(batch.len());
                for a in batch.drain(..) {
                    if let Some(orig) = idx_map.get(a.source_index).copied() {
                        let mut a = a;
                        a.source_index = orig;
                        remapped.push(a);
                    }
                }
                actions.append(&mut remapped);
            }
            Err(e) => {
                // Rule fallback keeps this batch's data instead of losing it.
                let base: Vec<(usize, NewsItem)> = idx_map.into_iter().zip(chunk_items).collect();
                let fallback = rule_extract(&base);
                if fallback.is_empty() {
                    failed += 1;
                    eprintln!("llm batch {batch_no} failed: {e}");
                } else {
                    eprintln!(
                        "llm batch {batch_no} failed ({e:#}); rule fallback kept {}",
                        fallback.len()
                    );
                    actions.extend(fallback);
                }
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

fn haystack(it: &NewsItem) -> String {
    format!(
        "{} {} {}",
        it.title,
        it.snippet.as_deref().unwrap_or(""),
        it.source.as_deref().unwrap_or("")
    )
    .to_lowercase()
}

fn action_signal(text: &str) -> Option<crate::models::ActionType> {
    if text.contains("improvement notice") {
        Some(crate::models::ActionType::ImprovementNotice)
    } else if text.contains("licence") && text.contains("suspend")
        || text.contains("license") && text.contains("suspend")
    {
        Some(crate::models::ActionType::LicenceSuspension)
    } else if text.contains("stop business")
        || text.contains("closure") && text.contains("order")
        || text.contains("shut down") && text.contains("fda")
    {
        Some(crate::models::ActionType::StopBusiness)
    } else if text.contains("seal") {
        Some(crate::models::ActionType::Sealing)
    } else if text.contains("seiz") {
        Some(crate::models::ActionType::Seizure)
    } else if text.contains("reopen") {
        Some(crate::models::ActionType::Reopened)
    } else if text.contains("raid") || text.contains("inspect") || text.contains("fda") {
        Some(crate::models::ActionType::Inspection)
    } else {
        None
    }
}

fn is_likely_relevant(it: &NewsItem) -> bool {
    let h = haystack(it);
    action_signal(&h).is_some()
        && (h.contains("fda")
            || h.contains("food safety")
            || h.contains("licence")
            || h.contains("license")
            || h.contains("seal")
            || h.contains("seiz")
            || h.contains("raid")
            || h.contains("inspect"))
}

// Deterministic safety net: when a batch's LLM path fails, keep one minimal
// record per signal-bearing item so the run degrades instead of zeroing out.
// Fields the rules can't know (area, violations, dates) stay empty —
// build_rows + coerce_action_date fill date defaults downstream.
fn rule_extract(indexed: &[(usize, NewsItem)]) -> Vec<crate::models::LlmAction> {
    indexed
        .iter()
        .filter_map(|(orig, it)| {
            let h = haystack(it);
            let action_type = action_signal(&h)?;
            let name = it
                .title
                .split(['|', '-', ':'])
                .next()
                .unwrap_or(it.title.as_str())
                .trim()
                .chars()
                .take(120)
                .collect::<String>()
                .trim()
                .to_string();
            if name.is_empty() {
                return None;
            }
            Some(crate::models::LlmAction {
                establishment: name,
                area: None,
                city: None,
                brand: None,
                operator: None,
                outlet_type: None,
                action_type,
                action_date: None,
                violations: Vec::new(),
                compliance_score: None,
                fssai_number: None,
                details: it.snippet.clone(),
                platforms: Vec::new(),
                source_index: *orig,
            })
        })
        .collect()
}

/// Ask Gemini which of today's records describe an event already covered by
/// another record or a recent DB row. Returns indices into `rows` to drop and
/// the number of API calls used. Best effort: on any failure returns no drops
/// so the name-heuristic dedup in db::upsert_actions stays the safety net.
pub async fn collapse_dupes(
    api_key: &str,
    model: &str,
    rows: &[ActionInsert],
    recent: &[RecentEvent],
) -> (Vec<usize>, usize) {
    let requests = AtomicUsize::new(0);
    let result = collapse_once(api_key, model, rows, recent, &requests).await;
    let calls = requests.load(Ordering::Relaxed);
    match result {
        Ok(drops) => (drops, calls),
        Err(e) => {
            eprintln!("dupe-collapse llm failed ({e:#}); using name heuristics only");
            (Vec::new(), calls)
        }
    }
}

async fn collapse_once(
    api_key: &str,
    model: &str,
    rows: &[ActionInsert],
    recent: &[RecentEvent],
    requests: &AtomicUsize,
) -> Result<Vec<usize>> {
    let record = |id_prefix: &str,
                  establishment: &str,
                  action_type: &str,
                  action_date: NaiveDate,
                  city: &Option<String>,
                  area: &Option<String>| {
        json!({
            "id": id_prefix.to_string(),
            "establishment": establishment,
            "actionType": action_type,
            "actionDate": action_date.to_string(),
            "city": city,
            "area": area,
        })
    };
    let new_items: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            record(
                &format!("N{i}"),
                &r.establishment,
                &r.action_type,
                r.action_date,
                &r.city,
                &r.area,
            )
        })
        .collect();
    let known_items: Vec<Value> = recent
        .iter()
        .enumerate()
        .map(|(i, e)| {
            record(
                &format!("K{i}"),
                &e.establishment,
                &e.action_type,
                e.action_date,
                &e.city,
                &e.area,
            )
        })
        .collect();

    let payload = json!({
        "system_instruction": {"parts": [{"text": DEDUPE_PROMPT}]},
        "contents": [{"parts": [{"text": serde_json::to_string(
            &json!({"new": new_items, "known": known_items})
        ).context("serialize dupe payload")?}]}],
        "generationConfig": {
            "temperature": 0.0,
            "responseMimeType": "application/json",
            "maxOutputTokens": 2048
        }
    });
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
    );
    requests.fetch_add(1, Ordering::Relaxed);
    let resp = crate::http_client()
        .post(&url)
        .json(&payload)
        .send()
        .await?;
    let status = resp.status();
    let body: Value = resp.json().await.context("gemini dupe json body")?;
    if !status.is_success() {
        anyhow::bail!("gemini http {status}");
    }
    let text = response_text(&body)?;
    if text.trim().is_empty() {
        anyhow::bail!("gemini returned empty response");
    }
    Ok(drops_from_groups(&parse_groups(&text)?, rows.len()))
}

fn parse_groups(text: &str) -> Result<Vec<Vec<String>>> {
    let stripped = strip_code_fences(text.trim());
    let parsed: Value = serde_json::from_str(&stripped).map_err(|e| {
        anyhow!(
            "dupe response invalid JSON: {e}; body: {}",
            truncate(text, 300)
        )
    })?;
    let groups = parsed
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("dupe response missing \"groups\" array"))?;
    Ok(groups
        .iter()
        .filter_map(|g| g.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .filter(|g| !g.is_empty())
        .collect())
}

/// A group means "these ids are one event". If it touches a known row, every
/// new id in it is a re-report and gets dropped; otherwise the lowest-indexed
/// new id survives as the canonical record. Unknown or out-of-range ids are
/// ignored.
fn drops_from_groups(groups: &[Vec<String>], n_new: usize) -> Vec<usize> {
    let mut drop = std::collections::BTreeSet::new();
    for group in groups {
        let mut news: Vec<usize> = group
            .iter()
            .filter_map(|id| id.strip_prefix('N').and_then(|n| n.parse().ok()))
            .filter(|i| *i < n_new)
            .collect();
        news.sort_unstable();
        news.dedup();
        if news.is_empty() {
            continue;
        }
        let touches_known = group.iter().any(|id| id.starts_with('K'));
        for i in news.iter().skip(usize::from(!touches_known)) {
            drop.insert(*i);
        }
    }
    drop.into_iter().collect()
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
            let text = response_text(&body)?;
            if text.trim().is_empty() {
                // ponytail: thinking models can burn the token budget and return
                // 200 with zero text; retry, then OpenRouter via the normal path
                let finish = body["candidates"][0]["finishReason"]
                    .as_str()
                    .unwrap_or("unknown");
                eprintln!(
                    "gemini empty response (finishReason={finish}), attempt {attempt}; retrying"
                );
                tokio::time::sleep(retry_delay(attempt, None)).await;
                continue;
            }
            return parse_llm_text(&text);
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
            let wait = retry_after.or_else(|| retry_delay_from_body(&text));
            tokio::time::sleep(retry_delay(attempt, wait)).await;
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

// Gemini sends the backoff in the error body
// (google.rpc.RetryInfo.retryDelay, e.g. "58s"), not the retry-after header
// the old code read. Honor it so retries stop escalating 5/min into 20/day.
fn retry_delay_from_body(text: &str) -> Option<u64> {
    let body: Value = serde_json::from_str(text).ok()?;
    let details = body.get("error")?.get("details")?.as_array()?;
    details.iter().find_map(|d| {
        let s = d.get("retryDelay")?.as_str()?;
        let num: String = s
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        num.parse::<f64>()
            .ok()
            .map(|secs| secs.ceil().max(1.0) as u64)
    })
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
    // ponytail: env override so the next model rot is a secret change, not a deploy
    let model =
        std::env::var("OPENROUTER_MODEL").unwrap_or_else(|_| DEFAULT_OPENROUTER_MODEL.to_string());
    openrouter_with_model(api_key, &model, items, requests, delivery).await
}

async fn openrouter_with_model(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
    requests: &AtomicUsize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    eprintln!("openrouter attempt: model={model}, items={}", items.len());
    let fallbacks: Vec<String> = std::env::var("OPENROUTER_FALLBACKS")
        .map(|s| {
            s.split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty() && m != model)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|_| {
            OPENROUTER_FALLBACKS
                .iter()
                .filter(|m| **m != model)
                .map(|m| m.to_string())
                .collect()
        });
    let fallbacks_json: Vec<serde_json::Value> = fallbacks
        .iter()
        .map(|m| serde_json::Value::String(m.clone()))
        .collect();
    let mut payload = json!({
        "model": model,
        "models": fallbacks_json,
        "messages": [
            {"role": "system", "content": system_prompt(delivery)},
            {"role": "user", "content": serde_json::to_string(&json!({ "items": items })).context("serialize news batch")?}
        ],
        "tools": [{"type": "openrouter:web_search"}],
        "response_format": {"type": "json_object"},
        "plugins": [{"id": "response-healing"}],
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
        // 4xx guardrails (privacy 404, auth, bad request, dead balance) never
        // succeed on retry — fail fast instead of burning 3 backoffs per batch.
        let code = status.as_u16();
        if matches!(code, 400 | 401 | 403 | 404) {
            return Err(anyhow!("openrouter http {status} (non-retryable): {text}"));
        }
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
    Err(anyhow!(
        "openrouter {model} failed after {OPENROUTER_MAX_ATTEMPTS} attempts"
    ))
}

fn response_text(body: &Value) -> Result<String> {
    Ok(body
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
        .unwrap_or_default())
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
        let actions = parse_llm_text(&response_text(&body).unwrap()).unwrap();
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
        let actions = parse_llm_text(&response_text(&body).unwrap()).unwrap();
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
        let actions = parse_llm_text(&response_text(&body).unwrap()).unwrap();
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
        assert_eq!(
            parse_llm_text(&response_text(&body).unwrap())
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn empty_items_skip_llm() {
        let (actions, calls) = extract("key", "model", &[], false).await.unwrap();
        assert!(actions.is_empty());
        assert_eq!(calls, 0);
    }

    #[test]
    fn empty_parts_yield_empty_text() {
        let body = json!({
            "candidates": [{"finishReason": "MAX_TOKENS", "content": {"parts": []}}]
        });
        assert_eq!(response_text(&body).unwrap(), "");
        assert!(response_text(&json!({})).unwrap().is_empty());
    }

    #[test]
    fn retry_delay_backs_off_and_caps() {
        let d = |a: usize| retry_delay(a, None).as_secs();
        assert!((8..=15).contains(&d(0)));
        assert!((15..=30).contains(&d(1)));
        assert!((30..=60).contains(&d(2)));
        assert!((60..=120).contains(&d(3)));
        assert!((60..=120).contains(&d(5)));
        assert!((3..=5).contains(&retry_delay(0, Some(5)).as_secs()));
        assert!((60..=120).contains(&retry_delay(0, Some(300)).as_secs()));
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
    fn retry_delay_reads_gemini_retry_info() {
        let body = r#"{"error": {"details": [{"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "58s"}]}}"#;
        assert_eq!(retry_delay_from_body(body), Some(58));
        let frac = r#"{"error": {"details": [{"retryDelay": "6.13s"}]}}"#;
        assert_eq!(retry_delay_from_body(frac), Some(7));
        assert_eq!(retry_delay_from_body("not json"), None);
    }

    #[test]
    fn rule_fallback_keeps_signal_items_only() {
        let mk = |title: &str| NewsItem {
            title: title.into(),
            url: "https://x.test/1".into(),
            source: None,
            published: None,
            snippet: None,
        };
        let items = vec![
            (4, mk("Domino's licence suspended in Mumbai over pests")),
            (7, mk("cricket highlights and match report")),
        ];
        assert!(!is_likely_relevant(&items[1].1));
        let kept = rule_extract(&items);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source_index, 4);
        assert_eq!(
            kept[0].action_type,
            crate::models::ActionType::LicenceSuspension
        );
    }

    #[test]
    fn parses_group_response_shapes() {
        assert_eq!(
            parse_groups(r#"{"groups": [["N0","N2"], ["K1","N5"]]}"#).unwrap(),
            vec![
                vec!["N0".to_string(), "N2".to_string()],
                vec!["K1".to_string(), "N5".to_string()]
            ]
        );
        assert_eq!(
            parse_groups("```json\n{\"groups\": []}\n```").unwrap(),
            Vec::<Vec<String>>::new()
        );
        assert!(parse_groups("no json").is_err());
        assert!(parse_groups(r#"{"actions": []}"#).is_err());
    }

    #[test]
    fn drops_rereports_keeps_one_per_event() {
        // pure new-vs-new group: keep lowest index
        assert_eq!(
            drops_from_groups(&[vec!["N3".into(), "N1".into(), "N7".into()]], 10),
            vec![3, 7]
        );
        // group touching a known row: every new id is a re-report
        assert_eq!(
            drops_from_groups(&[vec!["K4".into(), "N2".into()]], 10),
            vec![2]
        );
        // junk ids and out-of-range indices are ignored
        assert!(drops_from_groups(&[vec!["N9".into(), "bogus".into()]], 3).is_empty());
        assert!(drops_from_groups(&[], 3).is_empty());
    }
}
