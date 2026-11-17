use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rss::Channel;
use scraper::{Html, Selector};
use tokio::sync::Semaphore;
use urlencoding::encode;

use crate::models::NewsItem;

pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

const QUERIES: &[&str] = &[
    "\"Maharashtra FDA\"",
    "\"Maharashtra Food and Drug Administration\"",
    "\"FDA Mumbai\"",
    "\"Tukaram Mundhe\"",
    "\"Maharashtra FDA\" licence suspended",
    "\"Maharashtra FDA\" licence cancelled",
    "\"Maharashtra FDA\" seal",
    "\"Maharashtra FDA\" seizure",
    "\"Maharashtra FDA\" raid",
    "\"Maharashtra FDA\" restaurant hygiene",
    "\"Maharashtra FDA\" hotel dhaba eatery",
    "\"Maharashtra FDA\" \"improvement notice\"",
    "\"Maharashtra FDA\" expired cockroach",
    "\"Maharashtra FDA\" milk adulteration",
    "\"Maharashtra FDA\" chain restaurant",
    "Dominos OR \"Pizza Hut\" OR \"Burger King\" FDA Maharashtra",
    "KFC OR McDonalds OR Starbucks FDA licence Maharashtra",
    "Blinkit OR Zepto OR \"Swiggy Instamart\" FDA licence suspended",
    "Zomato OR Swiggy \"cloud kitchen\" FDA",
    "\"Maharashtra FDA\" Mumbai food safety",
    "\"Maharashtra FDA\" Pune",
    "\"Maharashtra FDA\" Nashik OR Thane OR Nagpur OR Aurangabad",
    "\"licence suspended\" \"Safe Food\" Maharashtra restaurant",
    "\"Maharashtra FDA\" prosecution",
    "\"Food Safety and Standards\" Maharashtra raid licence",
    "site:x.com \"Maharashtra FDA\"",
    "site:twitter.com \"Maharashtra FDA\"",
    "site:x.com \"Maharashtra FDA\" raid OR licence OR suspend",
    "site:x.com FDA Mumbai raid",
    "site:x.com \"Tukaram Mundhe\"",
    "site:x.com Mumbai restaurant FDA hygiene",
    "site:x.com Blinkit OR Zepto OR Instamart FDA",
];

/// The number of distinct, high-signal articles to enrich in one scheduled run.
///
/// A single FDA press note can name dozens of outlets, so dropping a relevant
/// roundup is much more costly than reading a few more articles. This is
/// deliberately applied before article fetching: Google News returns the same
/// story from several queries and fetching all of them used to spend the run's
/// time budget before the later truncation discarded them.
pub const MAX_ITEMS: usize = 100;
const FETCH_CONCURRENCY: usize = 8;
// Google answers all 32 queries from one runner IP at once with 503s when we
// fan out too wide; four at a time keeps the burst below the threshold.
const RSS_CONCURRENCY: usize = 4;

pub fn google_news_url(query: &str, window: Option<&str>) -> String {
    let q = match window {
        Some(w) if !w.is_empty() => format!("{query} {w}"),
        _ => query.to_string(),
    };
    format!(
        "https://news.google.com/rss/search?q={0}&hl=en-IN&gl=IN&ceid=IN:en",
        encode(&q)
    )
}

async fn fan_out<T, Fut>(concurrency: usize, jobs: Vec<Fut>) -> Vec<T>
where
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(jobs.len());
    for job in jobs {
        let sem = Arc::clone(&sem);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            job.await
        }));
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        match h.await {
            Ok(v) => out.push(v),
            Err(e) => eprintln!("fan-out task failed: {e}"),
        }
    }
    out
}

/// One retry covers Google News' transient 503 storms; a sustained IP-level
/// block still surfaces as `seen=0` in the run summary.
async fn fetch_feed(client: &reqwest::Client, url: &str) -> Result<Vec<NewsItem>> {
    match fetch_feed_once(client, url).await {
        Ok(items) => Ok(items),
        Err(first) => {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            fetch_feed_once(client, url)
                .await
                .map_err(|second| second.context(format!("after retry; first: {first:#}")))
        }
    }
}

