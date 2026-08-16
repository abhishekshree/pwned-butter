use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rss::Channel;
use scraper::{Html, Selector};
use tokio::sync::Semaphore;
use urlencoding::encode;

use crate::models::NewsItem;

pub const USER_AGENT: &str = "Mozilla/5.0 (compatible; fda-mumbai-tracker/0.1; news aggregator)";

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
];

pub const MAX_ITEMS: usize = 50;
pub const FETCH_CONCURRENCY: usize = 8;
pub const RSS_CONCURRENCY: usize = 8;

fn rss_concurrency() -> usize {
    std::env::var("FDA_RSS_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RSS_CONCURRENCY)
}

fn fetch_concurrency() -> usize {
    std::env::var("FDA_FETCH_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FETCH_CONCURRENCY)
}

pub fn google_news_url(query: &str, window: &str) -> String {
    let q = if window.is_empty() {
        query.to_string()
    } else {
        format!("{query} {window}")
    };
    format!(
        "https://news.google.com/rss/search?q={0}&hl=en-IN&gl=IN&ceid=IN:en",
        encode(&q)
    )
}

async fn fetch_feed(client: &reqwest::Client, url: &str) -> Result<Vec<NewsItem>> {
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
    let sem = Arc::new(Semaphore::new(rss_concurrency()));
    let mut tasks = Vec::new();
    for query in QUERIES {
        let url = google_news_url(query, window);
        let client = (*client).clone();
        let sem = Arc::clone(&sem);
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            fetch_feed(&client, &url).await
        }));
    }
    let mut items: Vec<NewsItem> = Vec::new();
    for t in tasks {
        match t.await {
            Ok(Ok(found)) => items.extend(found),
            Ok(Err(e)) => eprintln!("rss query failed: {e}"),
            Err(e) => eprintln!("rss task failed: {e}"),
        }
    }
    Ok(items)
}

fn extract_snippet(document: &str) -> Option<String> {
    let html = Html::parse_document(document);
    let og_title = Selector::parse("meta[property='og:title']").ok()?;
    let title_sel = Selector::parse("title").ok()?;
    let h1_sel = Selector::parse("h1").ok()?;
    let p_sel = Selector::parse("p").ok()?;

    let mut title = html
        .select(&og_title)
        .next()
        .and_then(|e| e.value().attr("content").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            html.select(&title_sel)
                .next()
                .map(|e| e.text().collect::<String>())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            html.select(&h1_sel)
                .next()
                .map(|e| e.text().collect::<String>())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    let mut paras: Vec<String> = html
        .select(&p_sel)
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
    paras.truncate(8);

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
    if body.chars().count() > 2500 {
        body = body.chars().take(2500).collect();
    }
    if body.to_lowercase().starts_with("google news") || body.trim().is_empty() {
        return None;
    }
    Some(body)
}

async fn fetch_article(client: &reqwest::Client, url: &str) -> Result<(String, Option<String>)> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("article get: {url}"))?
        .error_for_status()?;
    let final_url = resp.url().to_string();
    let bytes = resp.bytes().await.context("article body")?;
    let text = String::from_utf8_lossy(&bytes);
    Ok((final_url, extract_snippet(&text)))
}

pub async fn enrich(
    client: &reqwest::Client,
    items: Vec<NewsItem>,
    seen: &HashSet<String>,
    max_items: usize,
) -> Vec<NewsItem> {
    let sem = Arc::new(Semaphore::new(fetch_concurrency()));
    let mut tasks = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        let sem = Arc::clone(&sem);
        let client = (*client).clone();
        let url = item.url.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let res = fetch_article(&client, &url).await;
            (i, item, res)
        }));
    }

    let mut results: Vec<(usize, NewsItem, Result<(String, Option<String>)>)> =
        Vec::with_capacity(tasks.len());
    for t in tasks {
        match t.await {
            Ok(res) => results.push(res),
            Err(e) => eprintln!("enrich task join error: {e}"),
        }
    }
    results.sort_by_key(|(i, _, _)| *i);

    let mut out: Vec<NewsItem> = Vec::with_capacity(results.len());
    let mut urls: HashSet<String> = HashSet::new();
    for (_, mut item, res) in results {
        match res {
            Ok((final_url, snippet)) => {
                if urls.contains(&final_url) || seen.contains(&final_url) {
                    continue;
                }
                urls.insert(final_url.clone());
                item.url = final_url;
                item.snippet = snippet;
            }
            Err(e) => {
                eprintln!("article fetch failed ({}): {e}", item.url);
                if urls.contains(&item.url) || seen.contains(&item.url) {
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
    out.truncate(max_items);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, snippet: Option<&str>) -> NewsItem {
        NewsItem {
            title: title.into(),
            url: "https://t.test/x".into(),
            source: None,
            published: None,
            snippet: snippet.map(str::to_string),
        }
    }

    #[test]
    fn restaurant_filter_keeps_outlets_and_brands() {
        assert!(is_restaurant_relevant(&item(
            "Domino's outlet sealed in Mumbai",
            Some("hygiene violations"),
        )));
        assert!(is_restaurant_relevant(&item("Hotel Sharda dhaba licence suspended", None)));
        assert!(is_restaurant_relevant(&item(
            "Zepto dark store raided",
            Some("expired stock found"),
        )));
        assert!(is_restaurant_relevant(&item("Pizza", Some("Burger King fined in Pune"))));
    }

    #[test]
    fn restaurant_filter_drops_generic_news() {
        assert!(!is_restaurant_relevant(&item(
            "Maharashtra FDA seizes cosmetics racket worth 1 crore",
            None,
        )));
        assert!(!is_restaurant_relevant(&item(
            "Pune records highest unhygienic food complaints",
            Some("FDA held meeting"),
        )));
        assert!(!is_restaurant_relevant(&item("FDA issues advisory on monsoon", None)));
    }
}

/// Pre-filter for the backfill dump: keep only items that reference a food
/// outlet or known restaurant/quick-commerce brand, so generic regulatory news
/// (complaint trends, cosmetics seizures, ...) never reaches the extractor.
pub fn is_restaurant_relevant(item: &NewsItem) -> bool {
    let mut haystack = item.title.to_lowercase();
    if let Some(s) = &item.snippet {
        haystack.push(' ');
        haystack.push_str(&s.to_lowercase());
    }
    RESTAURANT_KEYWORDS.iter().any(|k| haystack.contains(k))
}

const RESTAURANT_KEYWORDS: &[&str] = &[
    "restaurant",
    "hotel",
    "dhaba",
    "eatery",
    "cafe",
    "cafeteria",
    "bhojnalaya",
    "bakery",
    "food court",
    "food outlet",
    "food joint",
    "fast food",
    "cloud kitchen",
    "dark store",
    "quick commerce",
    "canteen",
    "dining",
    "pizza",
    "burger",
    "biryani",
    "kebab",
    "chaat",
    "thali",
    "tandoor",
    "grill",
    "barbeque",
    "bbq",
    "snack",
    "ice cream",
    "sweet shop",
    "vada pav",
    "wada pav",
    "pani puri",
    "golgappa",
    "kfc",
    "mcdonald",
    "domino",
    "pizza hut",
    "burger king",
    "starbucks",
    "zomato",
    "swiggy",
    "blinkit",
    "instamart",
    "zepto",
    "restrow",
    "restraw",
];
