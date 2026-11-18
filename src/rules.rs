use std::sync::OnceLock;

use serde::Deserialize;

use crate::models::ActionType;

const TEXT: &str = include_str!("rules.toml");

#[derive(Debug, Deserialize)]
pub(crate) struct Rules {
    pub(crate) triage: Triage,
    pub(crate) quality: Quality,
    pub(crate) news: News,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Triage {
    pub(crate) rules: Vec<Signal>,
    pub(crate) corroboration: Vec<String>,
    pub(crate) cities: Vec<String>,
    pub(crate) establishment_prefixes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Signal {
    pub(crate) action: ActionType,
    /// Alternative conjunctions; all needles in one group must appear.
    pub(crate) needs: Vec<Vec<String>>,
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

/// Parsed once from the file embedded in the binary. `rules.toml` is shipped
/// with the crate and the test below validates it, so a bad parse is a build
/// bug, not runtime input.
pub(crate) fn rules() -> &'static Rules {
    BUNDLED.get_or_init(|| toml::from_str(TEXT).expect("bundled rules.toml is valid"))
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn bundled_rules_are_complete_and_lowercase() {
        let r = rules();
        let covered: Vec<_> = r.triage.rules.iter().map(|s| s.action).collect();
        for action in ActionType::iter() {
            assert!(covered.contains(&action), "no keyword rule for {action}");
        }
        for signal in &r.triage.rules {
            assert!(
                !signal.needs.is_empty(),
                "{} has no keywords",
                signal.action
            );
            for group in &signal.needs {
                for needle in group {
                    assert_eq!(needle, &needle.to_lowercase(), "{needle} must be lowercase");
                }
            }
        }
        for needle in &r.triage.corroboration {
            assert_eq!(needle, &needle.to_lowercase(), "{needle} must be lowercase");
        }
        assert!(!r.news.queries.is_empty());
        assert!(r.quality.min_details_chars > 0);
    }
}
