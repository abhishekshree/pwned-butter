use chrono::{DateTime, Utc};
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

/// Lowercase search text shared by RSS items and enriched articles.
pub fn haystack(title: &str, snippet: Option<&str>, source: Option<&str>) -> String {
    format!(
        "{} {} {}",
        title,
        snippet.unwrap_or(""),
        source.unwrap_or("")
    )
    .to_lowercase()
}

impl NewsItem {
    pub fn haystack(&self) -> String {
        haystack(&self.title, self.snippet.as_deref(), self.source.as_deref())
    }
}

/// strum's snake_case keeps the DB value, serde name and prompt name in sync.
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
    Dhaba,
    Hotel,
    Bakery,
    Club,
    Mess,
    Dairy,
    StreetVendor,
    Other,
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

/// An LLM action whose strings and lists have been bounded. The bounds live in
/// the constructor so no module can wrap raw model output and claim it is clean.
pub struct SanitizedAction(LlmAction);

impl SanitizedAction {
    pub(crate) fn new(mut value: LlmAction) -> Self {
        value.establishment = clamp(value.establishment, 200);
        clamp_opt(&mut value.area, 120);
        clamp_opt(&mut value.city, 120);
        clamp_opt(&mut value.brand, 120);
        clamp_opt(&mut value.operator, 200);
        clamp_opt(&mut value.fssai_number, 64);
        clamp_opt(&mut value.details, 2000);
        value.violations = value
            .violations
            .into_iter()
            .map(|v| clamp(v, 300))
            .filter(|v| !v.is_empty())
            .take(5)
            .collect();
        value.platforms = value
            .platforms
            .into_iter()
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty())
            .take(6)
            .collect();
        Self(value)
    }

    pub(crate) fn into_inner(self) -> LlmAction {
        self.0
    }
}

fn clamp(value: String, max: usize) -> String {
    let value = value.trim();
    if value.chars().count() > max {
        value.chars().take(max).collect()
    } else {
        value.to_string()
    }
}

fn clamp_opt(field: &mut Option<String>, max: usize) {
    if let Some(value) = field.take() {
        *field = Some(clamp(value, max));
    }
}
