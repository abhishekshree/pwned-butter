use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use serde_json::{json, Value};

use crate::db::{ActionInsert, RecentEvent};
use crate::models::{ActionType, LlmAction, NewsItem};

pub const SYSTEM_PROMPT: &str = include_str!("prompts/system.txt");

const DELIVERY_MODE: &str = include_str!("prompts/delivery.txt");
const DEDUPE_PROMPT: &str = include_str!("prompts/dedupe.txt");

fn system_prompt(delivery: bool) -> Cow<'static, str> {
    if delivery {
        format!("{SYSTEM_PROMPT}{DELIVERY_MODE}").into()
    } else {
        SYSTEM_PROMPT.into()
    }
}

const MAX_ATTEMPTS: usize = 6;
const OPENROUTER_MAX_ATTEMPTS: usize = 3;
const BATCH_SIZE: usize = 20;
// ponytail: free-tier quota is 5 RPM / 20 RPD — concurrent batches re-hit
// 429s in lockstep, so batches run single-file with no spawn/semaphore.

/// Floating alias: always tracks the newest Flash. Deliberate — pinned
/// 3.5-flash underperformed, so we ride latest instead of a version.
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
/// Verified live against the OpenRouter /models API. lfm-2.5 is built for
/// data extraction with structured-output support; z-ai stays last as the
/// privacy-safe resort under no-training settings.
const DEFAULT_OPENROUTER_MODEL: &str = "liquid/lfm-2.5-2.6b:free";
const OPENROUTER_FALLBACKS: &[&str] = &[
    "cohere/north-mini-code:free",
    "nvidia/nemotron-3-super-120b-a12b:free",
    "z-ai/glm-5.2:free",
];

/// A relevance-filtered batch item; `orig` indexes the caller's `items`
/// slice so `source_index` still addresses it for `build_rows`.
struct Job {
    orig: usize,
    item: NewsItem,
}

