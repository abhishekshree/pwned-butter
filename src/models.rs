use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub url: String,
    pub source: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub snippet: Option<String>,
}

/// Lowercased title, snippet and publisher, for keyword scoring and triage.
impl NewsItem {
    pub fn haystack(&self) -> String {
        format!(
            "{} {} {}",
            self.title,
            self.snippet.as_deref().unwrap_or(""),
            self.source.as_deref().unwrap_or("")
        )
        .to_lowercase()
    }
}

/// Codes come from `strum`'s `snake_case` attribute, so the DB value, serde
/// name and prompt name cannot drift apart.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumString,
    Display,
    EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ActionType {
    LicenceSuspension,
    StopBusiness,
    ImprovementNotice,
    Sealing,
    Seizure,
    #[default]
    Inspection,
    Reopened,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString, Display, EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OutletType {
    Restaurant,
    CloudKitchen,
    QuickCommerce,
    Warehouse,
    Dhaba,
    Hotel,
    Bakery,
    Club,
    Mess,
    Dairy,
    StreetVendor,
    Other,
}

impl OutletType {
    /// Unknown strings fall back to `Other` at the trust boundary.
    pub fn from_code(s: &str) -> Self {
        s.trim().parse().unwrap_or(Self::Other)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmAction {
    pub establishment: String,
    #[serde(default)]
    pub area: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub brand: Option<String>,
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default, alias = "outlet_type")]
    pub outlet_type: Option<String>,
    #[serde(alias = "action_type")]
    pub action_type: ActionType,
    #[serde(default, alias = "action_date")]
    pub action_date: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub violations: Vec<String>,
    #[serde(default)]
    pub compliance_score: Option<i32>,
    #[serde(default, alias = "fssai_number")]
    pub fssai_number: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub platforms: Vec<String>,
    #[serde(alias = "source_index")]
    pub source_index: usize,
}

fn deserialize_string_vec<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(de)?.unwrap_or_default())
}

impl LlmAction {
    /// Minimal record for the deterministic rule fallback: only what keyword
    /// rules can know. Everything else stays empty for downstream defaults.
    pub fn minimal(
        establishment: String,
        action_type: ActionType,
        source_index: usize,
        details: Option<String>,
    ) -> Self {
        Self {
            establishment,
            action_type,
            source_index,
            details,
            ..Default::default()
        }
    }
}

pub fn nonempty(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

pub fn coerce_action_date(raw: Option<&str>, published: Option<DateTime<Utc>>) -> NaiveDate {
    if let Some(s) = raw.and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()) {
        return s;
    }
    published
        .map(|d| d.date_naive())
        .unwrap_or_else(|| Utc::now().date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn action_type_round_trips() {
        for t in ActionType::iter() {
            let code = t.to_string();
            assert_eq!(code.parse::<ActionType>().unwrap(), t, "{code} round-trips");
        }
        assert!(
            "suspended".parse::<ActionType>().is_err(),
            "unknown code is rejected"
        );
    }

    #[test]
    fn outlet_type_round_trips() {
        for t in OutletType::iter() {
            let code = t.to_string();
            assert_eq!(OutletType::from_code(&code), t, "{code} round-trips");
        }
        assert_eq!(OutletType::from_code("stand"), OutletType::Other);
        assert_eq!(OutletType::from_code(""), OutletType::Other);
    }

    #[test]
    fn date_trusts_reported_value() {
        assert_eq!(
            coerce_action_date(Some("2026-08-11"), None),
            NaiveDate::from_ymd_opt(2026, 8, 11).unwrap()
        );
        let now = Utc::now();
        assert_eq!(coerce_action_date(None, Some(now)), now.date_naive());
        assert_eq!(
            coerce_action_date(Some("not a date"), Some(now)),
            now.date_naive()
        );
    }
}
