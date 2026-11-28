use chrono::{TimeZone, Utc};
use test_case::test_case;

use crate::domain::{Article, Candidate, CoveredCity};
use crate::error::Rejection;
use crate::models::{ActionType, LlmAction, Sanitized};

fn article(published: Option<chrono::DateTime<Utc>>) -> Article {
    Article {
        title: "Domino's licence suspended in Mumbai".into(),
        url: "https://a.test/1".into(),
        source: Some("Test Press".into()),
        published,
        snippet: Some("licence suspended over cockroach infestation".into()),
    }
}

fn action() -> LlmAction {
    LlmAction {
        establishment: "Domino's Vile Parle".into(),
        city: Some("Mumbai".into()),
        action_type: ActionType::LicenceSuspension,
        action_date: Some("2026-08-11".into()),
        details: Some(
            "licence suspended over cockroach infestation found during inspection".into(),
        ),
        ..Default::default()
    }
}

fn published() -> Option<chrono::DateTime<Utc>> {
    Some(
        Utc.with_ymd_and_hms(2026, 11, 18, 9, 0, 0)
            .single()
            .expect("valid test date"),
    )
}

#[test]
fn candidate_keeps_its_source_provenance() {
    let mut action = action();
    action.brand = Some("Domino's".into());
    action.outlet_type = Some("restaurant".into());
    let source = article(published());
    let candidate = Candidate::try_new(Sanitized::new(action), &source).expect("valid");

    assert_eq!(candidate.source_url, "https://a.test/1");
    assert_eq!(candidate.source_publisher.as_deref(), Some("Test Press"));
    assert_eq!(
        candidate.source_headline.as_deref(),
        Some("Domino's licence suspended in Mumbai")
    );
    assert_eq!(candidate.brand.as_deref(), Some("Domino's"));
    assert_eq!(candidate.action_type, ActionType::LicenceSuspension);
    assert_eq!(candidate.city, CoveredCity::Mumbai);
}

#[test]
fn rejects_out_of_scope_city() {
    let mut action = action();
    action.city = Some("Nashik".into());
    assert_eq!(
        Candidate::try_new(Sanitized::new(action), &article(published())).err(),
        Some(Rejection::OutOfScopeCity),
        "uncovered city is rejected at the boundary"
    );
}

#[test]
fn rejects_missing_action_date() {
    let mut action = action();
    action.action_date = None;
    assert_eq!(
        Candidate::try_new(Sanitized::new(action), &article(None)).err(),
        Some(Rejection::NoActionDate),
        "no reported or published date means no fabricated date"
    );
}

#[test]
fn unknown_outlet_code_becomes_none() {
    let mut action = action();
    action.outlet_type = Some("pavement stand".into());
    let candidate =
        Candidate::try_new(Sanitized::new(action), &article(published())).expect("valid candidate");
    assert_eq!(
        candidate.outlet_type, None,
        "unknown codes are not an error sink"
    );
}

#[test_case(Some(54), Some(54); "in range")]
#[test_case(Some(999), None; "too high")]
#[test_case(Some(-5), None; "negative")]
#[test_case(Some(101), None; "just out of range")]
#[test_case(None, None; "absent")]
fn compliance_score_keeps_only_0_to_100(score: Option<i32>, expected: Option<i32>) {
    let mut action = action();
    action.compliance_score = score;
    let candidate =
        Candidate::try_new(Sanitized::new(action), &article(published())).expect("valid candidate");
    assert_eq!(candidate.compliance_score, expected, "{score:?}");
}
