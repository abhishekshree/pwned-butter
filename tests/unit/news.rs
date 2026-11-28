use chrono::{Duration, Utc};

use super::{
    FetchedArticle, article_url, canonical, description_snippet, extract_snippet, fallback_snippet,
    select_for_enrichment,
};
use crate::models::NewsItem;

#[test]
fn extract_snippet_leads_with_title_then_paragraphs() {
    let html = "<html><head><title>FDA seals eatery</title></head>            <body><p>FDA officials sealed the eatery after finding serious hygiene violations and expired stock.</p></body></html>";
    let snippet = extract_snippet(html).expect("basic article yields a snippet");
    assert!(
        snippet.contains("FDA seals eatery"),
        "title leads the snippet"
    );
}

#[test]
fn extract_snippet_returns_none_for_empty_page() {
    assert_eq!(
        extract_snippet("<html><head></head><body></body></html>"),
        None
    );
}

#[test]
fn extract_snippet_rejects_google_news_landing_pages() {
    let html = "<html><head><title>Google News - FDA raid</title></head>            <body><p>Google News landing page for the FDA raid story with plenty of filler text here.</p></body></html>";
    assert_eq!(extract_snippet(html), None);
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
fn bing_description_strips_html_to_plain_text() {
    assert_eq!(
        description_snippet("<b>FDA</b> suspended the licence over &amp; pests.").as_deref(),
        Some("FDA suspended the licence over & pests."),
        "description supplies article text when the publisher page is blocked"
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

#[test]
fn fallback_snippet_drops_headline_echo() {
    let title = "FDA suspends six Mumbai licences";
    let item = NewsItem {
        title: title.into(),
        url: "https://news.google.com/original".into(),
        source: None,
        published: None,
        snippet: Some(format!("{title} - The 420.in")),
    };

    assert_eq!(
        fallback_snippet(&item),
        None,
        "a description that only repeats the title adds no evidence"
    );
}

#[test]
fn fallback_snippet_keeps_substantive_description() {
    let title = "FDA suspends six Mumbai licences";
    let detail = "Inspectors found cockroaches, uncovered dustbins and raw food stored with cooked food at six outlets including Blinkit in Fort.";
    let item = NewsItem {
        title: title.into(),
        url: "https://news.google.com/original".into(),
        source: None,
        published: None,
        snippet: Some(format!("{title} {detail}")),
    };

    assert_eq!(
        fallback_snippet(&item).as_deref(),
        Some(item.snippet.as_deref().expect("snippet set")),
        "real article text still survives a blocked publisher page"
    );
}
