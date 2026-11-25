use std::cmp::Reverse;
use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result as AnyResult, bail};
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

fn news_url(query: &str) -> String {
    format!(
        "https://www.bing.com/news/search?q={}&format=rss",
        encode(query)
    )
}

// Bing wraps publisher links in apiclick.aspx; the real URL is its `url` param.
fn article_url(link: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(link) else {
        return link.to_string();
    };
    if parsed.host_str() != Some("www.bing.com") || !parsed.path().ends_with("/apiclick.aspx") {
        return link.to_string();
    }
    if let Some((_, target)) = parsed.query_pairs().find(|(key, _)| key == "url") {
        return target.into_owned();
    }
    link.to_string()
}

fn description_snippet(raw: &str) -> Option<String> {
    let text = Html::parse_fragment(raw)
        .root_element()
        .text()
        .collect::<String>();
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Returns successful outputs and the number of tasks that panicked, so a
/// dropped query still counts as a failure instead of vanishing from the run.
async fn fan_out<T, Fut>(concurrency: usize, jobs: Vec<Fut>) -> (Vec<T>, usize)
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
    let mut dropped = 0;
    for h in handles {
        match h.await {
            Ok(v) => out.push(v),
            Err(e) => {
                error!("fan-out task failed: {e}");
                dropped += 1;
            }
        }
    }
    (out, dropped)
}

/// Reject a chunk that would push the accumulated body past MAX_ARTICLE_BYTES.
fn check_body_growth(accumulated: u64, chunk_len: usize) -> AnyResult<()> {
    let chunk_len = u64::try_from(chunk_len).context("chunk length overflow")?;
    let total = accumulated
        .checked_add(chunk_len)
        .context("body length overflow")?;
    if total > MAX_ARTICLE_BYTES {
        bail!("body too large");
    }
    Ok(())
}

/// Read incrementally: Content-Length is advisory and a missing/large value must not OOM the run.
async fn read_body(mut resp: reqwest::Response) -> AnyResult<Vec<u8>> {
    if resp.content_length().is_some_and(|n| n > MAX_ARTICLE_BYTES) {
        bail!("response too large");
    }
    let mut bytes = Vec::new();
    loop {
        let Some(chunk) = resp.chunk().await.context("read body chunk")? else {
            break;
        };
        let accumulated = u64::try_from(bytes.len()).context("body length overflow")?;
        check_body_growth(accumulated, chunk.len())?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
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
    let bytes = read_body(resp)
        .await
        .with_context(|| format!("rss feed: {url}"))?;
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
            url: article_url(link),
            source: it.source().and_then(|s| s.title().map(str::to_string)),
            published,
            snippet: it.description().and_then(description_snippet),
        });
    }
    Ok(items)
}

#[derive(Debug)]
pub(crate) struct FetchReport {
    pub items: Vec<NewsItem>,
    pub successful_queries: usize,
    pub failed_queries: usize,
}

/// Split fan-out results into items and errors, counting only completed successes.
fn collect_feeds<T>(
    results: impl IntoIterator<Item = AnyResult<Vec<T>>>,
) -> (Vec<T>, usize, Vec<anyhow::Error>) {
    let mut items = Vec::new();
    let mut successful = 0;
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(found) => {
                successful += 1;
                items.extend(found);
            }
            Err(e) => errors.push(e),
        }
    }
    (items, successful, errors)
}

#[instrument(skip(client))]
pub(crate) async fn fetch_items(client: &reqwest::Client) -> AnyResult<FetchReport> {
    let jobs = rules()
        .news
        .queries
        .iter()
        .map(|query| {
            let url = news_url(query);
            let client = (*client).clone();
            async move { fetch_feed(&client, &url).await }
        })
        .collect();
    let (results, dropped) = fan_out(RSS_CONCURRENCY, jobs).await;
    let (mut items, successful_queries, errors) = collect_feeds(results);
    let failed_queries = errors.len() + dropped;
    for e in errors {
        warn!("rss query failed: {e}");
    }
    let fetched = items.len();
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.url.clone()));
    info!(
        "rss: {fetched} hits from {} queries ({} ok, {} failed), {} distinct article URLs",
        rules().news.queries.len(),
        successful_queries,
        failed_queries,
        items.len()
    );
    Ok(FetchReport {
        items,
        successful_queries,
        failed_queries,
    })
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

/// Publisher redirects change the stored URL (source_url is the FINAL url), so
/// compare a parsed fingerprint: scheme, `www.`, fragment, trailing slash and
/// `/amp` are redirect noise. Host is case-insensitive; path and query are not,
/// so they are left exactly as the URL parser returns them.
fn url_fingerprint(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url.trim()) else {
        return format!("!{}", url.trim());
    };
    parsed.set_fragment(None);
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let path = parsed.path().trim_end_matches('/');
    let path = path.strip_suffix("/amp").unwrap_or(path);
    match parsed.query() {
        Some(query) if !query.is_empty() => format!("{host}{path}?{query}"),
        _ => format!("{host}{path}"),
    }
}

/// One dedup decision for both the RSS candidate and the final redirected URL.
fn keep_new_url(url: &str, seen_keys: &HashSet<String>, inserted: &mut HashSet<String>) -> bool {
    let key = url_fingerprint(url);
    !seen_keys.contains(&key) && inserted.insert(key)
}

