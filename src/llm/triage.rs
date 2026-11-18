use anyhow::{Result as AnyResult, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::domain::Article;

use super::strip_code_fences;

pub(crate) const TRIAGE_PROMPT: &str = include_str!("../prompts/triage.txt");

/// A relevance-filtered batch item. Provenance is the `&Article` itself, so
/// no positional index has to survive chunking.
pub(crate) struct Job<'a> {
    pub(crate) item: &'a Article,
}

/// One qualifying item as returned by the triage model. Triage is a pure
/// relevance filter: it carries only the source position and a short reason.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Verdict {
    pub(crate) source_index: usize,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

/// Response format for the model. Triage emits only a source index and a
/// short reason, so the schema has exactly those two fields.
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
mod tests {
    use super::*;

    #[test]
    fn parses_only_valid_verdicts() {
        let text = r#"[{"sourceIndex":3,"reason":"licence suspended over pests"},
            {"sourceIndex":4},
            {"reason":"no index"},
            {"sourceIndex":"bad","reason":"nope"}]"#;
        let verdicts = parse_verdicts(text).unwrap();
        assert_eq!(
            verdicts.len(),
            2,
            "records without a numeric sourceIndex are dropped"
        );
        assert_eq!(verdicts[0].source_index, 3);
        assert_eq!(
            verdicts[0].reason.as_deref(),
            Some("licence suspended over pests")
        );
        assert_eq!(verdicts[1].source_index, 4);
        assert_eq!(verdicts[1].reason, None, "reason defaults to None");
    }
}
