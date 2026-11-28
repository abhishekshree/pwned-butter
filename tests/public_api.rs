//! Tests that exercise only the crate's public surface.

use std::fs::{self, File};

use serde_json::json;

use fda_mumbai_tracker::artifacts::write_json;
use fda_mumbai_tracker::domain::EventKey;

fn same_event(a: &str, b: &str) -> bool {
    EventKey::from_name(a).matches(&EventKey::from_name(b))
}

#[test]
fn writes_named_json_file() {
    let dir = std::env::temp_dir().join(format!("pwned-butter-artifacts-{}", std::process::id()));
    let value = json!({"ok": true});
    write_json(&dir, "sample.json", &value).expect("write artifact");
    let read: serde_json::Value =
        serde_json::from_reader(File::open(dir.join("sample.json")).expect("open artifact"))
            .expect("parse artifact");
    assert_eq!(read, value);
    fs::remove_dir_all(dir).ok();
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
