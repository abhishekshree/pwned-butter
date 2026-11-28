use std::env::temp_dir;
use std::fs::remove_file;
use std::process::id;

use chrono::Duration;

use super::*;
use crate::domain::Article;
use crate::models::{ActionType, LlmAction, Sanitized};

fn candidate_on(url: &str, establishment: &str, date: &str) -> Candidate {
    let action = LlmAction {
        establishment: establishment.into(),
        city: Some("Mumbai".into()),
        action_type: ActionType::Inspection,
        action_date: Some(date.into()),
        details: Some(
            "licence suspended over cockroach infestation found during inspection".into(),
        ),
        ..Default::default()
    };
    let article = Article {
        title: "headline".into(),
        url: url.into(),
        source: None,
        published: None,
        snippet: None,
    };
    Candidate::try_new(Sanitized::new(action), &article).expect("valid candidate")
}

#[test]
fn run_ids_increment_once_per_run() {
    let path = temp_dir().join(format!("fda-local-runs-{}.jsonl", id()));
    let writer = LocalWriter::new(path);
    let _ = remove_file(writer.runs_path());

    let first = writer.begin_run().expect("first run");
    let second = writer.begin_run().expect("second run");

    assert_eq!((first, second), (1, 2));
    let _ = remove_file(writer.runs_path());
}

#[test]
fn writes_deduplicates_and_reports_seen_urls() {
    let path = temp_dir().join(format!("fda-local-writer-{}.jsonl", id()));
    let _ = remove_file(&path);
    let writer = LocalWriter::new(path.clone());
    let rows = [candidate_on(
        "https://example.test/1",
        "Domino's Vile Parle",
        "2026-08-11",
    )];

    let first = writer.upsert_actions(&rows).expect("first write");
    let second = writer.upsert_actions(&rows).expect("second write");
    let seen = writer
        .seen_urls(Utc::now() - Duration::days(1))
        .expect("read back");

    assert_eq!((first.inserted, second.skipped), (1, 1));
    assert!(seen.contains("https://example.test/1"));

    let _ = remove_file(&path);
}

#[test]
fn dedups_one_event_across_urls_and_nearby_dates() {
    let path = temp_dir().join(format!("fda-local-cross-{}.jsonl", id()));
    let _ = remove_file(&path);
    let writer = LocalWriter::new(path.clone());

    let first = writer
        .upsert_actions(&[candidate_on(
            "https://example.test/1",
            "National Sports Club of India",
            "2026-08-11",
        )])
        .expect("first write");
    let second = writer
        .upsert_actions(&[candidate_on(
            "https://example.test/2",
            "National Sports Club of India (NSCI)",
            "2026-08-13",
        )])
        .expect("second write");

    assert_eq!(
        (first.inserted, second.skipped),
        (1, 1),
        "a re-report by another outlet is one event, as in the Postgres store"
    );
    let _ = remove_file(&path);
}
