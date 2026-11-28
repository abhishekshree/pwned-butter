use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
pub(crate) use genai::Client;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, JsonSpec};
use genai::resolver::AuthData;
use serde_json::{Value, json};
use tokio::time::sleep;
use tracing::warn;

use crate::config::{Llm, Provider};
use crate::domain::{Article, Candidate};
use crate::models::{LlmAction, Sanitized};

use super::extract::{COLLAPSE_PROMPT, SYSTEM_PROMPT, strip_code_fences};
use super::triage::{TRIAGE_PROMPT, Verdict, parse_verdicts, triage_schema};

const MAX_ATTEMPTS: usize = 3;
/// A 429 is a tokens-per-minute window, so a short retry just burns more tokens.
const RATE_LIMIT_PAUSE: Duration = Duration::from_secs(60);

fn truncate(s: &str, max: usize) -> String {
    let out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{out}…")
    } else {
        out
    }
}

pub(crate) fn client(llm: &Llm) -> AnyResult<Client> {
    let adapter = match llm.provider {
        Provider::OpenRouter => AdapterKind::OpenRouter,
        Provider::Groq => AdapterKind::Groq,
    };
    Client::builder()
        .append_provider_config(adapter, AuthData::from_single(llm.key.expose()))
        .build()
        .context("build genai client")
}

fn json_options(max_tokens: u32, schema: Value, structured: bool) -> ChatOptions {
    let options = ChatOptions::default()
        .with_temperature(0.0)
        .with_max_tokens(max_tokens);
    if structured {
        options.with_response_format(JsonSpec::new("response", schema))
    } else {
        options
    }
}

pub(crate) struct ExtractOptions<'a> {
    pub(crate) models: &'a [String],
    pub(crate) structured: bool,
}

// Attempts are returned as data so a fully failed batch still reports its calls.
async fn chat_text(
    client: &Client,
    model: &str,
    request: ChatRequest,
    options: &ChatOptions,
) -> (AnyResult<String>, usize) {
    let mut calls = 0;
    for attempt in 0..MAX_ATTEMPTS {
        calls += 1;
        let error = match client
            .exec_chat(model, request.clone(), Some(options))
            .await
        {
            Ok(response) => match response.into_first_text() {
                Some(text) if !text.trim().is_empty() => return (Ok(text), calls),
                _ => anyhow::anyhow!("genai response had no text"),
            },
            Err(error) => anyhow::anyhow!("genai request failed: {error}"),
        };
        if attempt + 1 == MAX_ATTEMPTS {
            return (Err(error), calls);
        }
        let backoff = if format!("{error:#}").contains("429") {
            RATE_LIMIT_PAUSE
        } else {
            Duration::from_secs(1 << attempt)
        };
        sleep(backoff).await;
    }
    unreachable!("MAX_ATTEMPTS is nonzero")
}

fn log_llm(step: &str, model: &str, input: &str, output: &AnyResult<String>) {
    let error = output.as_ref().err().map(|error| format!("{error:#}"));
    tracing::debug!(
        target: "artifact",
        step,
        model,
        input = %input,
        output = %output.as_deref().unwrap_or_default(),
        error = error.as_deref(),
        "llm_call"
    );
}

/// Titles plus a short snippet are enough to judge relevance. URLs cost ~30
/// tokens each and the prompt forbids opening them, so they are not sent.
fn triage_payload(items: &[Article]) -> AnyResult<String> {
    let rows: Vec<Value> = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "sourceIndex": index,
                "title": item.title,
                "text": truncate(item.snippet.as_deref().unwrap_or(""), 150),
            })
        })
        .collect();
    serde_json::to_string(&json!({"items": rows})).context("serialize triage batch")
}

pub(crate) async fn triage_batch(
    client: &Client,
    model: &str,
    items: &[Article],
    structured: bool,
) -> (AnyResult<Vec<Verdict>>, usize) {
    let user = match triage_payload(items) {
        Ok(user) => user,
        Err(error) => return (Err(error), 0),
    };
    let request =
        ChatRequest::from_system(TRIAGE_PROMPT).append_message(ChatMessage::user(user.clone()));
    let options = json_options(2048, triage_schema(), structured);
    let (text, calls) = chat_text(client, model, request, &options).await;
    log_llm("triage", model, &user, &text);
    (text.and_then(|text| parse_verdicts(&text)), calls)
}

/// Names and dates only: the model needs no article text to spot alias variants.
pub(crate) async fn collapse_batch(
    client: &Client,
    model: &str,
    candidates: &[Candidate],
) -> (AnyResult<String>, usize) {
    let records: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(id, c)| {
            json!({
                "id": id,
                "name": c.establishment,
                "date": c.action_date,
                "type": c.action_type.to_string(),
            })
        })
        .collect();
    let user = match serde_json::to_string(&json!({"records": records})) {
        Ok(user) => user,
        Err(error) => return (Err(error).context("serialize collapse batch"), 0),
    };
    let request =
        ChatRequest::from_system(COLLAPSE_PROMPT).append_message(ChatMessage::user(user.clone()));
    // Groq rejects top-level array schemas; the response is parsed by hand anyway.
    let options = json_options(1024, json!({}), false);
    let (text, calls) = chat_text(client, model, request, &options).await;
    log_llm("collapse", model, &user, &text);
    (text, calls)
}

