use super::{extract, models_for, verdict_article};
use crate::config::{Config, Llm, Persistence, Provider, SecretString};
use crate::domain::Article;
use crate::llm::triage::Verdict;

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

#[tokio::test]
async fn empty_items_skip_llm() {
    let extraction = extract(&config(), &[]).await.unwrap();
    assert!(extraction.candidates.is_empty());
    assert_eq!(
        extraction.metrics.triage_calls + extraction.metrics.extraction_calls,
        0
    );
}
