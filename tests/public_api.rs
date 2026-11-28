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
fn same_event_matches() {
    let cases = [
        ("case-insensitive exact", "Otters Club", "otters club", true),
        (
            "locality qualifier still matches",
            "Otters Club",
            "Otters Club, Carter Road, Bandra West",
            true,
        ),
        (
            "punctuation-insensitive match",
            "Blinkit Dark Store (Malad West)",
            "Blinkit Dark Store Malad West",
            true,
        ),
        (
            "different localities are different events",
            "Domino's Pizza (Borivali West)",
            "Domino's Pizza (Ghatkopar West)",
            false,
        ),
        (
            "different brands never match",
            "Domino's Pizza",
            "Pizza Hut Borivali",
            false,
        ),
        (
            "short name containment is too loose",
            "KFC",
            "KFC Andheri",
            false,
        ),
        ("exact short names match", "KFC", "kfc", true),
        (
            "acronym plus venue-word drift matches",
            "MCA BKC Club",
            "Mumbai Cricket Association (BKC Facility)",
            true,
        ),
        (
            "shared generic words alone do not match",
            "Cricket Club of India Canteen",
            "Mumbai Cricket Association (BKC Facility)",
            false,
        ),
        (
            "acronym expands without a collapse pass",
            "NSCI",
            "National Sports Club of India",
            true,
        ),
    ];
    for (name, a, b, expected) in cases {
        assert_eq!(same_event(a, b), expected, "{name}: {a} vs {b}");
    }
}
