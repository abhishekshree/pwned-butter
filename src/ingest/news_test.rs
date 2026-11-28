use chrono::{Duration, Utc};
use insta::{assert_snapshot, glob};
use test_case::test_case;

use super::{
    FetchedArticle, article_url, canonical, description_snippet, extract_snippet, fallback_snippet,
    select_for_enrichment,
};
use crate::models::NewsItem;

#[test]
fn html_snippets() {
    glob!("fixtures/*.html", |path| {
        let html = std::fs::read_to_string(path).expect("read html fixture");
        assert_snapshot!(format!(
            "extract: {:?}\ndescription: {:?}",
            extract_snippet(&html),
            description_snippet(&html)
        ));
    });
}

#[test]
fn prioritises_explicit_enforcement_before_generic_news() {
    let now = Utc::now();
    let generic = NewsItem {
        title: "Maharashtra food trends this Diwali".into(),
        url: "https://example.test/generic".into(),
        source: None,
        published: Some(now),
        snippet: None,
    };
    let action = NewsItem {
        title: "Maharashtra FDA orders restaurant to cease business after raid".into(),
        url: "https://example.test/action".into(),
        source: None,
        published: Some(now - Duration::hours(2)),
        snippet: None,
    };
    let selected = select_for_enrichment(vec![generic, action], 1);
    assert_eq!(selected[0].url, "https://example.test/action");
}

#[test]
fn bing_apiclick_link_resolves_to_publisher_url() {
    let link = "https://www.bing.com/news/apiclick.aspx?ref=FexRss&url=https%3a%2f%2fwww.thehindu.com%2fnews%2ftest";
    assert_eq!(
        article_url(link),
        "https://www.thehindu.com/news/test",
        "publisher URL must be unwrapped before fetching"
    );
}

#[test]
fn canonical_uses_final_url_on_success_and_original_url_on_failure() {
    let item = NewsItem {
        title: "FDA seals eatery".into(),
        url: "https://news.google.com/original".into(),
        source: None,
        published: None,
        snippet: None,
    };

    let (url, snippet) = canonical(
        &item,
        Some(FetchedArticle {
            final_url: "https://publisher.test/story".into(),
            snippet: Some("extracted snippet".into()),
        }),
    );
    assert_eq!(
        url, "https://publisher.test/story",
        "success uses final URL"
    );
    assert_eq!(
        snippet.as_deref(),
        Some("extracted snippet"),
        "success uses fetched snippet"
    );

    let (url, snippet) = canonical(&item, None);
    assert_eq!(
        url, "https://news.google.com/original",
        "failure keeps original URL"
    );
    assert_eq!(snippet, None, "headline alone is not enough evidence");
}

#[test_case("FDA suspends six Mumbai licences - The 420.in", None; "headline echo adds no evidence")]
#[test_case("FDA suspends six Mumbai licences Inspectors found cockroaches, uncovered dustbins and raw food stored with cooked food at six outlets including Blinkit in Fort.", Some("FDA suspends six Mumbai licences Inspectors found cockroaches, uncovered dustbins and raw food stored with cooked food at six outlets including Blinkit in Fort."); "substantive description survives")]
fn fallback_snippet_uses_only_substantive_descriptions(snippet: &str, expected: Option<&str>) {
    let item = NewsItem {
        title: "FDA suspends six Mumbai licences".into(),
        url: "https://news.google.com/original".into(),
        source: None,
        published: None,
        snippet: Some(snippet.into()),
    };

    assert_eq!(fallback_snippet(&item).as_deref(), expected);
}
