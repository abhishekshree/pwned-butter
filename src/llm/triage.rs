use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use strum::IntoEnumIterator;
use tracing::warn;

use crate::models::{ActionType, LlmAction, NewsItem};
use crate::rules::rules;
use crate::scrape;

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
    #[serde(default)]
    pub(crate) evidence: Option<String>,
    #[serde(default)]
    pub(crate) confidence: Option<f32>,
}

/// Display casing for the city labels the model returns. The insert path
/// still matches case-insensitively via `in_scope`.
fn city_labels() -> Vec<String> {
    scrape::IN_SCOPE_CITIES
        .iter()
        .map(|city| {
            city.split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Response format for the model. The schema is generated from the same enums
/// and city list the insert path uses, so they cannot drift apart.
pub(crate) fn triage_schema() -> Value {
    let actions: Vec<String> = ActionType::iter().map(|t| t.to_string()).collect();
    let mut cities: Vec<Value> = city_labels().into_iter().map(Value::String).collect();
    cities.push(Value::Null);
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "sourceIndex": {"type": "integer"},
                "actionType": {"type": "string", "enum": actions},
                "city": {"type": ["string", "null"], "enum": cities},
                "establishment": {"type": ["string", "null"]},
                "reason": {"type": "string"},
                "evidence": {"type": "string"},
                "confidence": {"type": "number"}
            },
            "required": ["sourceIndex", "actionType", "reason", "evidence", "confidence"]
        }
    })
}

/// All prose lives in `triage.txt`. This only fills the placeholders with
/// values from the deterministic gate, so the two cannot drift apart.
pub(crate) fn build_system_prompt() -> String {
    let quality = &rules().quality;
    TRIAGE_PROMPT
        .replace(
            "{{MAX_ESTABLISHMENT_WORDS}}",
            &quality.max_establishment_words.to_string(),
        )
        .replace(
            "{{MIN_DETAILS_CHARS}}",
            &quality.min_details_chars.to_string(),
        )
        .replace("{{CITIES}}", &city_labels().join(", "))
        .replace("{{NO_ACTION_YET}}", &quality.no_action_yet.join(", "))
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

/// The model must quote the text it relied on; verify the quote is really
/// present instead of trusting a confident-sounding invention.
pub(crate) fn evidence_is_supplied(item: &NewsItem, evidence: Option<&str>) -> bool {
    let Some(evidence) = evidence else {
        return true;
    };
    let evidence = evidence.trim();
    if evidence.chars().count() < 8 {
        return false;
    }
    let hay = format!("{} {}", item.title, item.snippet.as_deref().unwrap_or("")).to_lowercase();
    hay.contains(&evidence.to_lowercase())
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
    use chrono::{TimeZone, Utc};

    fn item(title: &str, snippet: &str) -> NewsItem {
        NewsItem {
            title: title.into(),
            url: "https://example.test/1".into(),
            source: Some("Test Press".into()),
            published: Some(Utc.with_ymd_and_hms(2026, 11, 18, 9, 0, 0).unwrap()),
            snippet: Some(snippet.into()),
        }
    }

    #[test]
    fn parses_only_valid_verdicts() {
        let text = r#"[{"sourceIndex":3,"actionType":"licence_suspension",
            "city":"Mumbai","establishment":"Domino's Vile Parle",
            "reason":"licence suspended over pests",
            "evidence":"licence suspended over pests","confidence":0.9},
            {"sourceIndex":9,"actionType":"nonsense"}]"#;
        let verdicts = parse_verdicts(text).unwrap();
        assert_eq!(verdicts.len(), 1, "invalid action type is dropped");
        assert_eq!(verdicts[0].source_index, 3);
        assert_eq!(verdicts[0].action_type, ActionType::LicenceSuspension);
        assert_eq!(verdicts[0].confidence, Some(0.9));
    }

    #[test]
    fn rejects_evidence_absent_from_the_item() {
        let story = item(
            "FDA licence suspended in Pune",
            "Chetana had its licence suspended.",
        );
        assert!(evidence_is_supplied(&story, Some("licence suspended")));
        assert!(!evidence_is_supplied(
            &story,
            Some("cockroaches everywhere")
        ));
        assert!(evidence_is_supplied(&story, None));
    }

    #[test]
    fn prompt_carries_the_gate_and_examples() {
        let prompt = build_system_prompt();
        assert!(
            prompt.contains("at most 9 words"),
            "word cap is passed to the model"
        );
        assert!(!prompt.contains("{{"), "no placeholders left unfilled");
        assert!(
            TRIAGE_PROMPT.contains("Mumbra sweet shop"),
            "negative example is included"
        );
        assert!(
            TRIAGE_PROMPT.contains("Chetana"),
            "positive roundup example is included"
        );
    }

    #[test]
    fn schema_uses_the_live_vocabulary() {
        let schema = triage_schema().to_string();
        for code in ActionType::iter().map(|t| t.to_string()) {
            assert!(schema.contains(&code), "schema missing {code}");
        }
        assert!(
            schema.contains("Navi Mumbai"),
            "schema missing covered city"
        );
    }
}
