use crate::models::LlmAction;

/// Words that mark an establishment as a pharma product, never a food outlet.
const PRODUCT_WORDS: &[&str] = &["tablet", "syrup", "medicine", "drug"];

/// Phrasing that marks a story as "action pending", never a concrete action.
/// Deliberately specific: bare "await" would nuke genuine suspensions
/// "awaiting reinspection".
const NO_ACTION_YET: &[&str] = &[
    "awaits repl",
    "awaiting respon",
    "reply awaited",
    "response awaited",
    "seeks explanation",
    "seeks reply",
    "sought explanation",
    "yet to respond",
];

/// Longest genuine outlet name in history is 9 words
/// ("IIT Bombay Hostel Canteen (Hostels 12, 13 & 14)").
const MAX_ESTABLISHMENT_WORDS: usize = 9;
/// Shortest genuine details in history is 43 chars; floor at 20.
pub(crate) const MIN_DETAILS_CHARS: usize = 20;

/// Post-LLM quality gate: better no record than a wrong one. Every rule is
/// calibrated against the pre-Sept-2026 history (67/67 rows pass) and each
/// rejects a shape the 6 Sept 2026 junk took.
pub(crate) fn check(a: &LlmAction) -> Result<(), &'static str> {
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
    if NO_ACTION_YET.iter().any(|w| hay.contains(w)) {
        return Err("no concrete action yet");
    }
    if PRODUCT_WORDS.iter().any(|w| lower.contains(w)) {
        return Err("establishment is a product, not an outlet");
    }
    if lower.contains("fda") {
        return Err("establishment is the regulator, not an outlet");
    }
    if name.split_whitespace().count() > MAX_ESTABLISHMENT_WORDS {
        return Err("establishment reads like a headline fragment");
    }
    if details.chars().count() < MIN_DETAILS_CHARS {
        return Err("no substantive details");
    }
    if a.city.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Err("no city");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActionType;

    const FULL: &str = "licence suspended over cockroach infestation found during inspection";

    type Case<'a> = (
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        Result<(), &'static str>,
    );

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

    fn ok(est: &str) -> Case<'_> {
        (est, Some(FULL), Some("Mumbai"), Ok(()))
    }

    fn rejected<'a>(est: &'a str, reason: &'static str) -> Case<'a> {
        (est, Some(FULL), Some("Mumbai"), Err(reason))
    }

    #[test]
    fn gates_every_junk_shape_from_sept_2026() {
        let no_action = "no concrete action yet";
        let cases: [Case; 15] = [
            ok("Domino's Vile Parle"),
            ok("IIT Bombay Hostel Canteen (Hostels 12, 13 & 14)"),
            (
                "Domino's Vile Parle",
                Some("12345678901234567890"),
                Some("Mumbai"),
                Ok(()),
            ),
            rejected("'Popularity was never my goal'", "quote-led establishment"),
            rejected(
                "Popular Re 1 Ayurvedic Digestive Tablet Fails Maharashtra FDA Quality Test",
                "establishment is a product, not an outlet",
            ),
            rejected(
                "Maharashtra FDA rolls out year",
                "establishment is the regulator, not an outlet",
            ),
            rejected(
                "FDA awaiting response from restaurants at Mumbai Cricket Association premises",
                no_action,
            ),
            rejected(
                "FDA awaits replies from restaurants at MCA premises",
                no_action,
            ),
            rejected(
                "Maharashtra FDA awaits replies from five MCA restaurants over licensing violations",
                no_action,
            ),
            rejected(
                "Popular Re 1 Digestive Item Fails Maharashtra Quality Test Today Here",
                "establishment reads like a headline fragment",
            ),
            (
                "Domino's Vile Parle",
                None,
                Some("Mumbai"),
                Err("no substantive details"),
            ),
            (
                "Domino's Vile Parle",
                Some("1234567890123456789"),
                Some("Mumbai"),
                Err("no substantive details"),
            ),
            ("Domino's Vile Parle", Some(FULL), None, Err("no city")),
            (
                "MCA BKC Club",
                Some(
                    "FDA has issued a notice and seeks explanation from the club within seven days",
                ),
                Some("Mumbai"),
                Err(no_action),
            ),
            (
                "   ",
                Some(FULL),
                Some("Mumbai"),
                Err("empty establishment"),
            ),
        ];
        for (est, details, city, expected) in cases {
            assert_eq!(check(&action(est, details, city)), expected, "{est}");
        }
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
