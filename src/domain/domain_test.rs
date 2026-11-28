use super::*;
use crate::error::Rejection;
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

#[test]
fn gates_junk_shapes() {
    let cases = [
        ("complete record", "Domino's Vile Parle", Some(FULL), Ok(())),
        (
            "long genuine name",
            "IIT Bombay Hostel Canteen (Hostels 12, 13 & 14)",
            Some(FULL),
            Ok(()),
        ),
        (
            "details at floor",
            "Domino's Vile Parle",
            Some("12345678901234567890"),
            Ok(()),
        ),
        (
            "quote-led name",
            "'Popularity was never my goal'",
            Some(FULL),
            Err(Rejection::QuoteLedEstablishment),
        ),
        (
            "product name",
            "Popular Re 1 Ayurvedic Digestive Tablet Fails Maharashtra FDA Quality Test",
            Some(FULL),
            Err(Rejection::ProductNotOutlet),
        ),
        (
            "regulator name",
            "Maharashtra FDA rolls out year",
            Some(FULL),
            Err(Rejection::RegulatorNotOutlet),
        ),
        (
            "awaiting response",
            "FDA awaiting response from restaurants at Mumbai Cricket Association premises",
            Some(FULL),
            Err(Rejection::PendingAction),
        ),
        (
            "awaits replies",
            "FDA awaits replies from restaurants at MCA premises",
            Some(FULL),
            Err(Rejection::PendingAction),
        ),
        (
            "awaits replies long",
            "Maharashtra FDA awaits replies from five MCA restaurants over licensing violations",
            Some(FULL),
            Err(Rejection::PendingAction),
        ),
        (
            "headline fragment",
            "Popular Re 1 Digestive Item Fails Maharashtra Quality Test Today Here",
            Some(FULL),
            Err(Rejection::HeadlineFragment),
        ),
        (
            "no details",
            "Domino's Vile Parle",
            None,
            Err(Rejection::NoSubstantiveDetails),
        ),
        (
            "short details",
            "Domino's Vile Parle",
            Some("1234567890123456789"),
            Err(Rejection::NoSubstantiveDetails),
        ),
        (
            "seeks explanation",
            "MCA BKC Club",
            Some("FDA has issued a notice and seeks explanation from the club within seven days"),
            Err(Rejection::PendingAction),
        ),
        (
            "blank name",
            "   ",
            Some(FULL),
            Err(Rejection::NoEstablishment),
        ),
    ];
    for (name, est, details, expected) in cases {
        assert_eq!(
            candidate(est, details).map(|_| ()),
            expected,
            "{name}: {est}"
        );
    }
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
