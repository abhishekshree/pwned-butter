use std::time::Duration;

use anyhow::{Context, Result};
use genai::Client;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, JsonSpec};
use genai::resolver::AuthData;
use serde_json::{Value, json};

use crate::models::LlmAction;

use super::triage::Job;
use super::{strip_code_fences, system_prompt, truncate};

pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
pub(crate) const OPENROUTER_FALLBACKS: &[&str] = &["inclusionai/ling-3.2-flash:free"];
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

async fn chat_text(
    client: &Client,
    model: &str,
    request: ChatRequest,
    options: &ChatOptions,
    calls: &mut usize,
) -> Result<String> {
    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        *calls += 1;
        match client
            .exec_chat(model, request.clone(), Some(options))
            .await
        {
            Ok(response) => {
                if let Some(text) = response.into_first_text() {
                    if !text.trim().is_empty() {
                        return Ok(text);
                    }
                    last_error = Some(anyhow::anyhow!("genai returned empty response"));
                } else {
                    last_error = Some(anyhow::anyhow!("genai response had no text"));
                }
            }
            Err(error) => last_error = Some(anyhow::anyhow!("genai request failed: {error}")),
        }
        if attempt + 1 < MAX_ATTEMPTS {
            tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("genai request failed")))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn extract_batch(
    api_key: &str,
    model: &str,
    chunk: &[Job<'_>],
    calls: &mut usize,
    delivery: bool,
    openrouter_key: Option<&str>,
    openrouter_model: &str,
    openrouter_fallbacks: &[String],
) -> Result<Vec<LlmAction>> {
    let client = client(api_key, openrouter_key)?;
    let user = serde_json::to_string(&json!({
        "items": chunk.iter().map(|job| job.item).collect::<Vec<_>>()
    }))
    .context("serialize news batch")?;
    let request =
        ChatRequest::from_system(system_prompt(delivery)).append_message(ChatMessage::user(user));
    let options = json_options(16_384, json!({"type": "array"}));

    let mut gemini_models = vec![model];
    for fallback in crate::llm::GEMINI_FALLBACK_MODELS {
        if *fallback != model {
            gemini_models.push(*fallback);
        }
    }
    for candidate in gemini_models {
        match chat_text(&client, candidate, request.clone(), &options, calls).await {
            Ok(text) => match parse_llm_text(&text) {
                Ok(actions) => return Ok(remap(actions, chunk)),
                Err(error) => {
                    eprintln!("gemini model {candidate} returned invalid output: {error:#}")
                }
            },
            Err(error) => eprintln!("gemini model {candidate} failed: {error:#}"),
        }
    }

    eprintln!("all gemini models failed; falling back to openrouter");
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
    let mut last_error = None;
    for candidate in candidates {
        let model = format!("open_router::{candidate}");
        match chat_text(&client, &model, request.clone(), &options, calls).await {
            Ok(text) => match parse_llm_text(&text) {
                Ok(actions) => return Ok(remap(actions, chunk)),
                Err(error) => last_error = Some(error),
            },
            Err(error) => {
                eprintln!("openrouter model {candidate} failed: {error:#}");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("all openrouter models failed")))
}

pub(crate) async fn collapse_once(
    api_key: &str,
    model: &str,
    rows: &[crate::db::ActionInsert],
    recent: &[crate::db::RecentEvent],
    calls: &mut usize,
) -> Result<Vec<usize>> {
    let record = |id: String,
                  establishment: &str,
                  action_type: &str,
                  action_date: chrono::NaiveDate,
                  city: &Option<String>,
                  area: &Option<String>| {
        json!({"id": id, "establishment": establishment, "actionType": action_type,
            "actionDate": action_date.to_string(), "city": city, "area": area})
    };
    let new_items: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            record(
                format!("N{i}"),
                &row.establishment,
                &row.action_type,
                row.action_date,
                &row.city,
                &row.area,
            )
        })
        .collect();
    let known_items: Vec<Value> = recent
        .iter()
        .enumerate()
        .map(|(i, event)| {
            record(
                format!("K{i}"),
                &event.establishment,
                &event.action_type,
                event.action_date,
                &event.city,
                &event.area,
            )
        })
        .collect();
    let request = ChatRequest::from_system(include_str!("../prompts/dedupe.txt")).append_message(
        ChatMessage::user(serde_json::to_string(
            &json!({"new": new_items, "known": known_items}),
        )?),
    );
    let options = json_options(
        2048,
        json!({"type": "object", "properties": {"groups": {"type": "array"}}}),
    );
    let client = client(api_key, None)?;
    let text = chat_text(&client, model, request, &options, calls).await?;
    Ok(drops_from_groups(&parse_groups(&text)?, rows.len()))
}

fn remap(actions: Vec<LlmAction>, chunk: &[Job<'_>]) -> Vec<LlmAction> {
    actions
        .into_iter()
        .filter_map(|mut action| {
            let job = chunk.get(action.source_index)?;
            action.source_index = job.orig;
            Some(action)
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
            Err(error) => eprintln!(
                "dropping invalid LLM record: {error}: {}",
                truncate(&value.to_string(), 200)
            ),
        }
    }
    Ok(actions)
}

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

pub(crate) fn parse_groups(text: &str) -> Result<Vec<Vec<String>>> {
    let parsed: Value = serde_json::from_str(&strip_code_fences(text.trim())).map_err(|error| {
        anyhow::anyhow!(
            "dupe response invalid JSON: {error}; body: {}",
            truncate(text, 300)
        )
    })?;
    let groups = parsed
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("dupe response missing \"groups\" array"))?;
    Ok(groups
        .iter()
        .filter_map(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .filter(|group| !group.is_empty())
        .collect())
}

pub(crate) fn drops_from_groups(groups: &[Vec<String>], n_new: usize) -> Vec<usize> {
    let mut drops = std::collections::BTreeSet::new();
    for group in groups {
        let mut news: Vec<usize> = group
            .iter()
            .filter_map(|id| id.strip_prefix('N').and_then(|n| n.parse().ok()))
            .filter(|index| *index < n_new)
            .collect();
        news.sort_unstable();
        news.dedup();
        if news.is_empty() {
            continue;
        }
        let touches_known = group.iter().any(|id| id.starts_with('K'));
        for index in news.iter().skip(usize::from(!touches_known)) {
            drops.insert(*index);
        }
    }
    drops.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActionType;

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
    fn duplicate_groups_drop_re_reports_and_keep_lowest_new_index() {
        assert_eq!(
            drops_from_groups(
                &[
                    vec!["N3".into(), "N1".into(), "N7".into()],
                    vec!["K4".into(), "N2".into()]
                ],
                10
            ),
            vec![2, 3, 7]
        );
    }
}