/// The model returns chunk-local indices; `absolute` resolves them and drops out-of-range values.
pub(crate) async fn extract_batch<'a>(
    client: &Client,
    chunk: &[&'a Article],
    opts: &ExtractOptions<'_>,
) -> (AnyResult<Vec<(&'a Article, LlmAction)>>, usize) {
    let items: Vec<Value> = chunk
        .iter()
        .map(|item| {
            json!({
                "title": item.title,
                "source": item.source,
                "published": item.published,
                "snippet": truncate(item.snippet.as_deref().unwrap_or(""), 2500),
            })
        })
        .collect();
    let user = match serde_json::to_string(&json!({"items": items})) {
        Ok(user) => user,
        Err(error) => return (Err(error).context("serialize news batch"), 0),
    };
    let request =
        ChatRequest::from_system(SYSTEM_PROMPT).append_message(ChatMessage::user(user.clone()));
    let options = json_options(8192, json!({"type": "array"}), opts.structured);
    let mut calls = 0;
    let mut last_error = anyhow::anyhow!("no LLM models configured");
    for model in opts.models {
        let (text, used) = chat_text(client, model, request.clone(), &options).await;
        log_llm("extract", model, &user, &text);
        calls += used;
        match text
            .and_then(|text| parse_llm_text(&text))
            .map(|actions| absolute(actions, chunk))
        {
            Ok(actions) => return (Ok(actions), calls),
            Err(error) => {
                warn!(model = %model, error = %format!("{error:#}"), "extraction model failed");
                last_error = error;
            }
        }
    }
    (Err(last_error), calls)
}

fn absolute<'a>(actions: Vec<LlmAction>, chunk: &[&'a Article]) -> Vec<(&'a Article, LlmAction)> {
    actions
        .into_iter()
        .filter_map(|action| {
            let Some(article) = chunk.get(action.source_index) else {
                warn!(
                    index = action.source_index,
                    "extraction result index out of range"
                );
                return None;
            };
            Some((*article, action))
        })
        .collect()
}

pub(crate) fn parse_llm_text(text: &str) -> AnyResult<Vec<LlmAction>> {
    let text = strip_code_fences(text);
    let parsed: Value = serde_json::from_str(&text).map_err(|error| {
        anyhow::anyhow!(
            "LLM returned invalid JSON: {error}; body: {}",
            truncate(&text, 300)
        )
    })?;
    let raw = match parsed {
        Value::Array(items) => items,
        Value::Object(map) => ["actions", "records", "items", "data"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_array).cloned())
            .or_else(|| map.values().find_map(Value::as_array).cloned())
            .ok_or_else(|| {
                anyhow::anyhow!("expected JSON array or object with array of actions/records")
            })?,
        _ => anyhow::bail!("unexpected LLM response shape"),
    };
    let mut actions = Vec::with_capacity(raw.len());
    for value in raw {
        match serde_json::from_value(value.clone()) {
            Ok(action) => actions.push(action),
            Err(error) => warn!(
                error = %format!("{error:#}"),
                record = %truncate(&value.to_string(), 200),
                "dropping invalid LLM record"
            ),
        }
    }
    Ok(actions)
}

pub(crate) fn sanitize_action(mut action: LlmAction) -> Sanitized<LlmAction> {
    action.establishment = clamp(action.establishment, 200);
    clamp_opt(&mut action.area, 120);
    clamp_opt(&mut action.city, 120);
    clamp_opt(&mut action.brand, 120);
    clamp_opt(&mut action.operator, 200);
    clamp_opt(&mut action.fssai_number, 64);
    clamp_opt(&mut action.details, 2000);
    action.violations = action
        .violations
        .into_iter()
        .map(|value| clamp(value, 300))
        .filter(|value| !value.is_empty())
        .collect();
    action.violations.truncate(5);
    action.platforms = action
        .platforms
        .into_iter()
        .map(|platform| platform.trim().to_lowercase())
        .filter(|platform| !platform.is_empty())
        .collect();
    action.platforms.truncate(6);
    Sanitized::new(action)
}

fn clamp(value: String, max: usize) -> String {
    let value = value.trim().to_string();
    if value.chars().count() > max {
        value.chars().take(max).collect()
    } else {
        value
    }
}

fn clamp_opt(field: &mut Option<String>, max: usize) {
    if let Some(value) = field.take() {
        *field = Some(clamp(value, max));
    }
}

#[cfg(test)]
#[path = "genai_test.rs"]
mod tests;
