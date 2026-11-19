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

/// Marks a value that has passed validation; construct only through `new`.
pub struct Sanitized<T>(T);

impl<T> Sanitized<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(value)
    }

    pub(crate) fn into_inner(self) -> T {
        self.0
    }
}
