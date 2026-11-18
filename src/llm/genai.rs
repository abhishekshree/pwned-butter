use std::time::Duration;

use anyhow::{Context, Result};
use genai::Client;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, JsonSpec};
use genai::resolver::AuthData;
use serde_json::{Value, json};
use tracing::warn;

use crate::domain::Article;
use crate::models::LlmAction;
use crate::report::RunMetrics;

use super::triage::{Job, TRIAGE_PROMPT, Verdict, parse_verdicts, triage_schema};
use super::{SYSTEM_PROMPT, strip_code_fences, truncate};

const MAX_ATTEMPTS: usize = 3;

pub(crate) fn client(api_key: &str, openrouter_key: Option<&str>) -> Result<Client> {
    let mut builder = Client::builder()
        .append_provider_config(AdapterKind::Gemini, AuthData::from_single(api_key));
    if let Some(key) = openrouter_key {
        builder =
            builder.append_provider_config(AdapterKind::OpenRouter, AuthData::from_single(key));
    }
    builder.build().context("build genai client")
}

fn json_options(max_tokens: u32, schema: Value) -> ChatOptions {
    ChatOptions::default()
        .with_temperature(0.0)
        .with_max_tokens(max_tokens)
        .with_response_format(JsonSpec::new("response", schema))
}

/// Sends one request, retrying transient failures. `calls` counts every
/// attempt so a fully failed batch is still visible in the run metrics.
async fn chat_text(
    client: &Client,
    model: &str,
    request: ChatRequest,
    options: &ChatOptions,
    calls: &mut usize,
) -> Result<String> {
    for attempt in 0..MAX_ATTEMPTS {
        *calls += 1;
        let error = match client
            .exec_chat(model, request.clone(), Some(options))
            .await
        {
            Ok(response) => {
                if let Some(text) = response.into_first_text()
                    && !text.trim().is_empty()
                {
                    return Ok(text);
                }
                anyhow::anyhow!("genai response had no text")
            }
            Err(error) => anyhow::anyhow!("genai request failed: {error}"),
        };
        if attempt + 1 == MAX_ATTEMPTS {
            return Err(error);
        }
        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
    }
    unreachable!("MAX_ATTEMPTS is nonzero")
}

/// Strict relevance pass over one chunk of articles.
pub(crate) async fn triage_batch(
    client: &Client,
    model: &str,
    chunk: &[Article],
    metrics: &mut RunMetrics,
) -> Result<Vec<Verdict>> {
    let items: Vec<Value> = chunk
        .iter()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "sourceIndex": index,
                "title": item.title,
                "publisher": item.source,
                "published": item.published,
                "url": item.url,
                "text": item.snippet.as_deref().unwrap_or(""),
            })
        })
        .collect();
    let user = serde_json::to_string(&json!({"items": items}))?;
    let request = ChatRequest::from_system(TRIAGE_PROMPT).append_message(ChatMessage::user(user));
    let options = json_options(4096, triage_schema());
    let text = chat_text(client, model, request, &options, &mut metrics.triage_calls).await?;
    parse_verdicts(&text)
}

/// Extract actions for one chunk, pairing each result with its source article.
/// The model returns chunk-local indices; `absolute` resolves them through
/// `chunk` and drops out-of-range values.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn extract_batch<'a>(
    client: &Client,
    model: &str,
    chunk: &[Job<'a>],
    openrouter_key: Option<&str>,
    openrouter_model: &str,
    openrouter_fallbacks: &[String],
    metrics: &mut RunMetrics,
) -> Result<Vec<(&'a Article, LlmAction)>> {
    let items: Vec<Value> = chunk
        .iter()
        .map(|job| {
            let item = job.item;
            json!({
                "title": item.title,
                "url": item.url,
                "source": item.source,
                "published": item.published,
                "snippet": item.snippet,
            })
        })
        .collect();
    let user = serde_json::to_string(&json!({"items": items})).context("serialize news batch")?;
    let request = ChatRequest::from_system(SYSTEM_PROMPT).append_message(ChatMessage::user(user));
    let options = json_options(16_384, json!({"type": "array"}));

    let gemini = chat_text(
        client,
        model,
        request.clone(),
        &options,
        &mut metrics.extraction_calls,
    )
    .await
    .and_then(|text| parse_llm_text(&text))
    .map(|actions| absolute(actions, chunk));
    match gemini {
        Ok(actions) => return Ok(actions),
        Err(error) => warn!(
            model,
            error = %format!("{error:#}"),
            "gemini model failed"
        ),
    }

    warn!("all gemini models failed; falling back to openrouter");
    if openrouter_key.is_none() {
        anyhow::bail!("gemini API failed and no OPENROUTER_API_KEY fallback configured");
    }
    let mut candidates = vec![openrouter_model.to_string()];
    candidates.extend(
        openrouter_fallbacks
            .iter()
            .filter(|fallback| *fallback != openrouter_model)
            .cloned(),
    );
    let mut last_error = anyhow::anyhow!("no OpenRouter model configured");
    for candidate in &candidates {
        let model = format!("open_router::{candidate}");
        let outcome = chat_text(
            client,
            &model,
            request.clone(),
            &options,
            &mut metrics.extraction_calls,
        )
        .await
        .and_then(|text| parse_llm_text(&text))
        .map(|actions| absolute(actions, chunk));
        match outcome {
            Ok(actions) => return Ok(actions),
            Err(error) => {
                warn!(
                    model = %candidate,
                    error = %format!("{error:#}"),
                    "openrouter model failed"
                );
                last_error = error;
            }
        }
    }
    Err(last_error)
}

/// Attach each action's source article, resolved from its chunk-local
/// `sourceIndex`. Values outside the chunk are dropped with a warning.
fn absolute<'a>(actions: Vec<LlmAction>, chunk: &[Job<'a>]) -> Vec<(&'a Article, LlmAction)> {
    actions
        .into_iter()
        .filter_map(|action| {
            let Some(job) = chunk.get(action.source_index) else {
                warn!(
                    index = action.source_index,
                    "extraction result index out of range"
                );
                return None;
            };
            Some((job.item, action))
        })
        .collect()
}

pub(crate) fn parse_llm_text(text: &str) -> Result<Vec<LlmAction>> {
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

/// Bound untrusted LLM strings before they reach validation or the DB.
pub(crate) fn sanitize_action(mut action: LlmAction) -> LlmAction {
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
    action
}

#[allow(dead_code)]
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
        let mut action: LlmAction = serde_json::from_value(json!({
            "establishment": "  X  ",
            "actionType": "inspection",
            "sourceIndex": 0,
            "violations": ["  issue  ", ""],
            "platforms": [" Swiggy ", " "]
        }))
        .unwrap();
        action = sanitize_action(action);
        assert_eq!(action.establishment, "X");
        assert_eq!(action.violations, vec!["issue"]);
        assert_eq!(action.platforms, vec!["swiggy"]);
    }

    #[test]
    fn attaches_source_articles_and_drops_out_of_range() {
        let first = article("first");
        let second = article("second");
        let chunk = [Job { item: &first }, Job { item: &second }];
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
            std::ptr::eq(result[0].0, &second),
            "chunk-local index 1 resolves to the second article"
        );
        assert_eq!(result[0].1.establishment, "B");
        assert_eq!(result[0].1.source_index, 1, "wire index is left untouched");
        assert!(
            std::ptr::eq(result[1].0, &first),
            "chunk-local index 0 resolves to the first article"
        );
        assert_eq!(result[1].1.establishment, "A");
        assert_eq!(result[1].1.source_index, 0, "wire index is left untouched");
    }
}