async fn fetch_feed_once(client: &reqwest::Client, url: &str) -> Result<Vec<NewsItem>> {
    let resp = client
        .get(url)
        .send()
        .await
        .context("rss get")?
        .error_for_status()?;
    let bytes = resp.bytes().await.context("rss bytes")?;
    let channel = Channel::read_from(&bytes[..]).context("rss parse")?;
    let mut items = Vec::new();
    for it in channel.into_items() {
        let Some(title) = it.title() else { continue };
        let title = title.trim();
        if title.is_empty() {
            continue;
        }
        let Some(link) = it.link() else { continue };
        let link = link.trim();
        if link.is_empty() {
            continue;
        }
        let published = it
            .pub_date()
            .and_then(|d| DateTime::parse_from_rfc2822(d).ok())
            .map(|dt| dt.with_timezone(&Utc));
        items.push(NewsItem {
            title: title.to_string(),
            url: link.to_string(),
            source: it.source().and_then(|s| s.title().map(str::to_string)),
            published,
            snippet: None,
        });
    }
    Ok(items)
}

pub async fn fetch_items(client: &reqwest::Client, window: &str) -> Result<Vec<NewsItem>> {
    let jobs = QUERIES
        .iter()
        .map(|query| {
            let url = google_news_url(query, (!window.is_empty()).then_some(window));
            let client = (*client).clone();
            async move { fetch_feed(&client, &url).await }
        })
        .collect();
    let mut items: Vec<NewsItem> = Vec::new();
    for res in fan_out(RSS_CONCURRENCY, jobs).await {
        match res {
            Ok(found) => items.extend(found),
            Err(e) => eprintln!("rss query failed: {e}"),
        }
    }
    let fetched = items.len();
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.url.clone()));
    eprintln!(
        "rss: {fetched} hits from {} queries, {} distinct Google News URLs",
        QUERIES.len(),
        items.len()
    );
    Ok(items)
}

/// Rank likely enforcement stories from their RSS metadata before downloading
/// publisher pages. This is only a priority signal, never an exclusion: a
/// lower-scoring item still gets selected when there is room.
fn enforcement_score(item: &NewsItem) -> u8 {
    let haystack = format!(
        "{} {} {}",
        item.title,
        item.snippet.as_deref().unwrap_or(""),
        item.source.as_deref().unwrap_or("")
    )
    .to_lowercase();

    let mut score = 0;
    if haystack.contains("maharashtra fda")
        || haystack.contains("food and drug administration")
        || haystack.contains("tukaram mundhe")
    {
        score += 8;
    }
    let has_licence = haystack.contains("licence") || haystack.contains("license");
    let has_licence_action =
        haystack.contains("suspend") || haystack.contains("cancel") || haystack.contains("revok");
    if has_licence && has_licence_action {
        score += 3;
    }
    for signal in [
        "ordered to shut",
        "ordered to stop",
        "cease business",
        "stop operations",
        "sealed",
        "seized",
    ] {
        if haystack.contains(signal) {
            score += 3;
        }
    }
    if haystack.contains("restaurant")
        || haystack.contains("hotel")
        || haystack.contains("cafe")
        || haystack.contains("caterer")
        || haystack.contains("dark store")
        || haystack.contains("quick-commerce")
    {
        score += 1;
    }
    score
}

fn select_for_enrichment(mut items: Vec<NewsItem>, max_items: usize) -> Vec<NewsItem> {
    // enforcement_score lowercases the whole haystack, so compute it once per
    // item via sort_by_cached_key instead of on every comparison.
    items.sort_by_cached_key(|item| {
        (
            std::cmp::Reverse(enforcement_score(item)),
            std::cmp::Reverse(item.published.unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)),
            item.url.clone(),
        )
    });
    items.truncate(max_items);
    items
}

static OG_TITLE_SEL: OnceLock<Selector> = OnceLock::new();
static TITLE_SEL: OnceLock<Selector> = OnceLock::new();
static H1_SEL: OnceLock<Selector> = OnceLock::new();
static P_SEL: OnceLock<Selector> = OnceLock::new();

