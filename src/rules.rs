use std::sync::OnceLock;

use serde::Deserialize;

const TEXT: &str = include_str!("rules.toml");

#[derive(Debug, Deserialize)]
pub(crate) struct Rules {
    pub(crate) quality: Quality,
    pub(crate) news: News,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Quality {
    pub(crate) product_words: Vec<String>,
    pub(crate) no_action_yet: Vec<String>,
    pub(crate) regulator_words: Vec<String>,
    pub(crate) max_establishment_words: usize,
    pub(crate) min_details_chars: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct News {
    pub(crate) queries: Vec<String>,
    pub(crate) authority_signals: Vec<String>,
    pub(crate) action_signals: Vec<String>,
    pub(crate) venue_signals: Vec<String>,
    pub(crate) licence_words: Vec<String>,
    pub(crate) licence_actions: Vec<String>,
}

static BUNDLED: OnceLock<Rules> = OnceLock::new();

/// A bad bundled rules.toml is a build bug, hence the expect.
pub(crate) fn rules() -> &'static Rules {
    BUNDLED.get_or_init(|| toml::from_str(TEXT).expect("bundled rules.toml is valid"))
}

#[cfg(test)]
mod tests {
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
}
