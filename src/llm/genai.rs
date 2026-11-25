use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
pub(crate) use genai::Client;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, JsonSpec};
use genai::resolver::AuthData;
use serde_json::{Value, json};
use tokio::time::sleep;
use tracing::warn;

use crate::config::Llm;
use crate::domain::{Article, Candidate};
use crate::models::{LlmAction, Sanitized};

use super::triage::{TRIAGE_PROMPT, Verdict, parse_verdicts, triage_schema};
use super::{COLLAPSE_PROMPT, SYSTEM_PROMPT, strip_code_fences, truncate};

const MAX_ATTEMPTS: usize = 3;
/// A 429 is a tokens-per-minute window, so a short retry just burns more tokens.
const RATE_LIMIT_PAUSE: Duration = Duration::from_secs(60);

pub(crate) fn client(llm: &Llm) -> AnyResult<Client> {
    let mut builder = Client::builder();
    let (adapter, key) = match llm {
        Llm::Gemini { key, .. } => (AdapterKind::Gemini, key),
        Llm::OpenRouter { key, .. } => (AdapterKind::OpenRouter, key),
        Llm::Groq { key, .. } => (AdapterKind::Groq, key),
    };
    builder = builder.append_provider_config(adapter, AuthData::from_single(key.expose()));
    builder.build().context("build genai client")
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
    let request = ChatRequest::from_system(TRIAGE_PROMPT).append_message(ChatMessage::user(user));
    let options = json_options(2048, triage_schema(), structured);
    let (text, calls) = chat_text(client, model, request, &options).await;
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
    let request = ChatRequest::from_system(COLLAPSE_PROMPT).append_message(ChatMessage::user(user));
    // Groq rejects top-level array schemas; the response is parsed by hand anyway.
    let options = json_options(1024, json!({}), false);
    chat_text(client, model, request, &options).await
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
    let request = ChatRequest::from_system(SYSTEM_PROMPT).append_message(ChatMessage::user(user));
    let options = json_options(8192, json!({"type": "array"}), opts.structured);
    let mut calls = 0;
    let mut last_error = anyhow::anyhow!("no LLM models configured");
    for model in opts.models {
        let (text, used) = chat_text(client, model, request.clone(), &options).await;
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
mod tests {
    use std::ptr::eq;

    use super::*;
    use crate::models::ActionType;

    fn article(title: &str) -> Article {
        Article {
            title: title.into(),
            url: "https://example.test/1".into(),
            source: None,
            published: None,
            snippet: None,
        }
    }

    #[test]
    fn triage_payload_omits_urls_and_caps_snippets() {
        let item = Article {
            title: "FDA suspends licence".into(),
            url: "https://example.test/a/long/path/that/wastes/tokens".into(),
            source: Some("Times".into()),
            published: None,
            snippet: Some("x".repeat(1000)),
        };
        let payload = triage_payload(&[item]).expect("serialize compact triage payload");
        assert!(
            !payload.contains("example.test"),
            "URLs cost tokens and the prompt forbids opening them"
        );
        let parsed: Value = serde_json::from_str(&payload).expect("valid JSON payload");
        assert_eq!(parsed["items"][0]["sourceIndex"], 0);
        assert_eq!(
            parsed["items"][0]["text"].as_str().unwrap().chars().count(),
            151,
            "snippet is capped at 150 chars plus the truncation ellipsis"
        );
    }

    #[test]
    fn parses_fenced_actions_and_object_wrappers() {
        let actions = parse_llm_text(
            "```json\n{\"records\":[{\"establishment\":\"X\",\"actionType\":\"inspection\",\"sourceIndex\":1}]}\n```",
        )
        .unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].establishment, "X");
        assert_eq!(actions[0].action_type, ActionType::Inspection);
        assert_eq!(actions[0].source_index, 1);
        assert!(parse_llm_text("not json").is_err());
    }

    #[test]
    fn sanitizes_strings_and_limits_lists() {
        let action: LlmAction = serde_json::from_value(json!({
            "establishment": "  X  ",
            "actionType": "inspection",
            "sourceIndex": 0,
            "violations": ["  issue  ", ""],
            "platforms": [" Swiggy ", " "]
        }))
        .unwrap();
        let action = sanitize_action(action).into_inner();
        assert_eq!(action.establishment, "X");
        assert_eq!(action.violations, vec!["issue"]);
        assert_eq!(action.platforms, vec!["swiggy"]);
    }

    #[test]
    fn attaches_source_articles_and_drops_out_of_range() {
        let first = article("first");
        let second = article("second");
        let chunk: Vec<&Article> = vec![&first, &second];
        let actions = vec![
            LlmAction {
                source_index: 1,
                establishment: "B".into(),
                ..Default::default()
            },
            LlmAction {
                source_index: 0,
                establishment: "A".into(),
                ..Default::default()
            },
            LlmAction {
                source_index: 7,
                establishment: "out of range".into(),
                ..Default::default()
            },
        ];

        let result = absolute(actions, &chunk);

        assert_eq!(result.len(), 2, "out-of-range index is dropped");
        assert!(
            eq(result[0].0, &second),
            "chunk-local index 1 resolves to the second article"
        );
        assert_eq!(result[0].1.establishment, "B");
        assert_eq!(result[0].1.source_index, 1, "wire index is left untouched");
        assert!(
            eq(result[1].0, &first),
            "chunk-local index 0 resolves to the first article"
        );
        assert_eq!(result[1].1.establishment, "A");
        assert_eq!(result[1].1.source_index, 0, "wire index is left untouched");
    }
}
