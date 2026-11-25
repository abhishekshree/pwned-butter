//! Validated domain types.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use strum::{EnumIter, IntoEnumIterator};

use crate::error::Rejection;
use crate::models::{ActionType, NewsItem, OutletType, SanitizedAction, haystack};
use crate::rules::rules;

/// The only cities this tracker covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, Serialize)]
pub enum CoveredCity {
    Mumbai,
    Pune,
    Thane,
    #[serde(rename = "Navi Mumbai")]
    NaviMumbai,
    Lonavala,
}

impl CoveredCity {
    /// Accepts locality qualifiers ("Mumbai Suburban"), punctuation variants
    /// ("Mumbai.", "Navi-Mumbai") and legacy local JSON ("NaviMumbai"), but
    /// not prefix lookalikes ("Mumbaiwadi").
    pub fn parse(raw: &str) -> Option<Self> {
        let compact = |value: &str| {
            value
                .to_lowercase()
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
        };
        let value = compact(raw);
        Self::iter()
            .filter(|city| {
                let name = compact(city.as_str());
                value
                    .strip_prefix(&name)
                    .is_some_and(|rest| rest.is_empty() || rest == "suburban")
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
        haystack(&self.title, self.snippet.as_deref(), self.source.as_deref())
    }
}

/// A validated record ready for persistence — also the insert DTO. Private
/// fields are the construction boundary: only `try_new` can build one.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    establishment: String,
    area: Option<String>,
    city: CoveredCity,
    brand: Option<String>,
    operator: Option<String>,
    outlet_type: Option<OutletType>,
    action_type: ActionType,
    action_date: NaiveDate,
    violations: Vec<String>,
    compliance_score: Option<i32>,
    fssai_number: Option<String>,
    details: Option<String>,
    platforms: Vec<String>,
    source_url: String,
    source_publisher: Option<String>,
    source_headline: Option<String>,
    published_at: Option<DateTime<Utc>>,
}

impl Candidate {
    pub fn establishment(&self) -> &str {
        &self.establishment
    }

    pub fn area(&self) -> Option<&str> {
        self.area.as_deref()
    }

    pub fn city(&self) -> CoveredCity {
        self.city
    }

    pub fn brand(&self) -> Option<&str> {
        self.brand.as_deref()
    }

    pub fn operator(&self) -> Option<&str> {
        self.operator.as_deref()
    }

    pub fn outlet_type(&self) -> Option<OutletType> {
        self.outlet_type
    }

    pub fn action_type(&self) -> ActionType {
        self.action_type
    }

    pub fn action_date(&self) -> NaiveDate {
        self.action_date
    }

    pub fn violations(&self) -> &[String] {
        &self.violations
    }

    pub fn compliance_score(&self) -> Option<i32> {
        self.compliance_score
    }

    pub fn fssai_number(&self) -> Option<&str> {
        self.fssai_number.as_deref()
    }

    pub fn details(&self) -> Option<&str> {
        self.details.as_deref()
    }

    pub fn platforms(&self) -> &[String] {
        &self.platforms
    }

    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    pub fn source_publisher(&self) -> Option<&str> {
        self.source_publisher.as_deref()
    }

    pub fn source_headline(&self) -> Option<&str> {
        self.source_headline.as_deref()
    }

    pub fn published_at(&self) -> Option<DateTime<Utc>> {
        self.published_at
    }

    /// Re-validates an LLM-supplied canonical name before replacing the outlet name.
    pub(crate) fn set_establishment(&mut self, name: String) -> Result<(), Rejection> {
        let name = nonempty(Some(name.as_str())).ok_or(Rejection::NoEstablishment)?;
        check_establishment(&name, self.details())?;
        self.establishment = name;
        Ok(())
    }

    /// Unknown outlet codes become `None` because the DB column is nullable.
    pub fn try_new(action: SanitizedAction, article: &Article) -> Result<Self, Rejection> {
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

        check_establishment(&establishment, details.as_deref())?;
        if details
            .as_deref()
            .is_none_or(|d| d.chars().count() < rules().quality.min_details_chars)
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
        })
    }
}

