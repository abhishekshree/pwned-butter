use std::cmp::Reverse;
use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use chrono::{DateTime, Utc};
use rss::Channel;
use scraper::{Html, Selector};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{error, info, instrument, warn};
use urlencoding::encode;

use crate::domain::Article;
use crate::models::NewsItem;
use crate::rules::rules;

pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub(crate) const MAX_ITEMS: usize = 100;
const FETCH_CONCURRENCY: usize = 8;
/// One huge publisher page can OOM the run before the snippet is truncated.
const MAX_ARTICLE_BYTES: u64 = 5 * 1024 * 1024;
// Google 503s when all queries hit from one IP at once.
const RSS_CONCURRENCY: usize = 4;

fn google_news_url(query: &str, window: Option<&str>) -> String {
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
    Fut: Future<Output = T> + Send + 'static,
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
            Err(e) => error!("fan-out task failed: {e}"),
        }
    }
    out
}

/// One retry covers transient 503 storms; a sustained block shows up as seen=0.
async fn fetch_feed(client: &reqwest::Client, url: &str) -> AnyResult<Vec<NewsItem>> {
    match fetch_feed_once(client, url).await {
        Ok(items) => Ok(items),
        Err(first) => {
            sleep(Duration::from_secs(10)).await;
            fetch_feed_once(client, url)
                .await
                .map_err(|second| second.context(format!("after retry; first: {first:#}")))
        }
    }
}

async fn fetch_feed_once(client: &reqwest::Client, url: &str) -> AnyResult<Vec<NewsItem>> {
    let resp = client
        .get(url)
        .send()
        .await
        .context("rss get")?
        .error_for_status()?;
    if resp.content_length().is_some_and(|n| n > MAX_ARTICLE_BYTES) {
        anyhow::bail!("rss feed too large: {url}");
    }
    let bytes = resp.bytes().await.context("rss bytes")?;
    if u64::try_from(bytes.len()).is_ok_and(|len| len > MAX_ARTICLE_BYTES) {
        anyhow::bail!("rss feed too large: {url}");
    }
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

#[instrument(skip(client), fields(window = %window))]
pub(crate) async fn fetch_items(
    client: &reqwest::Client,
    window: &str,
) -> AnyResult<Vec<NewsItem>> {
    let jobs = rules()
        .news
        .queries
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
            Err(e) => warn!("rss query failed: {e}"),
        }
    }
    let fetched = items.len();
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.url.clone()));
    info!(
        "rss: {fetched} hits from {} queries, {} distinct Google News URLs",
        rules().news.queries.len(),
        items.len()
    );
    Ok(items)
}

fn enforcement_score(item: &NewsItem) -> u32 {
    let config = &rules().news;
    let hay = item.haystack();
    let any = |words: &[String]| words.iter().any(|w| hay.contains(w));
    let mut score = 8 * u32::from(any(&config.authority_signals));
    score = score
        .saturating_add(3 * u32::from(any(&config.licence_words) && any(&config.licence_actions)));
    let action_hits = u32::try_from(
        config
            .action_signals
            .iter()
            .filter(|s| hay.contains(*s))
            .count(),
    )
    .unwrap_or(u32::MAX);
    score = score.saturating_add(action_hits.saturating_mul(3));
    score = score.saturating_add(u32::from(any(&config.venue_signals)));
    score
}

fn select_for_enrichment(mut items: Vec<NewsItem>, max_items: usize) -> Vec<NewsItem> {
    items.sort_by_cached_key(|item| {
        (
            Reverse(enforcement_score(item)),
            Reverse(item.published.unwrap_or(DateTime::<Utc>::MIN_UTC)),
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
            (t.len() >= 40).then(|| t.to_string())
        })
        .collect();
    // Roundups put the full outlet list after the opening paragraphs.
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

async fn fetch_article(client: &reqwest::Client, url: &str) -> AnyResult<FetchedArticle> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("article get: {url}"))?
        .error_for_status()?;
    let final_url = resp.url().to_string();
    if resp.content_length().is_some_and(|n| n > MAX_ARTICLE_BYTES) {
        anyhow::bail!("article too large: {url}");
    }
    let bytes = resp.bytes().await.context("article body")?;
    // Content-Length is advisory; reject an oversized body after the read too.
    if u64::try_from(bytes.len()).is_ok_and(|len| len > MAX_ARTICLE_BYTES) {
        anyhow::bail!("article too large: {url}");
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(FetchedArticle {
        final_url,
        snippet: extract_snippet(&text),
    })
}

fn canonical(item: &NewsItem, fetched: Option<FetchedArticle>) -> (String, Option<String>) {
    match fetched {
        Some(FetchedArticle { final_url, snippet }) => (final_url, snippet),
        None => {
            let snippet = item.snippet.clone().or_else(|| Some(item.title.clone()));
            (item.url.clone(), snippet)
        }
    }
}

#[instrument(skip(client, items, seen), fields(max_items = max_items, available = items.len()))]
pub(crate) async fn enrich(
    client: &reqwest::Client,
    items: Vec<NewsItem>,
    seen: &HashSet<String>,
    max_items: usize,
) -> Vec<Article> {
    let available = items.len();
    let items = select_for_enrichment(items, max_items);
    info!(
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
                (i, item, res)
            }
        })
        .collect();
    let mut results = fan_out(FETCH_CONCURRENCY, jobs).await;
    results.sort_by_key(|(i, _, _)| *i);

    let mut out: Vec<Article> = Vec::with_capacity(results.len());
    let mut urls: HashSet<String> = HashSet::new();
    for (_, item, res) in results {
        let fetched = match res {
            Ok(fetched) => Some(fetched),
            Err(e) => {
                warn!("article fetch failed ({}): {e}", item.url);
                None
            }
        };
        let (url, snippet) = canonical(&item, fetched);
        if urls.contains(&url) || seen.contains(&url) {
            continue;
        }
        urls.insert(url.clone());
        out.push(Article::new(item, url, snippet));
    }
    out.sort_by(|a, b| {
        b.published
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
            .cmp(&a.published.unwrap_or(DateTime::<Utc>::MIN_UTC))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::{FetchedArticle, canonical, extract_snippet, select_for_enrichment};
    use crate::models::NewsItem;
    use chrono::{Duration, Utc};

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
        assert_eq!(
            snippet.as_deref(),
            Some("FDA seals eatery"),
            "failure degrades to headline"
        );
    }
}
