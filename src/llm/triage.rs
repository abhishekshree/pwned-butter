use crate::models::{ActionType, LlmAction, NewsItem};
use crate::rules::rules;

/// A relevance-filtered batch item; `orig` still indexes the caller's `items`
/// for `build_rows`.
pub(crate) struct Job<'a> {
    pub(crate) orig: usize,
    pub(crate) item: &'a NewsItem,
}

/// Typed action from prose; `None` when no rule matches.
fn signal(hay: &str) -> Option<ActionType> {
    rules().triage.rules.iter().find_map(|rule| {
        rule.needs
            .iter()
            .any(|needles| needles.iter().all(|n| hay.contains(n)))
            .then_some(rule.action)
    })
}

/// Keyword signal plus a corroboration gate for ambiguous action types.
/// `hay` must already be lowercase — [`NewsItem::haystack`] guarantees it.
pub(crate) fn triage(hay: &str) -> Option<ActionType> {
    let action = signal(hay)?;
    let needs_corroboration = matches!(
        action,
        ActionType::StopBusiness
            | ActionType::Sealing
            | ActionType::Seizure
            | ActionType::Inspection
    );
    (!needs_corroboration || rules().triage.corroboration.iter().any(|c| hay.contains(c)))
        .then_some(action)
}

pub(crate) fn city_of(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    rules()
        .triage
        .cities
        .iter()
        .find(|city| lower.contains(&city.to_lowercase()))
        .cloned()
}

fn clean_establishment_name(raw: &str) -> String {
    let trimmed = raw.trim();
    for prefix in &rules().triage.establishment_prefixes {
        // `get` guards against a non-char-boundary slice on odd UTF-8.
        let Some(head) = trimmed.get(..prefix.len()) else {
            continue;
        };
        if !head.eq_ignore_ascii_case(prefix) {
            continue;
        }
        let rest = trimmed[prefix.len()..]
            .trim_start_matches([':', '-', ',', ' '])
            .trim();
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    trimmed.to_string()
}

/// Headline before the "|", "-" or ":" separator, with the boilerplate
/// enforcement prefix stripped.
fn outlet_from_title(title: &str) -> Option<String> {
    let name: String = title
        .split(['|', '-', ':'])
        .next()?
        .trim()
        .chars()
        .take(120)
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| clean_establishment_name(name))
}

// Deterministic safety net: when a batch's LLM path fails, keep one minimal
// record per signal-bearing item so the run degrades instead of zeroing out.
// Rows with no named city are dropped here, not assigned a guessed one.
pub(crate) fn rule_extract(jobs: &[Job<'_>]) -> Vec<LlmAction> {
    jobs.iter()
        .filter_map(|j| {
            let action_type = triage(&j.item.haystack())?;
            let name = outlet_from_title(&j.item.title)?;
            let city = city_of(&format!(
                "{} {}",
                j.item.title,
                j.item.snippet.as_deref().unwrap_or("")
            ))?;
            let mut action = LlmAction::minimal(name, action_type, j.orig, j.item.snippet.clone());
            action.city = Some(city);
            Some(action)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActionType;

    #[test]
    fn triage_drops_ambiguous_signals_without_corroboration() {
        assert_eq!(
            triage("fda raid seals eatery"),
            Some(ActionType::Sealing),
            "sealing with fda context passes"
        );
        assert_eq!(
            triage("shop sealed after fire"),
            None,
            "sealing without corroboration is dropped"
        );
    }

    #[test]
    fn triage_drops_fda_only_stories() {
        // 6 Sept 2026 junk: bare FDA mentions with no raid/inspection action.
        assert_eq!(
            triage("popular re 1 ayurvedic digestive tablet fails maharashtra fda quality test"),
            None,
        );
        assert_eq!(
            triage("maharashtra fda rolls out year-round food safety plan for festivals"),
            None,
        );
        assert_eq!(
            triage("fda awaiting response from restaurants at mumbai cricket association premises"),
            None,
        );
        assert_eq!(
            triage(
                "maharashtra fda awaits replies from five mca restaurants over licensing violations"
            ),
            None,
        );
        assert_eq!(
            triage("fda raid at hotel cites hygiene violations"),
            Some(ActionType::Inspection),
        );
        assert_eq!(
            triage("food safety inspectors raid eatery"),
            Some(ActionType::Inspection),
        );
    }

    #[test]
    fn triage_passes_unambiguous_signals_freely() {
        assert_eq!(
            triage("outlet served improvement notice"),
            Some(ActionType::ImprovementNotice)
        );
        assert_eq!(triage("eatery reopened"), Some(ActionType::Reopened));
        assert_eq!(
            triage("licence suspended over pests"),
            Some(ActionType::LicenceSuspension)
        );
        assert_eq!(
            triage("maharashtra fda orders rajdhani biryani centre to cease business"),
            Some(ActionType::StopBusiness)
        );
        assert_eq!(
            triage("fda revoked the food licence after hygiene violations"),
            Some(ActionType::LicenceSuspension)
        );
    }

    #[test]
    fn triage_does_not_treat_a_non_regulatory_closure_as_enforcement() {
        assert_eq!(
            triage("Restaurant ordered to shut after a landlord dispute"),
            None
        );
    }

    #[test]
    fn rule_fallback_keeps_signal_items_only() {
        let licenced = NewsItem {
            title: "Domino's licence suspended in Mumbai over pests".into(),
            url: "https://x.test/1".into(),
            source: None,
            published: None,
            snippet: None,
        };
        let cityless = NewsItem {
            title: "FDA seals eatery over pests".into(),
            url: "https://x.test/2".into(),
            source: None,
            published: None,
            snippet: None,
        };
        let cricket = NewsItem {
            title: "cricket highlights and match report".into(),
            url: "https://x.test/1".into(),
            source: None,
            published: None,
            snippet: None,
        };
        let jobs = vec![
            Job {
                orig: 4,
                item: &licenced,
            },
            Job {
                orig: 5,
                item: &cityless,
            },
            Job {
                orig: 7,
                item: &cricket,
            },
        ];
        let kept = rule_extract(&jobs);
        assert_eq!(kept.len(), 1, "only signal items with a city are kept");
        assert_eq!(kept[0].source_index, 4, "orig index survives");
        assert_eq!(
            kept[0].action_type,
            ActionType::LicenceSuspension,
            "licence suspension detected"
        );
        assert_eq!(
            kept[0].city.as_deref(),
            Some("Mumbai"),
            "city inferred from the headline"
        );
    }

    #[test]
    fn city_of_prefers_the_longer_place_name() {
        assert_eq!(
            city_of("outlet sealed in Navi Mumbai").as_deref(),
            Some("Navi Mumbai")
        );
        assert_eq!(city_of("FDA raid in Pune").as_deref(), Some("Pune"));
        assert_eq!(city_of("no location here"), None);
    }
}