fn nonempty(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

/// Shared by first extraction and the LLM alias-collapse rewrite, so a
/// canonical name cannot bypass the gates that guard the original name.
fn check_establishment(name: &str, details: Option<&str>) -> Result<(), Rejection> {
    let config = &rules().quality;
    let lower = name.to_lowercase();
    let hay = format!("{lower} {}", details.unwrap_or("").to_lowercase());
    if name.starts_with(['\'', '"', '‘', '“']) {
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
    if name.chars().count() > 200
        || name.split_whitespace().count() > config.max_establishment_words
    {
        return Err(Rejection::HeadlineFragment);
    }
    Ok(())
}

/// Venue words that vary between reports of the same outlet.
const GENERIC_VENUE_WORDS: [&str; 7] = [
    "club",
    "facility",
    "canteen",
    "restaurant",
    "hotel",
    "outlet",
    "kitchen",
];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn distinctive(s: &str) -> Vec<String> {
    words(s)
        .into_iter()
        .filter(|t| !GENERIC_VENUE_WORDS.contains(&t.as_str()))
        .collect()
}

/// "NSCI" from "National Sports Club of India": initials in order, with
/// skipped and trailing words allowed. This is the positive identity evidence
/// the consecutive-acronym rule below cannot express.
fn is_initials(acro: &str, name: &str) -> bool {
    let mut chars = acro.chars();
    let Some(mut wanted) = chars.next() else {
        return false;
    };
    let mut matched_all = false;
    for word in words(name) {
        if word.starts_with(wanted) {
            match chars.next() {
                Some(next) => wanted = next,
                None => {
                    matched_all = true;
                    break;
                }
            }
        }
    }
    matched_all
}

/// A missing area is uncertainty; a named one must be corroborated by the
/// other report's establishment name (directly or via an acronym like "BKC").
pub(crate) fn area_in_name(area: &str, name: &str) -> bool {
    let area = distinctive(area);
    if area.is_empty() {
        return true;
    }
    let name = distinctive(name);
    area.iter().all(|t| name.contains(t))
        || name.iter().any(|t| {
            t.chars().count() <= area.len() && t.chars().zip(&area).all(|(c, w)| w.starts_with(c))
        })
}

/// Outlet identity used to detect the same action across reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventKey {
    pub establishment: String,
    pub city: Option<CoveredCity>,
    pub area: Option<String>,
}

impl EventKey {
    pub fn from_name(name: &str) -> Self {
        Self {
            establishment: name.to_string(),
            city: None,
            area: None,
        }
    }

    pub fn from_row(name: &str, city: Option<&str>, area: Option<&str>) -> Self {
        Self {
            establishment: name.to_string(),
            city: city.and_then(CoveredCity::parse),
            area: nonempty(area),
        }
    }

    pub fn from_candidate(c: &Candidate) -> Self {
        Self {
            establishment: c.establishment().to_string(),
            city: Some(c.city()),
            area: c.area().map(str::to_string),
        }
    }

    /// Positive name evidence: exact/containment, a consecutive acronym, or
    /// every distinctive token corroborated by the other name (including
    /// non-consecutive initials like NSCI). City and area must agree first.
    pub fn matches(&self, other: &Self) -> bool {
        let norm = |s: &str| -> String { words(s).join(" ") };

        // Both-known or one-known: a city mismatch is hard evidence against a
        // match. Both-unknown stays compatible for legacy rows and test helpers.
        if self.city != other.city {
            return false;
        }
        // Locality is only used to REJECT a match. Appending it to the name
        // would break acronym matching ("BKC" vs "Bandra Kurla Complex").
        match (self.area.as_deref(), other.area.as_deref()) {
            (Some(a), Some(b)) => {
                let (a, b) = (norm(a), norm(b));
                if a != b && !a.contains(&b) && !b.contains(&a) {
                    return false;
                }
            }
            (Some(area), None) => {
                if !area_in_name(area, &other.establishment) {
                    return false;
                }
            }
            (None, Some(area)) => {
                if !area_in_name(area, &self.establishment) {
                    return false;
                }
            }
            (None, None) => {}
        }

        let (a, b) = (norm(&self.establishment), norm(&other.establishment));
        if a == b {
            return true;
        }
        // Short-name containment is too loose ("KFC" everywhere).
        if a.len().min(b.len()) >= 6 && (a.contains(&b) || b.contains(&a)) {
            return true;
        }
        let is_acronym_of = |acro: &str, words: &[String]| -> bool {
            !acro.is_empty()
                && acro.len() <= words.len()
                && acro.chars().zip(words).all(|(c, w)| w.starts_with(c))
        };
        // Three-letter initials are too ambiguous (MCA could name two orgs);
        // the consecutive-acronym rule above still covers those aliases.
        let initials_match = |t: &str, raw: &str| t.chars().count() >= 4 && is_initials(t, raw);
        let token_matches = |t: &str, to: &[String], raw: &str| -> bool {
            to.iter().any(|w| w == t)
                || (0..=to.len().saturating_sub(t.len())).any(|i| is_acronym_of(t, &to[i..]))
                || initials_match(t, raw)
        };
        let side_matches = |from: &[String], to: &[String], raw: &str| {
            let initials = from.iter().all(|t| initials_match(t, raw));
            (from.iter().map(String::len).sum::<usize>() >= 6 || initials)
                && from.iter().all(|t| token_matches(t, to, raw))
                && (from.iter().any(|t| t.len() >= 3 && to.contains(t)) || initials)
        };
        let (ta, tb) = (distinctive(&a), distinctive(&b));
        side_matches(&ta, &tb, &b) || side_matches(&tb, &ta, &a)
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;
    use crate::models::{ActionType, LlmAction, SanitizedAction};

    const FULL: &str = "licence suspended over cockroach infestation found during inspection";

    fn same_event(a: &str, b: &str) -> bool {
        EventKey::from_name(a).matches(&EventKey::from_name(b))
    }

    #[test]
    fn same_event_matches_case_and_containment() {
        assert!(
            same_event("Otters Club", "otters club"),
            "case-insensitive exact"
        );
        assert!(
            same_event("Otters Club", "Otters Club, Carter Road, Bandra West"),
            "locality qualifier still matches"
        );
        assert!(
            same_event(
                "Blinkit Dark Store (Malad West)",
                "Blinkit Dark Store Malad West"
            ),
            "punctuation-insensitive match"
        );
    }

    #[test]
    fn same_event_rejects_different_outlets() {
        assert!(
            !same_event(
                "Domino's Pizza (Borivali West)",
                "Domino's Pizza (Ghatkopar West)"
            ),
            "different localities are different events"
        );
        assert!(
            !same_event("Domino's Pizza", "Pizza Hut Borivali"),
            "different brands never match"
        );
    }

    #[test]
    fn same_event_short_names_match_exactly_only() {
        assert!(
            !same_event("KFC", "KFC Andheri"),
            "short name containment is too loose"
        );
        assert!(same_event("KFC", "kfc"), "exact short names match");
    }

    #[test]
    fn same_event_matches_non_consecutive_initials() {
        assert!(
            same_event("NSCI", "National Sports Club of India"),
            "NSCI is the initials of the full name, not a consecutive acronym"
        );
        assert!(
            !same_event("NSCI", "National Sports Club of Pune"),
            "a shared prefix must not hide a different establishment"
        );
    }

    #[test]
    fn same_event_matches_acronym_and_venue_drift() {
        assert!(
            same_event("MCA BKC Club", "Mumbai Cricket Association (BKC Facility)"),
            "acronym plus venue-word drift matches (Aug 2026 dupes)"
        );
        assert!(
            !same_event(
                "Cricket Club of India Canteen",
                "Mumbai Cricket Association (BKC Facility)"
            ),
            "shared generic words alone do not match"
        );
    }

    #[test]
    fn matching_rejects_same_brand_in_different_areas() {
        let borivali = EventKey::from_row("Domino's Pizza", Some("Mumbai"), Some("Borivali West"));
        let ghatkopar =
            EventKey::from_row("Domino's Pizza", Some("Mumbai"), Some("Ghatkopar West"));
        assert!(
            !borivali.matches(&ghatkopar),
            "area fields name different branches"
        );
    }

    #[test]
    fn matching_uses_area_when_the_name_lacks_it() {
        let with_area = EventKey::from_row("Domino's", Some("Mumbai"), Some("Ghatkopar West"));
        let in_name = EventKey::from_row("Domino's Pizza, Ghatkopar West", Some("Mumbai"), None);
        assert!(
            with_area.matches(&in_name),
            "locality in the name is as good as an area field"
        );
    }

    #[test]
    fn matching_rejects_different_cities() {
        let mumbai = EventKey::from_row("Domino's Pizza", Some("Mumbai"), Some("Bandra West"));
        let pune = EventKey::from_row("Domino's Pizza", Some("Pune"), Some("Bandra West"));
        assert!(
            !mumbai.matches(&pune),
            "identical outlets in different cities are different events"
        );
        let unknown = EventKey::from_row("Domino's Pizza", None, Some("Bandra West"));
        assert!(
            !unknown.matches(&mumbai),
            "a legacy row with no city must not merge into a known city"
        );
    }

    #[test]
    fn matching_survives_acronym_drift_with_locality() {
        let short = EventKey::from_row("MCA BKC Club", Some("Mumbai"), None);
        let full = EventKey::from_row(
            "Mumbai Cricket Association (BKC Facility)",
            Some("Mumbai Suburban"),
            None,
        );
        assert!(
            short.matches(&full),
            "acronym and venue drift still match with city evidence"
        );
    }

    #[test]
    fn matching_survives_acronym_when_area_expands_it() {
        let short =
            EventKey::from_row("MCA BKC Club", Some("Mumbai"), Some("Bandra Kurla Complex"));
        let full = EventKey::from_row(
            "Mumbai Cricket Association (BKC Facility)",
            Some("Mumbai Suburban"),
            None,
        );
        assert!(
            short.matches(&full),
            "a multi-word area must not break the BKC acronym match"
        );
    }

    #[test]
    fn matching_rejects_conflicting_area_fields() {
        let borivali = EventKey::from_row(
            "Domino's Pizza Borivali West",
            Some("Mumbai"),
            Some("Borivali West"),
        );
        let ghatkopar = EventKey::from_row(
            "Domino's Pizza Borivali West",
            Some("Mumbai"),
            Some("Ghatkopar West"),
        );
        assert!(
            !borivali.matches(&ghatkopar),
            "contradictory area fields are different branches"
        );
    }

    #[test]
    fn matching_rejects_area_contradicted_by_the_other_name() {
        let with_area = EventKey::from_row("Domino's Pizza", Some("Mumbai"), Some("Borivali West"));
        let other_branch =
            EventKey::from_row("Domino's Pizza Ghatkopar West", Some("Mumbai"), None);
        assert!(
            !with_area.matches(&other_branch),
            "a named locality must appear in an area-less report"
        );
    }

    #[test]
    fn city_parse_accepts_punctuation_and_legacy_local_values() {
        for raw in ["Mumbai.", "Navi-Mumbai", "NaviMumbai", "Mumbai Suburban"] {
            assert!(
                CoveredCity::parse(raw).is_some(),
                "{raw} is a known city spelling"
            );
        }
        assert!(CoveredCity::parse("Mumbaiwadi").is_none());
    }

    #[test]
    fn set_establishment_reruns_the_content_gates() {
        let mut candidate = candidate("Domino's Vile Parle", Some(FULL)).expect("valid candidate");
        assert!(
            candidate
                .set_establishment("Ayurvedic Tablet".into())
                .is_err(),
            "the product-name gate applies to LLM canonical names too"
        );
        assert_eq!(candidate.establishment(), "Domino's Vile Parle");
    }

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
        Candidate::try_new(SanitizedAction::new(action), &article())
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
        assert!(Candidate::try_new(SanitizedAction::new(action), &article()).is_ok());
    }

    #[test]
    fn covered_city_serializes_navi_mumbai_with_space() {
        assert_eq!(
            serde_json::to_string(&CoveredCity::NaviMumbai).expect("serialize city"),
            "\"Navi Mumbai\""
        );
    }

    #[test]
    fn event_key_from_candidate_keeps_locality() {
        let action = LlmAction {
            establishment: "Domino's".into(),
            area: Some("Ghatkopar West".into()),
            city: Some("Navi Mumbai".into()),
            action_date: Some("2026-08-11".into()),
            details: Some(FULL.into()),
            ..Default::default()
        };
        let c =
            Candidate::try_new(SanitizedAction::new(action), &article()).expect("valid candidate");
        let key = EventKey::from_candidate(&c);
        assert_eq!(key.establishment, "Domino's");
        assert_eq!(key.city, Some(CoveredCity::NaviMumbai));
        assert_eq!(key.area.as_deref(), Some("Ghatkopar West"));
    }

    #[test_case("  Domino's Vile Parle  ", Ok(()); "trims")]
    #[test_case("   ", Err(Rejection::NoEstablishment); "blank")]
    #[test_case("\"Quoted outlet\"", Err(Rejection::QuoteLedEstablishment); "leading quote")]
    #[test_case("one two three four five six seven eight nine ten", Err(Rejection::HeadlineFragment); "too many words")]
    fn set_establishment_revalidates(name: &str, expected: Result<(), Rejection>) {
        let mut c = candidate("Domino's Vile Parle", Some(FULL)).expect("valid candidate");
        assert_eq!(c.set_establishment(name.to_string()), expected, "{name}");
        if expected.is_ok() {
            assert_eq!(c.establishment(), name.trim());
        }
    }

    #[test]
    fn set_establishment_rejects_overlong_name() {
        let mut c = candidate("Domino's Vile Parle", Some(FULL)).expect("valid candidate");
        assert_eq!(
            c.set_establishment("x".repeat(201)),
            Err(Rejection::HeadlineFragment)
        );
    }
}
