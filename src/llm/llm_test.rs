use super::{apply_collapse, extract, models_for, parse_collapse, verdict_article};
use crate::config::{Config, Llm, Persistence, Provider, SecretString};
use crate::domain::Article;
use crate::llm::triage::Verdict;
use crate::models::{ActionType, LlmAction, Sanitized};

fn config() -> Config {
    Config {
        persistence: Persistence::Postgres {
            url: SecretString::new(String::new()),
        },
        llm: Llm {
            provider: Provider::OpenRouter,
            key: SecretString::new("key".into()),
            model: "model".into(),
            fallbacks: Vec::new(),
        },
    }
}

fn article(title: &str) -> Article {
    Article {
        title: title.into(),
        url: format!("https://example.test/{title}"),
        source: None,
        published: None,
        snippet: None,
    }
}

#[test]
fn verdict_index_resolves_against_the_whole_item_list() {
    let items: Vec<Article> = (0..6)
        .map(|index| article(&format!("item-{index}")))
        .collect();
    let last = Verdict {
        source_index: 5,
        reason: None,
    };

    assert_eq!(
        verdict_article(&items, &last).map(|item| item.title.as_str()),
        Some("item-5"),
        "a single triage call uses absolute indices"
    );
    assert!(
        verdict_article(
            &items,
            &Verdict {
                source_index: 6,
                reason: None,
            }
        )
        .is_none()
    );
}

fn groq() -> Llm {
    Llm {
        provider: Provider::Groq,
        key: SecretString::new("key".into()),
        model: "qwen/qwen3.8-27b".into(),
        fallbacks: vec!["openai/gpt-oss-120b".into()],
    }
}

#[test]
fn groq_models_use_the_groq_namespace() {
    assert_eq!(
        models_for(&groq()),
        ["groq::qwen/qwen3.8-27b", "groq::openai/gpt-oss-120b"]
    );
}

#[test]
fn groq_disables_strict_json_schema() {
    assert!(!groq().structured_output());
    assert!(config().llm.structured_output());
}

#[test]
fn collapse_ignores_bad_and_singleton_groups() {
    let groups = parse_collapse(
        r#"{"groups":[
            {"ids":[0,2],"canonical":" National Sports Club of India "},
            {"ids":[4],"canonical":"singleton"},
            {"ids":[99,100],"canonical":"out of range"},
            {"ids":[-1],"canonical":"negative"}
        ]}"#,
    )
    .expect("parse collapse response");
    assert_eq!(
        groups,
        vec![
            (vec![0, 2], "National Sports Club of India".to_string()),
            (vec![99, 100], "out of range".to_string()),
        ],
        "singletons and negative ids are dropped; range is checked at apply time"
    );
    assert!(parse_collapse("not json").is_err());
}

#[test]
fn collapse_renames_every_record_in_a_group() {
    let action = LlmAction {
        establishment: "NSCI".into(),
        city: Some("Mumbai".into()),
        action_type: ActionType::LicenceSuspension,
        action_date: Some("2026-11-23".into()),
        details: Some(
            "licence suspended over cockroach infestation found during inspection".into(),
        ),
        ..Default::default()
    };
    let article = article("headline");
    let mut candidates: Vec<_> = (0..2)
        .map(|_| {
            crate::domain::Candidate::try_new(Sanitized::new(action.clone()), &article)
                .expect("valid candidate")
        })
        .collect();

    apply_collapse(
        &mut candidates,
        &[(vec![0, 1, 99], "National Sports Club of India".into())],
    );

    assert!(
        candidates
            .iter()
            .all(|c| c.establishment == "National Sports Club of India"),
        "both aliases become the canonical name so the store can dedup them; \
         an out-of-range id is ignored"
    );
}

#[tokio::test]
async fn empty_items_skip_llm() {
    let extraction = extract(&config(), &[]).await.unwrap();
    assert!(extraction.candidates.is_empty());
    assert_eq!(
        extraction.metrics.triage_calls + extraction.metrics.extraction_calls,
        0
    );
}
