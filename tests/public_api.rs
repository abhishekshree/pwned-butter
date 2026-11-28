//! Tests that exercise only the crate's public surface.

use std::fs::{self, File};

use serde_json::json;
use test_case::test_case;

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

#[test_case("Otters Club", "otters club", true; "case-insensitive exact")]
#[test_case("Otters Club", "Otters Club, Carter Road, Bandra West", true; "locality qualifier still matches")]
#[test_case("Blinkit Dark Store (Malad West)", "Blinkit Dark Store Malad West", true; "punctuation-insensitive match")]
#[test_case("Domino's Pizza (Borivali West)", "Domino's Pizza (Ghatkopar West)", false; "different localities are different events")]
#[test_case("Domino's Pizza", "Pizza Hut Borivali", false; "different brands never match")]
#[test_case("KFC", "KFC Andheri", false; "short name containment is too loose")]
#[test_case("KFC", "kfc", true; "exact short names match")]
#[test_case("MCA BKC Club", "Mumbai Cricket Association (BKC Facility)", true; "acronym plus venue-word drift matches")]
#[test_case("Cricket Club of India Canteen", "Mumbai Cricket Association (BKC Facility)", false; "shared generic words alone do not match")]
fn same_event_matches(a: &str, b: &str, expected: bool) {
    assert_eq!(same_event(a, b), expected, "{a} vs {b}");
}
