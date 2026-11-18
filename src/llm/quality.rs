use crate::models::LlmAction;
use crate::rules::rules;

/// Post-LLM quality gate: better no record than a wrong one. Every rule is
/// calibrated against the pre-Sept-2026 history (67/67 rows pass).
pub(crate) fn check(a: &LlmAction) -> Result<(), &'static str> {
    let config = &rules().quality;
    let name = a.establishment.trim();
    if name.is_empty() {
        return Err("empty establishment");
    }
    if name.starts_with(['\'', '"', '‘', '“']) {
        return Err("quote-led establishment");
    }
    let lower = name.to_lowercase();
    let details = a.details.as_deref().unwrap_or("").trim();
    let hay = format!("{lower} {}", details.to_lowercase());
    if config.no_action_yet.iter().any(|w| hay.contains(w)) {
        return Err("no concrete action yet");
    }
    if config.product_words.iter().any(|w| lower.contains(w)) {
        return Err("establishment is a product, not an outlet");
    }
    if config.regulator_words.iter().any(|w| lower.contains(w)) {
        return Err("establishment is the regulator, not an outlet");
    }
    if name.split_whitespace().count() > config.max_establishment_words {
        return Err("establishment reads like a headline fragment");
    }
    if details.chars().count() < config.min_details_chars {
        return Err("no substantive details");
    }
    if a.city.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Err("no city");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;
    use crate::models::ActionType;

    const FULL: &str = "licence suspended over cockroach infestation found during inspection";

    fn action(est: &str, details: Option<&str>, city: Option<&str>) -> LlmAction {
        LlmAction {
            establishment: est.into(),
            action_type: ActionType::LicenceSuspension,
            violations: vec!["pest infestation".into()],
            details: details.map(str::to_string),
            city: city.map(str::to_string),
            source_index: 0,
            ..Default::default()
        }
    }

    #[test_case("Domino's Vile Parle", Some(FULL), Some("Mumbai"), Ok(()); "complete record")]
    #[test_case("IIT Bombay Hostel Canteen (Hostels 12, 13 & 14)", Some(FULL), Some("Mumbai"), Ok(()); "long genuine name")]
    #[test_case("Domino's Vile Parle", Some("12345678901234567890"), Some("Mumbai"), Ok(()); "details at floor")]
    #[test_case("'Popularity was never my goal'", Some(FULL), Some("Mumbai"), Err("quote-led establishment"); "quote-led name")]
    #[test_case("Popular Re 1 Ayurvedic Digestive Tablet Fails Maharashtra FDA Quality Test", Some(FULL), Some("Mumbai"), Err("establishment is a product, not an outlet"); "product name")]
    #[test_case("Maharashtra FDA rolls out year", Some(FULL), Some("Mumbai"), Err("establishment is the regulator, not an outlet"); "regulator name")]
    #[test_case("FDA awaiting response from restaurants at Mumbai Cricket Association premises", Some(FULL), Some("Mumbai"), Err("no concrete action yet"); "awaiting response")]
    #[test_case("FDA awaits replies from restaurants at MCA premises", Some(FULL), Some("Mumbai"), Err("no concrete action yet"); "awaits replies")]
    #[test_case("Maharashtra FDA awaits replies from five MCA restaurants over licensing violations", Some(FULL), Some("Mumbai"), Err("no concrete action yet"); "awaits replies long")]
    #[test_case("Popular Re 1 Digestive Item Fails Maharashtra Quality Test Today Here", Some(FULL), Some("Mumbai"), Err("establishment reads like a headline fragment"); "headline fragment")]
    #[test_case("Domino's Vile Parle", None, Some("Mumbai"), Err("no substantive details"); "no details")]
    #[test_case("Domino's Vile Parle", Some("1234567890123456789"), Some("Mumbai"), Err("no substantive details"); "short details")]
    #[test_case("Domino's Vile Parle", Some(FULL), None, Err("no city"); "no city")]
    #[test_case("MCA BKC Club", Some("FDA has issued a notice and seeks explanation from the club within seven days"), Some("Mumbai"), Err("no concrete action yet"); "seeks explanation")]
    #[test_case("   ", Some(FULL), Some("Mumbai"), Err("empty establishment"); "blank name")]
    fn gates_junk_shapes(
        est: &str,
        details: Option<&str>,
        city: Option<&str>,
        expected: Result<(), &'static str>,
    ) {
        assert_eq!(check(&action(est, details, city)), expected, "{est}");
    }

    #[test]
    fn keeps_inspection_without_violations_when_detailed() {
        // History rows 14/32: inspections with no cited violations but real details.
        let mut a = action(
            "Bombay High Court Canteen",
            Some(
                "FDA team inspected canteens inside the Bombay High Court premises following queries from the court.",
            ),
            Some("Mumbai"),
        );
        a.action_type = ActionType::Inspection;
        a.violations.clear();
        assert!(check(&a).is_ok());
    }
}