/// Filter before ranking so already-seen articles never consume the max_items budget.
fn select_unseen(items: Vec<NewsItem>, seen: &HashSet<String>, max_items: usize) -> Vec<NewsItem> {
    let seen_keys: HashSet<String> = seen.iter().map(|url| url_fingerprint(url)).collect();
    let fresh = items
        .into_iter()
        .filter(|item| !seen_keys.contains(&url_fingerprint(&item.url)))
        .collect();
    select_for_enrichment(fresh, max_items)
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
    let bytes = read_body(resp)
        .await
        .with_context(|| format!("article: {url}"))?;
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
) -> (Vec<Article>, usize) {
    let available = items.len();
    let seen_keys: HashSet<String> = seen.iter().map(|url| url_fingerprint(url)).collect();
    let already_seen = items
        .iter()
        .filter(|item| seen_keys.contains(&url_fingerprint(&item.url)))
        .count();
    let items = select_unseen(items, seen, max_items);
    info!(
        "article enrichment: selected {}/{} highest-signal RSS items ({} already seen)",
        items.len(),
        available,
        already_seen
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
    let (mut results, dropped) = fan_out(FETCH_CONCURRENCY, jobs).await;
    results.sort_by_key(|(i, _, _)| *i);

    let mut failures = dropped;
    let mut out: Vec<Article> = Vec::with_capacity(results.len());
    let mut urls: HashSet<String> = HashSet::new();
    for (_, item, res) in results {
        let fetched = match res {
            Ok(fetched) => Some(fetched),
            Err(e) => {
                failures += 1;
                warn!("article fetch failed ({}): {e}", item.url);
                None
            }
        };
        let (url, snippet) = canonical(&item, fetched);
        if !keep_new_url(&url, &seen_keys, &mut urls) {
            continue;
        }
        out.push(Article::new(item, url, snippet));
    }
    out.sort_by(|a, b| {
        b.published
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
            .cmp(&a.published.unwrap_or(DateTime::<Utc>::MIN_UTC))
    });
    (out, failures)
}

#[cfg(test)]
mod tests {
    use super::{
        FetchedArticle, MAX_ARTICLE_BYTES, article_url, canonical, check_body_growth,
        collect_feeds, description_snippet, extract_snippet, keep_new_url, select_for_enrichment,
        select_unseen, url_fingerprint,
    };
    use std::collections::HashSet;

    use anyhow::anyhow;
    use chrono::{Duration, Utc};

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
    fn body_growth_stops_at_the_byte_cap_and_on_overflow() {
        assert!(
            check_body_growth(MAX_ARTICLE_BYTES - 1, 1).is_ok(),
            "a body exactly at the cap is allowed"
        );
        assert!(
            check_body_growth(MAX_ARTICLE_BYTES, 1).is_err(),
            "one byte past the cap is rejected"
        );
        assert!(
            check_body_growth(u64::MAX, 1).is_err(),
            "length overflow is rejected"
        );
    }

    #[test]
    fn collect_feeds_counts_partial_rss_failures() {
        let (items, successful, errors) = collect_feeds(vec![
            Ok(vec!["first"]),
            Err(anyhow!("503")),
            Ok(vec![]),
            Err(anyhow!("timeout")),
        ]);

        assert_eq!(
            items,
            vec!["first"],
            "only successful feeds contribute items"
        );
        assert_eq!(successful, 2, "empty successes still count as successful");
        assert_eq!(errors.len(), 2, "each failed query is counted once");
    }

    #[test]
    fn seen_items_do_not_consume_the_enrichment_budget() {
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
            url: "http://thehindu.com/news/action/".into(),
            source: None,
            published: Some(now - Duration::hours(2)),
            snippet: None,
        };
        // Stored source_url is the final redirected URL; the RSS item is not.
        let seen = HashSet::from(["https://www.thehindu.com/news/action".to_string()]);

        let selected = select_unseen(vec![generic, action], &seen, 1);

        assert_eq!(
            selected[0].url, "https://example.test/generic",
            "a redirected, already-seen item must not take the only slot"
        );
    }

    #[test]
    fn url_fingerprint_ignores_common_redirects_only() {
        assert_eq!(
            url_fingerprint("http://www.thehindu.com/news/story/?utm=1#top"),
            url_fingerprint("https://thehindu.com/news/story/?utm=1"),
            "scheme, www and fragment are redirect noise"
        );
        assert_ne!(
            url_fingerprint("https://thehindu.com/news/story-a"),
            url_fingerprint("https://thehindu.com/news/story-b"),
            "different stories must never share a fingerprint"
        );
        assert_ne!(
            url_fingerprint("https://thehindu.com/news/Story"),
            url_fingerprint("https://thehindu.com/news/story"),
            "path case is significant"
        );
        assert_ne!(
            url_fingerprint("https://thehindu.com/news/story?tag=A"),
            url_fingerprint("https://thehindu.com/news/story?tag=a"),
            "query case is significant"
        );
        assert_eq!(
            url_fingerprint("https://EXAMPLE.test/Story"),
            url_fingerprint("https://example.test/Story"),
            "host case is not significant"
        );
    }

    #[test]
    fn final_redirect_url_uses_the_same_fingerprint_as_the_seen_set() {
        let seen_keys = HashSet::from([url_fingerprint("http://thehindu.com/news/action/")]);
        let mut inserted = HashSet::new();
        assert!(
            !keep_new_url(
                "https://www.thehindu.com/news/action",
                &seen_keys,
                &mut inserted
            ),
            "a redirected final URL must be recognised as seen"
        );
        assert!(
            keep_new_url(
                "https://www.thehindu.com/news/other",
                &seen_keys,
                &mut inserted
            ),
            "a genuinely new final URL must survive"
        );
        assert!(
            !keep_new_url("https://thehindu.com/news/other", &seen_keys, &mut inserted),
            "two final URLs for one article are deduped together"
        );
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
        assert_eq!(
            snippet.as_deref(),
            Some("FDA seals eatery"),
            "failure degrades to headline"
        );
    }
}
