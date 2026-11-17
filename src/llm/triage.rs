use crate::models::{ActionType, LlmAction, NewsItem};

/// A relevance-filtered batch item; `orig` indexes the caller's `items`
/// slice so `source_index` still addresses it for `build_rows`.
pub(crate) struct Job<'a> {
    pub(crate) orig: usize,
    pub(crate) item: &'a NewsItem,
}

pub(crate) fn haystack(it: &NewsItem) -> String {
    format!(
        "{} {} {}",
        it.title,
        it.snippet.as_deref().unwrap_or(""),
        it.source.as_deref().unwrap_or("")
    )
    .to_lowercase()
}

/// Keyword rules as data, not branches: each entry is alternative
/// conjunctions + the action they signal. First match in table order wins,
/// so entries run most- to least-specific. Adding a keyword edits this
/// table, never the control flow below it.
struct Rule {
    needs: &'static [&'static [&'static str]],
    action: ActionType,
}

const RULES: &[Rule] = &[
    Rule {
        needs: &[&["improvement notice"]],
        action: ActionType::ImprovementNotice,
    },
    Rule {
        needs: &[
            &["licence", "suspend"],
            &["license", "suspend"],
            &["licence", "suspension"],
            &["license", "suspension"],
            &["licence", "cancel"],
            &["license", "cancel"],
            &["licence", "revok"],
            &["license", "revok"],
        ],
        action: ActionType::LicenceSuspension,
    },
    Rule {
        needs: &[
            &["stop business"],
            &["cease business"],
            &["stop operations"],
            &["operations", "stopped"],
            &["closure", "order"],
            &["ordered", "shut"],
            &["shut down", "fda"],
        ],
        action: ActionType::StopBusiness,
    },
    Rule {
        needs: &[&["seal"]],
        action: ActionType::Sealing,
    },
    Rule {
        needs: &[&["seiz"]],
        action: ActionType::Seizure,
    },
    Rule {
        needs: &[&["reopen"]],
        action: ActionType::Reopened,
    },
    Rule {
        // Inspection needs an enforcement verb AND fda context: bare "fda"
        // alone passes quality-test reports, plan announcements and
        // awaiting-reply stories (6 Sept 2026 junk) that name no outlet
        // facing action.
        needs: &[
            &["raid", "fda"],
            &["raid", "food", "safety"],
            &["raid", "licence"],
            &["raid", "license"],
            &["inspect", "fda"],
            &["inspect", "food", "safety"],
            &["inspect", "licence"],
            &["inspect", "license"],
        ],
        action: ActionType::Inspection,
    },
];

/// Validated keyword signal: prose in, typed action out. Single-use plain
/// fn — the old Signal/TryFrom/From ceremony paid for reuse that never came.
fn signal(hay: &str) -> Option<ActionType> {
    RULES.iter().find_map(|rule| {
        rule.needs
            .iter()
            .any(|needles| needles.iter().all(|n| hay.contains(n)))
            .then_some(rule.action)
    })
}

// Only these corroborate a generic-English trigger: "fda" and
// licence/license are unambiguous enough. Deliberately excludes
// seal/seiz/raid/inspect — those ARE the ambiguous words, so they
// can't corroborate themselves.
const CORROBORATION: &[&str] = &["fda", "food safety", "licence", "license"];

/// Keyword signal plus a corroboration gate for ambiguous action types.
/// `hay` must already be lowercase: needle matching and the `CORROBORATION`
/// checks are case-sensitive `contains` calls. Callers build `hay` with
/// [`haystack`], which lowercases title + snippet + source.
pub(crate) fn triage(hay: &str) -> Option<ActionType> {
    let hay = hay.to_lowercase();
    let action = signal(&hay)?;
    let needs_corroboration = matches!(
        action,
        ActionType::StopBusiness
            | ActionType::Sealing
            | ActionType::Seizure
            | ActionType::Inspection
    );
    (!needs_corroboration || CORROBORATION.iter().any(|c| hay.contains(c))).then_some(action)
}

/// Cities that appear in Maharashtra FDA coverage, longest first so
/// "Navi Mumbai" wins over "Mumbai". Rule-extracted rows have no LLM to name
/// the city, so read it from the headline/body instead of leaving the
/// dashboard's area filters empty.
const CITIES: &[&str] = &[
    "Navi Mumbai",
    "Pimpri-Chinchwad",
    "Mahabaleshwar",
    "Aurangabad",
    "Chandrapur",
    "Ratnagiri",
    "Kolhapur",
    "Amravati",
    "Jalgaon",
    "Solapur",
    "Nanded",
    "Palghar",
    "Mumbai",
    "Thane",
    "Nashik",
    "Nagpur",
    "Pune",
    "Sangli",
    "Satara",
    "Akola",
    "Latur",
    "Dhule",
    "Kalyan",
    "Vasai",
    "Panvel",
];

pub(crate) fn city_of(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    CITIES
        .iter()
        .find(|city| lower.contains(&city.to_lowercase()))
        .map(|city| (*city).to_string())
}

fn clean_establishment_name(raw: &str) -> String {
    let mut s = raw.trim();
    for prefix in &[
        "Maharashtra FDA suspends food licences of",
        "Maharashtra FDA suspends food licence of",
        "Maharashtra FDA suspends licences of",
        "Maharashtra FDA suspends licence of",
        "Maharashtra FDA suspends",
        "Maharashtra FDA cancels",
        "Maharashtra FDA seals",
        "Maharashtra FDA seizes",
        "Maharashtra FDA raids",
        "Maharashtra FDA crackdown on",
        "Maharashtra FDA",
        "FDA suspends food licences of",
        "FDA suspends food licence of",
        "FDA suspends licences of",
        "FDA suspends licence of",
        "FDA suspends",
        "FDA seals",
        "FDA seizes",
        "FDA raids",
        "FDA",
    ] {
        if s.to_lowercase().starts_with(&prefix.to_lowercase()) {
            // Lowercasing can change byte length, so the prefix offset may not
            // be a char boundary in `s`; skip rather than panic on odd UTF-8.
            let Some(rest) = s.get(prefix.len()..) else {
                continue;
            };
            let rest = rest.trim().trim_start_matches([':', '-', ',', ' ']).trim();
            if !rest.is_empty() {
                s = rest;
                break;
            }
        }
    }
    s.to_string()
}

// Deterministic safety net: when a batch's LLM path fails, keep one minimal
// record per signal-bearing item so the run degrades instead of zeroing out.
// Fields the rules can't know (area, violations, dates) stay empty —
// build_rows + coerce_action_date fill date defaults downstream.
pub(crate) fn rule_extract(jobs: &[Job<'_>]) -> Vec<LlmAction> {
    jobs.iter()
        .filter_map(|j| {
            let action_type = triage(&haystack(j.item))?;
            let name: String = j
                .item
                .title
                .split(['|', '-', ':'])
                .next()?
                .trim()
                .chars()
                .take(120)
                .collect();
            let name = name.trim();
            (!name.is_empty()).then(|| {
                let cleaned_name = clean_establishment_name(name);
                let final_name = if cleaned_name.is_empty() {
                    name.to_string()
                } else {
                    cleaned_name
                };
                let mut action =
                    LlmAction::minimal(final_name, action_type, j.orig, j.item.snippet.clone());
                let city = city_of(&format!(
                    "{} {}",
                    j.item.title,
                    j.item.snippet.as_deref().unwrap_or("")
                ))
                .unwrap_or_else(|| "Mumbai".to_string());
                action.city = Some(city);
                if action
                    .details
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .chars()
                    .count()
                    < 20
                {
                    action.details = Some(format!("FDA enforcement action: {}", j.item.title));
                }
                action
            })
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
                orig: 7,
                item: &cricket,
            },
        ];
        let kept = rule_extract(&jobs);
        assert_eq!(kept.len(), 1, "only the signal item is kept");
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