pub async fn extract(
    api_key: &str,
    model: &str,
    items: &[NewsItem],
    delivery: bool,
) -> Result<(Vec<LlmAction>, usize)> {
    let mut jobs = Vec::new();
    for (orig, item) in items.iter().enumerate() {
        if triage(&haystack(item)).is_some() {
            jobs.push(Job {
                orig,
                item: item.clone(),
            });
        }
    }
    eprintln!(
        "llm pre-filter: {}/{} items relevant",
        jobs.len(),
        items.len()
    );
    if jobs.is_empty() {
        return Ok((Vec::new(), 0));
    }

    let mut calls = 0;
    let mut actions = Vec::new();
    let mut failed = 0;
    for (batch_no, chunk) in jobs.chunks(BATCH_SIZE).enumerate() {
        match extract_batch(api_key, model, chunk, &mut calls, delivery).await {
            Ok(batch) => actions.extend(batch),
            Err(e) => {
                let kept = rule_extract(chunk);
                if kept.is_empty() {
                    failed += 1;
                    eprintln!("llm batch {batch_no} failed: {e:#}");
                } else {
                    eprintln!(
                        "llm batch {batch_no} failed ({e:#}); rule fallback kept {}",
                        kept.len()
                    );
                    actions.extend(kept);
                }
            }
        }
    }
    let batch_count = jobs.len().div_ceil(BATCH_SIZE);
    if failed == batch_count {
        return Err(anyhow!("all {batch_count} llm batches failed"));
    }

    let mut seen = HashSet::new();
    actions = actions
        .into_iter()
        .filter(|a| !a.establishment.trim().is_empty())
        .filter(|a| {
            seen.insert((
                a.source_index,
                a.establishment.to_lowercase(),
                a.action_type,
            ))
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

/// Keyword rules as data, not branches: each entry is alternative
/// conjunctions + the action they signal. First match in table order wins,
/// so entries run most- to least-specific. Adding a keyword edits this
/// table, never the control flow below it.
const SIGNAL_RULES: &[(&[&[&str]], ActionType)] = &[
    (&[&["improvement notice"]], ActionType::ImprovementNotice),
    (
        &[&["licence", "suspend"], &["license", "suspend"]],
        ActionType::LicenceSuspension,
    ),
    (
        &[
            &["stop business"],
            &["closure", "order"],
            &["shut down", "fda"],
        ],
        ActionType::StopBusiness,
    ),
    (&[&["seal"]], ActionType::Sealing),
    (&[&["seiz"]], ActionType::Seizure),
    (&[&["reopen"]], ActionType::Reopened),
    (&[&["raid"], &["inspect"], &["fda"]], ActionType::Inspection),
];

/// Validated keyword signal: prose in, typed action out. Parsing is the
/// fallible boundary (TryFrom); mapping a signal to its action is total
/// (From). Distinct from ActionType::from_str, which parses wire codes —
/// one name, one concept per conversion.
struct Signal(ActionType);

#[derive(Debug)]
struct NoSignal;

impl TryFrom<&str> for Signal {
    type Error = NoSignal;

    fn try_from(hay: &str) -> Result<Self, Self::Error> {
        SIGNAL_RULES
            .iter()
            .find_map(|(groups, action)| {
                groups
                    .iter()
                    .any(|needles| needles.iter().all(|n| hay.contains(n)))
                    .then_some(*action)
            })
            .map(Signal)
            .ok_or(NoSignal)
    }
}

impl From<Signal> for ActionType {
    fn from(signal: Signal) -> Self {
        signal.0
    }
}

// Only these corroborate a generic-English trigger: "fda" and
// licence/license are unambiguous enough. Deliberately excludes
// seal/seiz/raid/inspect — those ARE the ambiguous words, so they
// can't corroborate themselves.
const CORROBORATION: &[&str] = &["fda", "food safety", "licence", "license"];

fn triage(hay: &str) -> Option<ActionType> {
    let action = ActionType::from(Signal::try_from(hay).ok()?);
    let needs_corroboration = matches!(
        action,
        ActionType::Sealing | ActionType::Seizure | ActionType::Inspection
    );
    (!needs_corroboration || CORROBORATION.iter().any(|c| hay.contains(c))).then_some(action)
}

// Deterministic safety net: when a batch's LLM path fails, keep one minimal
// record per signal-bearing item so the run degrades instead of zeroing out.
// Fields the rules can't know (area, violations, dates) stay empty —
// build_rows + coerce_action_date fill date defaults downstream.
fn rule_extract(jobs: &[Job]) -> Vec<LlmAction> {
    jobs.iter()
        .filter_map(|j| {
            let action_type = triage(&haystack(&j.item))?;
            let name: String = j
                .item
                .title
                .split(['|', '-', ':'])
                .next()?
                .trim()
                .chars()
                .take(120)
                .collect();
            let name = name.trim();
            (!name.is_empty()).then(|| {
                LlmAction::minimal(
                    name.to_string(),
                    action_type,
                    j.orig,
                    j.item.snippet.clone(),
                )
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
    let mut calls = 0;
    match collapse_once(api_key, model, rows, recent, &mut calls).await {
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
    calls: &mut usize,
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
    let resp = post(&gemini_url(model, api_key), None, &payload, calls).await?;
    let status = resp.status();
    let body: Value = resp.json().await.context("gemini dupe json body")?;
    if !status.is_success() {
        anyhow::bail!("gemini http {status}");
    }
    let text = response_text(&body);
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
    let mut drop = BTreeSet::new();
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

fn gemini_url(model: &str, api_key: &str) -> String {
    format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}")
}

fn retry_delay(attempt: usize, retry_after: Option<u64>) -> Duration {
    let secs = retry_after
        .map(|s| s.min(120))
        .unwrap_or_else(|| (15u64 * (1u64 << attempt.min(4))).min(120));
    // full jitter so concurrent batches don't re-hit the API in lockstep;
    // nanos are enough entropy for sequential workers
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.subsec_nanos() as u64)
        .unwrap_or(0);
    Duration::from_secs(secs - nanos % (secs / 2 + 1))
}

fn retry_after(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

async fn post(
    url: &str,
    key: Option<&str>,
    payload: &Value,
    calls: &mut usize,
) -> Result<reqwest::Response> {
    *calls += 1;
    let mut req = crate::http_client().post(url).json(payload);
    if let Some(key) = key {
        req = req.bearer_auth(key);
    }
    req.send().await.map_err(|e| anyhow!("request error: {e}"))
}

async fn extract_batch(
    api_key: &str,
    model: &str,
    chunk: &[Job],
    calls: &mut usize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    let payload = json!({
        "system_instruction": {"parts": [{"text": system_prompt(delivery)}]},
        "contents": [{"parts": [{"text": serde_json::to_string(
            &json!({ "items": chunk.iter().map(|j| &j.item).collect::<Vec<_>>() })
        ).context("serialize news batch")?}]}],
        "generationConfig": {
            "temperature": 0.0,
            "responseMimeType": "application/json",
            "maxOutputTokens": 8192
        }
    });
    let url = gemini_url(model, api_key);
    let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();

    for attempt in 0..MAX_ATTEMPTS {
        let resp = match post(&url, None, &payload, calls).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("gemini {e:#}");
                tokio::time::sleep(retry_delay(attempt, None)).await;
                continue;
            }
        };
        let wait = retry_after(&resp);
        let status = resp.status();
        let text = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
        if status.is_success() {
            let body: Value = serde_json::from_str(&text).context("gemini json")?;
            let text = response_text(&body);
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
            return Ok(remap(parse_llm_text(&text)?, chunk));
        }
        if status.as_u16() == 429 || status.is_server_error() {
            eprintln!("gemini http {status}, attempt {attempt}; {text}");
            if is_quota_exhausted(&text) {
                return escalate(openrouter_key.as_deref(), chunk, calls, delivery).await;
            }
            let wait = wait.or_else(|| retry_delay_from_body(&text));
            tokio::time::sleep(retry_delay(attempt, wait)).await;
            continue;
        }
        return Err(anyhow!("gemini http {status}: {text}"));
    }
    eprintln!("gemini API failed after {MAX_ATTEMPTS} attempts; falling back to openrouter");
    escalate(openrouter_key.as_deref(), chunk, calls, delivery).await
}

/// Single OpenRouter escalation path: fall back, then readdress positions
/// to the caller's item indices; orphan indices the LLM invented are dropped.
async fn escalate(
    openrouter_key: Option<&str>,
    chunk: &[Job],
    calls: &mut usize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    Ok(remap(
        fallback(openrouter_key, chunk, calls, delivery).await?,
        chunk,
    ))
}

fn remap(actions: Vec<LlmAction>, chunk: &[Job]) -> Vec<LlmAction> {
    actions
        .into_iter()
        .filter_map(|mut a| {
            let job = chunk.get(a.source_index)?;
            a.source_index = job.orig;
            Some(a)
        })
        .collect()
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

async fn fallback(
    api_key: Option<&str>,
    chunk: &[Job],
    calls: &mut usize,
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
    openrouter_with_model(api_key, &model, chunk, calls, delivery).await
}

/// OpenRouter 404s/4xx on guardrails never succeed on retry — fail fast
/// instead of burning backoffs per batch. Returns the error to abort with.
fn openrouter_fatal(status: u16, text: &str) -> Option<anyhow::Error> {
    match status {
        400 | 401 | 403 | 404 => Some(anyhow!("openrouter http {status} (non-retryable): {text}")),
        402 if text.contains("Insufficient credits")
            || text.contains("never purchased credits") =>
        {
            Some(anyhow!("openrouter 402 insufficient credits: {text}"))
        }
        _ => None,
    }
}

async fn openrouter_with_model(
    api_key: &str,
    model: &str,
    chunk: &[Job],
    calls: &mut usize,
    delivery: bool,
) -> Result<Vec<LlmAction>> {
    eprintln!("openrouter attempt: model={model}, items={}", chunk.len());
    let mut fallbacks: Vec<String> = std::env::var("OPENROUTER_FALLBACKS")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|_| OPENROUTER_FALLBACKS.iter().map(|m| m.to_string()).collect());
    fallbacks.retain(|m| m != model);
    let mut payload = json!({
        "model": model,
        "models": fallbacks,
        "messages": [
            {"role": "system", "content": system_prompt(delivery)},
            {"role": "user", "content": serde_json::to_string(
                &json!({ "items": chunk.iter().map(|j| &j.item).collect::<Vec<_>>() })
            ).context("serialize news batch")?}
        ],
        "tools": [{"type": "openrouter:web_search"}],
        "response_format": {"type": "json_object"},
        "plugins": [{"id": "response-healing"}],
        "temperature": 0.0,
        "max_tokens": 32768
    });
    for attempt in 0..OPENROUTER_MAX_ATTEMPTS {
        let resp = match post(OPENROUTER_URL, Some(api_key), &payload, calls).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("openrouter request error: {e}");
                tokio::time::sleep(retry_delay(attempt, None)).await;
                continue;
            }
        };
        let wait = retry_after(&resp);
        let status = resp.status();
        let text = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<unreadable body: {e}>"));
        if status.is_success() {
            let body: Value = serde_json::from_str(&text).context("openrouter json")?;
            let responded = body["model"].as_str().unwrap_or(model);
            let citations = body["citations"].as_array().map_or(0, Vec::len);
            eprintln!("openrouter ok: model={responded}, web_searches={citations}");
            let text = body["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| anyhow!("no text in openrouter response"))?;
            return parse_llm_text(text);
        }
        eprintln!("openrouter http {status}, attempt {attempt}; {text}");
        if let Some(e) = openrouter_fatal(status.as_u16(), &text) {
            return Err(e);
        }
        if status.as_u16() == 402 {
            let cur = payload["max_tokens"].as_u64().unwrap_or(32768);
            if cur > 8192 {
                let lower = cur / 2;
                payload["max_tokens"] = json!(lower);
                eprintln!("openrouter 402 (low balance); retrying with max_tokens={lower}");
                continue;
            }
            return Err(anyhow!("openrouter low balance (402): {text}"));
        }
        tokio::time::sleep(retry_delay(attempt, wait)).await;
    }
    Err(anyhow!(
        "openrouter {model} failed after {OPENROUTER_MAX_ATTEMPTS} attempts"
    ))
}

fn response_text(body: &Value) -> String {
    body.pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
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

    let mut actions = Vec::with_capacity(raw.len());
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
        let actions = parse_llm_text(&response_text(&body)).unwrap();
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
        let actions = parse_llm_text(&response_text(&body)).unwrap();
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
        let actions = parse_llm_text(&response_text(&body)).unwrap();
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
        assert_eq!(parse_llm_text(&response_text(&body)).unwrap().len(), 0);
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
        assert_eq!(response_text(&body), "");
        assert!(response_text(&json!({})).is_empty());
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
    fn triage_needs_corroboration_only_for_ambiguous_types() {
        let yes_fda = "fda raid seals eatery";
        let no_ctx = "shop sealed after fire";
        assert_eq!(triage(yes_fda), Some(ActionType::Sealing));
        assert_eq!(triage(no_ctx), None);
        // unambiguous types pass with no corroboration
        assert_eq!(
            triage("outlet served improvement notice"),
            Some(ActionType::ImprovementNotice)
        );
        assert_eq!(triage("eatery reopened"), Some(ActionType::Reopened));
        assert_eq!(
            triage("licence suspended over pests"),
            Some(ActionType::LicenceSuspension)
        );
    }

    #[test]
    fn rule_fallback_keeps_signal_items_only() {
        let mk = |title: &str| Job {
            orig: 0,
            item: NewsItem {
                title: title.into(),
                url: "https://x.test/1".into(),
                source: None,
                published: None,
                snippet: None,
            },
        };
        let jobs = vec![
            Job {
                orig: 4,
                item: mk("Domino's licence suspended in Mumbai over pests").item,
            },
            Job {
                orig: 7,
                item: mk("cricket highlights and match report").item,
            },
        ];
        assert!(triage(&haystack(&jobs[1].item)).is_none());
        let kept = rule_extract(&jobs);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source_index, 4);
        assert_eq!(kept[0].action_type, ActionType::LicenceSuspension);
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
