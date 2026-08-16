use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rss::Channel;
use scraper::{Html, Selector};
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
    "site:x.com \"Maharashtra FDA\"",
    "site:twitter.com \"Maharashtra FDA\"",
    "site:x.com \"Maharashtra FDA\" raid OR licence OR suspend",
    "site:x.com FDA Mumbai raid",
    "site:x.com \"Tukaram Mundhe\"",
    "site:x.com Mumbai restaurant FDA hygiene",
    "site:x.com Blinkit OR Zepto OR Instamart FDA",
];

pub const MAX_ITEMS: usize = 50;

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
    let mut items: Vec<NewsItem> = Vec::new();
    for query in QUERIES {
        let url = google_news_url(query, window);
        match fetch_feed(client, &url).await {
            Ok(found) => items.extend(found),
            Err(e) => eprintln!("rss query failed ({query}): {e}"),
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
    let mut out: Vec<NewsItem> = Vec::with_capacity(items.len());
    let mut urls: HashSet<String> = HashSet::new();
    for mut item in items {
        match fetch_article(client, &item.url).await {
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