fn first_text(html: &Html, sel: &Selector) -> Option<String> {
    html.select(sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn extract_snippet(document: &str) -> Option<String> {
    let html = Html::parse_document(document);
    let og_title =
        OG_TITLE_SEL.get_or_init(|| Selector::parse("meta[property='og:title']").expect("static"));
    let title_sel = TITLE_SEL.get_or_init(|| Selector::parse("title").expect("static"));
    let h1_sel = H1_SEL.get_or_init(|| Selector::parse("h1").expect("static"));
    let p_sel = P_SEL.get_or_init(|| Selector::parse("p").expect("static"));

    let mut title = html
        .select(og_title)
        .next()
        .and_then(|e| e.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| first_text(&html, title_sel))
        .or_else(|| first_text(&html, h1_sel));

    let mut paras: Vec<String> = html
        .select(p_sel)
        .filter_map(|e| {
            let t = e.text().collect::<String>();
            let t = t.trim();
            if t.len() >= 40 {
                Some(t.to_string())
            } else {
                None
            }
        })
        .collect();
    // Roundups often put the complete list of affected outlets after the
    // opening paragraphs. Keep enough article text for the extractor to see
    // all names while remaining comfortably below a Flash batch context.
    paras.truncate(30);

    if title.is_none() && paras.is_empty() {
        return None;
    }
    let mut body = title.take().unwrap_or_default();
    if !paras.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&paras.join(" "));
    }
    if body.chars().count() > 12_000 {
        body = body.chars().take(12_000).collect();
    }
    if body.to_lowercase().starts_with("google news") || body.trim().is_empty() {
        return None;
    }
    Some(body)
}

struct FetchedArticle {
    final_url: String,
    snippet: Option<String>,
}

struct EnrichResult(Result<FetchedArticle>);

async fn fetch_article(client: &reqwest::Client, url: &str) -> Result<FetchedArticle> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("article get: {url}"))?
        .error_for_status()?;
    let final_url = resp.url().to_string();
    let bytes = resp.bytes().await.context("article body")?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(FetchedArticle {
        final_url,
        snippet: extract_snippet(&text),
    })
}

fn already_seen(urls: &HashSet<String>, seen: &HashSet<String>, url: &str) -> bool {
    urls.contains(url) || seen.contains(url)
}

pub async fn enrich(
    client: &reqwest::Client,
    items: Vec<NewsItem>,
    seen: &HashSet<String>,
    max_items: usize,
) -> Vec<NewsItem> {
    let available = items.len();
    let items = select_for_enrichment(items, max_items);
    eprintln!(
        "article enrichment: selected {}/{} highest-signal RSS items",
        items.len(),
        available
    );
    let jobs = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let client = (*client).clone();
            let url = item.url.clone();
            async move {
                let res = fetch_article(&client, &url).await;
                (i, item, EnrichResult(res))
            }
        })
        .collect();
    let mut results = fan_out(FETCH_CONCURRENCY, jobs).await;
    results.sort_by_key(|(i, _, _)| *i);

    let mut out: Vec<NewsItem> = Vec::with_capacity(results.len());
    let mut urls: HashSet<String> = HashSet::new();
    for (_, mut item, EnrichResult(res)) in results {
        match res {
            Ok(FetchedArticle { final_url, snippet }) => {
                if already_seen(&urls, seen, &final_url) {
                    continue;
                }
                urls.insert(final_url.clone());
                item.url = final_url;
                item.snippet = snippet;
            }
            Err(e) => {
                eprintln!("article fetch failed ({}): {e}", item.url);
                if already_seen(&urls, seen, &item.url) {
                    continue;
                }
                urls.insert(item.url.clone());
                if item.snippet.is_none() {
                    item.snippet = Some(item.title.clone());
                }
            }
        }
        out.push(item);
    }
    out.sort_by(|a, b| {
        b.published
            .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
            .cmp(&a.published.unwrap_or(chrono::DateTime::<Utc>::MIN_UTC))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::{already_seen, extract_snippet, select_for_enrichment};
    use crate::models::NewsItem;
    use chrono::{Duration, Utc};
    use std::collections::HashSet;

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
    fn already_seen_dedups_urls() {
        let urls: HashSet<String> = ["https://a.test/1".to_string()].into_iter().collect();
        let seen: HashSet<String> = ["https://seen.test/9".to_string()].into_iter().collect();
        assert!(
            already_seen(&urls, &seen, "https://a.test/1"),
            "in-batch url is seen"
        );
        assert!(
            already_seen(&urls, &seen, "https://seen.test/9"),
            "db-seen url is seen"
        );
        assert!(
            !already_seen(&urls, &seen, "https://fresh.test/2"),
            "fresh url passes"
        );
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
}
