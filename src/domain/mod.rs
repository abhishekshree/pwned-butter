//! Validated domain types. Raw RSS items and untrusted LLM output do not pass
//! beyond this module without becoming one of these.

use chrono::{DateTime, NaiveDate, Utc};
use strum::{EnumIter, IntoEnumIterator};

use crate::error::Rejection;
use crate::models::{ActionType, LlmAction, NewsItem, OutletType, nonempty};

/// Cities this tracker covers, and only these. Replaces the string-prefix
/// `IN_SCOPE_CITIES` check that could drift from the prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub enum CoveredCity {
    Mumbai,
    Pune,
    Thane,
    NaviMumbai,
    Lonavala,
}

impl CoveredCity {
    /// Accepts a city with a locality qualifier ("Mumbai Suburban") but not a
    /// different place that merely starts with the same characters. The names
    /// come from `as_str`, so there is only one copy of each city string.
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw.trim().to_lowercase();
        Self::iter()
            .filter(|city| {
                let name = city.as_str().to_lowercase();
                value
                    .strip_prefix(&name)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
            })
            .max_by_key(|city| city.as_str().len())
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mumbai => "Mumbai",
            Self::Pune => "Pune",
            Self::Thane => "Thane",
            Self::NaviMumbai => "Navi Mumbai",
            Self::Lonavala => "Lonavala",
        }
    }
}

/// An RSS item after enrichment. Its `url` is the canonical publisher URL
/// when the fetch succeeded, otherwise the original Google News URL — but it
/// is set exactly once, by `news::enrich`, and never mutated afterwards.
#[derive(Debug, Clone)]
pub struct Article {
    pub title: String,
    pub url: String,
    pub source: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub snippet: Option<String>,
}

impl Article {
    pub fn new(item: NewsItem, url: String, snippet: Option<String>) -> Self {
        Self {
            title: item.title,
            url,
            source: item.source,
            published: item.published,
            snippet,
        }
    }

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

/// A validated record ready for persistence. This is also the insert DTO,
/// replacing `db::ActionInsert` so the same shape cannot drift in two places.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub establishment: String,
    pub area: Option<String>,
    pub city: CoveredCity,
    pub brand: Option<String>,
    pub operator: Option<String>,
    pub outlet_type: Option<OutletType>,
    pub action_type: ActionType,
    pub action_date: NaiveDate,
    pub violations: Vec<String>,
    pub compliance_score: Option<i32>,
    pub fssai_number: Option<String>,
    pub details: Option<String>,
    pub platforms: Vec<String>,
    pub source_url: String,
    pub source_publisher: Option<String>,
    pub source_headline: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

impl Candidate {
    /// Structural validation only. Business-quality rules live in `quality`.
    /// Unknown outlet codes become `None` (the DB column is nullable) instead
    /// of the `Other` error sink; unrecognised action codes are already
    /// rejected by serde when the LLM response is parsed.
    pub fn try_new(action: LlmAction, article: &Article) -> Result<Self, Rejection> {
        let establishment =
            nonempty(Some(action.establishment.as_str())).ok_or(Rejection::NoEstablishment)?;
        let city = CoveredCity::parse(action.city.as_deref().unwrap_or(""))
            .ok_or(Rejection::OutOfScopeCity)?;
        let action_date = action
            .action_date
            .as_deref()
            .and_then(|raw| NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").ok())
            .or_else(|| article.published.map(|date| date.date_naive()))
            .ok_or(Rejection::NoActionDate)?;

        Ok(Self {
            establishment,
            area: nonempty(action.area.as_deref()),
            city,
            brand: nonempty(action.brand.as_deref()),
            operator: nonempty(action.operator.as_deref()),
            outlet_type: action
                .outlet_type
                .as_deref()
                .and_then(|raw| raw.trim().parse::<OutletType>().ok()),
            action_type: action.action_type,
            action_date,
            violations: action.violations,
            compliance_score: action
                .compliance_score
                .filter(|score| (0..=100).contains(score)),
            fssai_number: nonempty(action.fssai_number.as_deref()),
            details: nonempty(action.details.as_deref()),
            platforms: action.platforms,
            source_url: article.url.clone(),
            source_publisher: nonempty(article.source.as_deref()),
            source_headline: Some(article.title.clone()),
            published_at: article.published,
        })
    }
}

/// The dashboard's three display states, derived from the closed action enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Suspended,
    Reopened,
    Active,
}

impl From<ActionType> for Status {
    fn from(action: ActionType) -> Self {
        match action {
            ActionType::Reopened => Self::Reopened,
            ActionType::ImprovementNotice | ActionType::Inspection | ActionType::Seizure => {
                Self::Active
            }
            ActionType::LicenceSuspension | ActionType::StopBusiness | ActionType::Sealing => {
                Self::Suspended
            }
        }
    }
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Suspended => "suspended",
            Self::Reopened => "reopened",
            Self::Active => "active",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CoveredCity, Status};
    use crate::models::ActionType;

    #[test]
    fn covered_cities_accept_qualifiers_and_reject_prefixes() {
        assert_eq!(
            CoveredCity::parse("Mumbai Suburban"),
            Some(CoveredCity::Mumbai)
        );
        assert_eq!(
            CoveredCity::parse("navi mumbai"),
            Some(CoveredCity::NaviMumbai)
        );
        assert_eq!(CoveredCity::parse("Panvel"), None);
        assert_eq!(CoveredCity::parse("Mumbaiwadi"), None);
    }

    #[test]
    fn status_maps_every_action_exhaustively() {
        assert_eq!(Status::from(ActionType::Reopened), Status::Reopened);
        assert_eq!(Status::from(ActionType::Inspection), Status::Active);
        assert_eq!(Status::from(ActionType::Sealing), Status::Suspended);
    }
}
