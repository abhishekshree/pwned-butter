//! Validated domain types.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use strum::{EnumIter, IntoEnumIterator};

use crate::error::Rejection;
use crate::models::{ActionType, LlmAction, NewsItem, OutletType, Sanitized};
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
#[derive(Debug, Clone, Serialize)]
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
    pub fn try_new(action: Sanitized<LlmAction>, article: &Article) -> Result<Self, Rejection> {
        let action = action.into_inner();
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

/// Outlet identity used to detect the same action across reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventKey {
    pub establishment: String,
}

impl EventKey {
    pub fn from_name(name: &str) -> Self {
        Self {
            establishment: name.to_string(),
        }
    }

    pub fn from_candidate(c: &Candidate) -> Self {
        Self::from_name(&c.establishment)
    }

    /// Exact, containment for 6+ char names, or token/acronym match with a 3+ char word in common.
    pub fn matches(&self, other: &Self) -> bool {
        // Venue words that vary between reports of the same outlet.
        const GENERIC_WORDS: [&str; 7] = [
            "club",
            "facility",
            "canteen",
            "restaurant",
            "hotel",
            "outlet",
            "kitchen",
        ];

        fn tokens(s: &str) -> Vec<&str> {
            s.split(|c: char| !c.is_alphanumeric())
                .filter(|t| !t.is_empty() && !GENERIC_WORDS.contains(t))
                .collect()
        }

        // "mca" = initials of the consecutive run "mumbai cricket association".
        fn is_acronym_of(acro: &str, words: &[&str]) -> bool {
            !acro.is_empty()
                && acro.len() <= words.len()
                && acro.chars().zip(words).all(|(c, w)| w.starts_with(c))
        }

        fn token_matches(t: &str, words: &[&str]) -> bool {
            words.contains(&t)
                || (0..=words.len().saturating_sub(t.len())).any(|i| is_acronym_of(t, &words[i..]))
        }

        let norm = |s: &str| -> String {
            s.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (a, b) = (norm(&self.establishment), norm(&other.establishment));
        if a == b {
            return true;
        }
        // Short-name containment is too loose ("KFC" everywhere).
        if a.len().min(b.len()) >= 6 && (a.contains(&b) || b.contains(&a)) {
            return true;
        }
        let side_matches = |from: &[&str], to: &[&str]| {
            from.iter().map(|t| t.len()).sum::<usize>() >= 6
                && from.iter().all(|t| token_matches(t, to))
                && from.iter().any(|t| t.len() >= 3 && to.contains(t))
        };
        let (ta, tb) = (tokens(&a), tokens(&b));
        side_matches(&ta, &tb) || side_matches(&tb, &ta)
    }
}
