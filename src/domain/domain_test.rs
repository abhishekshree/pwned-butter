use test_case::test_case;

use super::*;
use crate::error::Rejection;
use crate::models::{ActionType, LlmAction, Sanitized};

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
    Candidate::try_new(Sanitized::new(action), &article())
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
    assert!(Candidate::try_new(Sanitized::new(action), &article()).is_ok());
}
