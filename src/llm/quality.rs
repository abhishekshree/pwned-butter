use crate::domain::Candidate;
use crate::error::Rejection;
use crate::rules::rules;

/// Post-LLM quality gate: better no record than a wrong one. Every rule is
/// calibrated against the pre-Sept-2026 history (67/67 rows pass). Structural
/// checks (establishment, city, date) live in `Candidate::try_new`.
pub(crate) fn check(c: &Candidate) -> Result<(), Rejection> {
    let config = &rules().quality;
    let name = c.establishment.trim();
    if name.is_empty() {
        return Err(Rejection::NoEstablishment);
    }
    if name.starts_with(['\'', '"', '‘', '“']) {
        return Err(Rejection::QuoteLedEstablishment);
    }
    let lower = name.to_lowercase();
    let details = c.details.as_deref().unwrap_or("").trim();
    let hay = format!("{lower} {}", details.to_lowercase());
    if config.no_action_yet.iter().any(|w| hay.contains(w)) {
        return Err(Rejection::PendingAction);
    }
    if config.product_words.iter().any(|w| lower.contains(w)) {
        return Err(Rejection::ProductNotOutlet);
    }
    if config.regulator_words.iter().any(|w| lower.contains(w)) {
        return Err(Rejection::RegulatorNotOutlet);
    }
    if name.split_whitespace().count() > config.max_establishment_words {
        return Err(Rejection::HeadlineFragment);
    }
    if details.chars().count() < config.min_details_chars {
        return Err(Rejection::NoSubstantiveDetails);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use test_case::test_case;

    use super::*;
    use crate::domain::CoveredCity;
    use crate::models::ActionType;

    const FULL: &str = "licence suspended over cockroach infestation found during inspection";

    fn candidate(est: &str, details: Option<&str>) -> Candidate {
        Candidate {
            establishment: est.into(),
            area: None,
            city: CoveredCity::Mumbai,
            brand: None,
            operator: None,
            outlet_type: None,
            action_type: ActionType::LicenceSuspension,
            action_date: NaiveDate::from_ymd_opt(2026, 8, 11).expect("valid test date"),
            violations: vec!["pest infestation".into()],
            compliance_score: None,
            fssai_number: None,
            details: details.map(str::to_string),
            platforms: Vec::new(),
            source_url: "https://example.test/1".into(),
            source_publisher: None,
            source_headline: None,
            published_at: None,
        }
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
        assert_eq!(check(&candidate(est, details)), expected, "{est}");
    }

    #[test]
    fn keeps_inspection_without_violations_when_detailed() {
        // History rows 14/32: inspections with no cited violations but real details.
        let mut c = candidate(
            "Bombay High Court Canteen",
            Some(
                "FDA team inspected canteens inside the Bombay High Court premises following queries from the court.",
            ),
        );
        c.action_type = ActionType::Inspection;
        c.violations.clear();
        assert!(check(&c).is_ok(), "detailed inspection is kept");
    }
}
