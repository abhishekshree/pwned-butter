//! Validated domain types.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use strum::{EnumIter, IntoEnumIterator};

use crate::error::Rejection;
use crate::models::{ActionType, LlmAction, NewsItem, OutletType};
use crate::rules::rules;

/// The only cities this tracker covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, Serialize)]
pub enum CoveredCity {
    Mumbai,
    Pune,
    Thane,
    NaviMumbai,
    Lonavala,
}

impl CoveredCity {
    /// Accepts locality qualifiers ("Mumbai Suburban") but not prefix lookalikes ("Mumbaiwadi").
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

/// An RSS item after enrichment, with its canonical URL resolved.
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

/// A validated record ready for persistence — also the insert DTO.
// #[non_exhaustive] only seals against other crates; this field seals within the crate too.
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, Serialize)]
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
    #[serde(skip)]
    _sealed: (),
}

impl Candidate {
    /// Unknown outlet codes become `None` because the DB column is nullable.
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
        let details = nonempty(action.details.as_deref());

        let config = &rules().quality;
        let lower = establishment.to_lowercase();
        let hay = format!(
            "{lower} {}",
            details.as_deref().unwrap_or("").to_lowercase()
        );
        if lower.starts_with(['\'', '"', '‘', '“']) {
            return Err(Rejection::QuoteLedEstablishment);
        }
        if config.no_action_yet.iter().any(|w| hay.contains(w)) {
            return Err(Rejection::PendingAction);
        }
        if config.product_words.iter().any(|w| lower.contains(w)) {
            return Err(Rejection::ProductNotOutlet);
        }
        if config.regulator_words.iter().any(|w| lower.contains(w)) {
            return Err(Rejection::RegulatorNotOutlet);
        }
        if establishment.split_whitespace().count() > config.max_establishment_words {
            return Err(Rejection::HeadlineFragment);
        }
        if details
            .as_deref()
            .is_none_or(|d| d.chars().count() < config.min_details_chars)
        {
            return Err(Rejection::NoSubstantiveDetails);
        }

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
            details,
            platforms: action.platforms,
            source_url: article.url.clone(),
            source_publisher: nonempty(article.source.as_deref()),
            source_headline: Some(article.title.clone()),
            published_at: article.published,
            _sealed: (),
        })
    }
}

fn nonempty(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;
    use crate::models::{ActionType, LlmAction};

    const FULL: &str = "licence suspended over cockroach infestation found during inspection";

    fn article() -> Article {
        Article {
            title: "source headline".into(),
            url: "https://example.test/1".into(),
            source: None,
            published: None,
            snippet: None,
        }
    }

    fn candidate(est: &str, details: Option<&str>) -> Result<Candidate, Rejection> {
        let action = LlmAction {
            establishment: est.into(),
            city: Some("Mumbai".into()),
            action_date: Some("2026-08-11".into()),
            details: details.map(str::to_string),
            ..Default::default()
        };
        Candidate::try_new(action, &article())
    }

    #[test_case("Domino's Vile Parle", Some(FULL), Ok(()); "complete record")]
    #[test_case("IIT Bombay Hostel Canteen (Hostels 12, 13 & 14)", Some(FULL), Ok(()); "long genuine name")]
    #[test_case("Domino's Vile Parle", Some("12345678901234567890"), Ok(()); "details at floor")]
    #[test_case("'Popularity was never my goal'", Some(FULL), Err(Rejection::QuoteLedEstablishment); "quote-led name")]
    #[test_case("Popular Re 1 Ayurvedic Digestive Tablet Fails Maharashtra FDA Quality Test", Some(FULL), Err(Rejection::ProductNotOutlet); "product name")]
    #[test_case("Maharashtra FDA rolls out year", Some(FULL), Err(Rejection::RegulatorNotOutlet); "regulator name")]
    #[test_case("FDA awaiting response from restaurants at Mumbai Cricket Association premises", Some(FULL), Err(Rejection::PendingAction); "awaiting response")]
    #[test_case("FDA awaits replies from restaurants at MCA premises", Some(FULL), Err(Rejection::PendingAction); "awaits replies")]
    #[test_case("Maharashtra FDA awaits replies from five MCA restaurants over licensing violations", Some(FULL), Err(Rejection::PendingAction); "awaits replies long")]
    #[test_case("Popular Re 1 Digestive Item Fails Maharashtra Quality Test Today Here", Some(FULL), Err(Rejection::HeadlineFragment); "headline fragment")]
    #[test_case("Domino's Vile Parle", None, Err(Rejection::NoSubstantiveDetails); "no details")]
    #[test_case("Domino's Vile Parle", Some("1234567890123456789"), Err(Rejection::NoSubstantiveDetails); "short details")]
    #[test_case("MCA BKC Club", Some("FDA has issued a notice and seeks explanation from the club within seven days"), Err(Rejection::PendingAction); "seeks explanation")]
    #[test_case("   ", Some(FULL), Err(Rejection::NoEstablishment); "blank name")]
    fn gates_junk_shapes(est: &str, details: Option<&str>, expected: Result<(), Rejection>) {
        assert_eq!(candidate(est, details).map(|_| ()), expected, "{est}");
    }

    #[test]
    fn keeps_inspection_without_violations_when_detailed() {
        let action = LlmAction {
            establishment: "Bombay High Court Canteen".into(),
            city: Some("Mumbai".into()),
            action_date: Some("2026-08-11".into()),
            action_type: ActionType::Inspection,
            details: Some(
                "FDA team inspected canteens inside the Bombay High Court premises following queries from the court.".into(),
            ),
            ..Default::default()
        };
        assert!(Candidate::try_new(action, &article()).is_ok());
    }
}
