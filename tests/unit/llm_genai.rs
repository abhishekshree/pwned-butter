use std::ptr::eq;

use super::*;
use crate::models::ActionType;

fn article(title: &str) -> Article {
    Article {
        title: title.into(),
        url: "https://example.test/1".into(),
        source: None,
        published: None,
        snippet: None,
    }
}

#[test]
fn triage_payload_omits_urls_and_caps_snippets() {
    let item = Article {
        title: "FDA suspends licence".into(),
        url: "https://example.test/a/long/path/that/wastes/tokens".into(),
        source: Some("Times".into()),
        published: None,
        snippet: Some("x".repeat(1000)),
    };
    let payload = triage_payload(&[item]).expect("serialize compact triage payload");
    assert!(
        !payload.contains("example.test"),
        "URLs cost tokens and the prompt forbids opening them"
    );
    let parsed: Value = serde_json::from_str(&payload).expect("valid JSON payload");
    assert_eq!(parsed["items"][0]["sourceIndex"], 0);
    assert_eq!(
        parsed["items"][0]["text"].as_str().unwrap().chars().count(),
        151,
        "snippet is capped at 150 chars plus the truncation ellipsis"
    );
}

#[test]
fn parses_fenced_actions_and_object_wrappers() {
    let actions = parse_llm_text(
        "```json\n{\"records\":[{\"establishment\":\"X\",\"actionType\":\"inspection\",\"sourceIndex\":1}]}\n```",
    )
    .unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].establishment, "X");
    assert_eq!(actions[0].action_type, ActionType::Inspection);
    assert_eq!(actions[0].source_index, 1);
    assert!(parse_llm_text("not json").is_err());
}

#[test]
fn sanitizes_strings_and_limits_lists() {
    let action: LlmAction = serde_json::from_value(json!({
        "establishment": "  X  ",
        "actionType": "inspection",
        "sourceIndex": 0,
        "violations": ["  issue  ", ""],
        "platforms": [" Swiggy ", " "]
    }))
    .unwrap();
    let action = sanitize_action(action).into_inner();
    assert_eq!(action.establishment, "X");
    assert_eq!(action.violations, vec!["issue"]);
    assert_eq!(action.platforms, vec!["swiggy"]);
}

#[test]
fn attaches_source_articles_and_drops_out_of_range() {
    let first = article("first");
    let second = article("second");
    let chunk: Vec<&Article> = vec![&first, &second];
    let actions = vec![
        LlmAction {
            source_index: 1,
            establishment: "B".into(),
            ..Default::default()
        },
        LlmAction {
            source_index: 0,
            establishment: "A".into(),
            ..Default::default()
        },
        LlmAction {
            source_index: 7,
            establishment: "out of range".into(),
            ..Default::default()
        },
    ];

    let result = absolute(actions, &chunk);

    assert_eq!(result.len(), 2, "out-of-range index is dropped");
    assert!(
        eq(result[0].0, &second),
        "chunk-local index 1 resolves to the second article"
    );
    assert_eq!(result[0].1.establishment, "B");
    assert_eq!(result[0].1.source_index, 1, "wire index is left untouched");
    assert!(
        eq(result[1].0, &first),
        "chunk-local index 0 resolves to the first article"
    );
    assert_eq!(result[1].1.establishment, "A");
    assert_eq!(result[1].1.source_index, 0, "wire index is left untouched");
}
