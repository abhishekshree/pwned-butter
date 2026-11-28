use super::*;

#[test]
fn bundled_rules_are_lowercase_and_nonempty() {
    let r = rules();
    let groups: [&[String]; 6] = [
        &r.quality.product_words,
        &r.quality.no_action_yet,
        &r.quality.regulator_words,
        &r.news.authority_signals,
        &r.news.action_signals,
        &r.news.venue_signals,
    ];
    for group in groups {
        assert!(!group.is_empty());
        for needle in group {
            assert_eq!(needle, &needle.to_lowercase(), "{needle} must be lowercase");
        }
    }
    assert!(!r.news.queries.is_empty());
    assert!(r.quality.min_details_chars > 0);
    assert!(r.quality.max_establishment_words > 0);
}
