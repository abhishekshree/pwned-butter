use anyhow::{Result as AnyResult, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use super::extract::strip_code_fences;

pub(crate) const TRIAGE_PROMPT: &str = include_str!("../prompts/triage.txt");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Verdict {
    pub(crate) source_index: usize,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

pub(crate) fn triage_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "sourceIndex": {"type": "integer"},
                "reason": {"type": "string"}
            },
            "required": ["sourceIndex", "reason"]
        }
    })
}

pub(crate) fn parse_verdicts(text: &str) -> AnyResult<Vec<Verdict>> {
    let text = strip_code_fences(text);
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|error| anyhow!("triage returned invalid JSON: {error}"))?;
    let raw = parsed
        .as_array()
        .ok_or_else(|| anyhow!("triage response is not a JSON array"))?;
    Ok(raw
        .iter()
        .filter_map(|value| match serde_json::from_value(value.clone()) {
            Ok(verdict) => Some(verdict),
            Err(error) => {
                warn!(error = %error, "dropping invalid triage verdict");
                None
            }
        })
        .collect())
}

#[cfg(test)]
#[path = "triage_test.rs"]
mod tests;
