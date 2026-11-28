use super::*;

#[test]
fn parses_only_valid_verdicts() {
    let text = r#"[{"sourceIndex":3,"reason":"licence suspended over pests"},
        {"sourceIndex":4},
        {"reason":"no index"},
        {"sourceIndex":"bad","reason":"nope"}]"#;
    let verdicts = parse_verdicts(text).unwrap();
    assert_eq!(
        verdicts.len(),
        2,
        "records without a numeric sourceIndex are dropped"
    );
    assert_eq!(verdicts[0].source_index, 3);
    assert_eq!(
        verdicts[0].reason.as_deref(),
        Some("licence suspended over pests")
    );
    assert_eq!(verdicts[1].source_index, 4);
    assert_eq!(verdicts[1].reason, None, "reason defaults to None");
}
