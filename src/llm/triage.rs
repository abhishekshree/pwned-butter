use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::models::{ActionType, LlmAction, NewsItem};

use super::strip_code_fences;

pub(crate) const TRIAGE_PROMPT: &str = include_str!("../prompts/triage.txt");

/// A relevance-filtered batch item; `orig` indexes the caller's `items` slice
/// so verdicts still address the original input after chunking.
pub(crate) struct Job<'a> {
    pub(crate) orig: usize,
    pub(crate) item: &'a NewsItem,
}

/// One qualifying item as returned by the triage model.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Verdict {
    pub(crate) source_index: usize,
    pub(crate) action_type: ActionType,
    #[serde(default)]
    pub(crate) city: Option<String>,
    #[serde(default)]
    pub(crate) establishment: Option<String>,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

pub(crate) fn parse_verdicts(text: &str) -> Result<Vec<Verdict>> {
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

/// Minimal record for the rule-free fallback: only what the triage model
/// already told us. The quality gate and `build_rows` still own the final
/// decision.
pub(crate) fn minimal_from_verdict(v: &Verdict, item: &NewsItem) -> LlmAction {
    LlmAction {
        establishment: v.establishment.clone().unwrap_or_default(),
        city: v.city.clone(),
        action_type: v.action_type,
        source_index: v.source_index,
        details: item.snippet.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_valid_verdicts() {
        let text = r#"[{"sourceIndex":3,"actionType":"licence_suspension",
            "city":"Mumbai","establishment":"Domino's Vile Parle",
            "reason":"licence suspended over pests"},
            {"sourceIndex":9,"actionType":"nonsense"}]"#;
        let verdicts = parse_verdicts(text).unwrap();
        assert_eq!(verdicts.len(), 1, "invalid action type is dropped");
        assert_eq!(verdicts[0].source_index, 3);
        assert_eq!(verdicts[0].action_type, ActionType::LicenceSuspension);
    }

    #[test]
    fn empty_array_is_valid() {
        assert!(parse_verdicts("[]").unwrap().is_empty());
        assert!(parse_verdicts("nope").is_err());
    }

    #[test]
    fn prompt_names_every_action_code() {
        use strum::IntoEnumIterator;

        for code in ActionType::iter().map(|t| t.to_string()) {
            assert!(
                TRIAGE_PROMPT.contains(&code),
                "triage prompt missing {code}"
            );
        }
    }
}
