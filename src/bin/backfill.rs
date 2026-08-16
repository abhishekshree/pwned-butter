use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use tokio::sync::Semaphore;

use fda_mumbai_tracker::db;
use fda_mumbai_tracker::llm;
use fda_mumbai_tracker::models::{LlmAction, NewsItem};
use fda_mumbai_tracker::news;
use fda_mumbai_tracker::scrape::{self, run_with_window};

const BACKFILL_DAYS: u32 = 30;
const SEEN_DAYS: i64 = 45;
const MAX_ITEMS: usize = 100;
const DUMP_MAX_ITEMS: usize = 500;
const DUMP_DAY_CONCURRENCY: usize = 2;
const DAY_TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_DATA_DIR: &str = "data/backfill";

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("dump") => dump(&args[1..]).await?,
        Some("ingest") => ingest(&args[1..]).await?,
        _ => gemini_backfill(&args).await?,
    }
    Ok(())
}

async fn gemini_backfill(args: &[String]) -> Result<()> {
    let gemini_key = std::env::var("GEMINI_API_KEY")?;
    let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-latest".into());
    let today = Utc::now().date_naive();

    let (from_days_ago, to_days_ago) = parse_days(args.first(), args.get(1))?;

    let pool = db::pool().await?;
    db::mark_stale_runs(pool).await?;

    for d in (to_days_ago..=from_days_ago).rev() {
        let date = today - ChronoDuration::days(i64::from(d));
        let window = format!("after:{date} before:{}", date + ChronoDuration::days(1));
        println!("\n=== day {date} ({window}) ===");
        let fut = run_with_window(&gemini_key, &model, &window, SEEN_DAYS, MAX_ITEMS, true);
        match tokio::time::timeout(DAY_TIMEOUT, fut).await {
            Ok(Ok((run_id, report))) => println!(
                "day {date}: run {run_id} ok: seen={} new={} upserted={} llm_calls={}",
                report.articles_seen,
                report.articles_new,
                report.actions_upserted,
                report.llm_calls
            ),
            Ok(Err(e)) => eprintln!("day {date} failed: {e:#}"),
            Err(_) => eprintln!("day {date} timed out after {}s", DAY_TIMEOUT.as_secs()),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    if args.is_empty() {
        println!("\n=== wide pass when:30d ===");
        let fut = run_with_window(&gemini_key, &model, "when:30d", SEEN_DAYS, MAX_ITEMS, true);
        match tokio::time::timeout(DAY_TIMEOUT, fut).await {
            Ok(Ok((run_id, report))) => println!(
                "wide: run {run_id} ok: seen={} new={} upserted={} llm_calls={}",
                report.articles_seen,
                report.articles_new,
                report.actions_upserted,
                report.llm_calls
            ),
            Ok(Err(e)) => eprintln!("wide pass failed: {e:#}"),
            Err(_) => eprintln!("wide pass timed out after {}s", DAY_TIMEOUT.as_secs()),
        }
    }

    Ok(())
}

fn parse_days(from_arg: Option<&String>, to_arg: Option<&String>) -> Result<(u32, u32)> {
    let from = match from_arg {
        Some(a) => a.parse()?,
        None => BACKFILL_DAYS,
    };
    let to = match to_arg {
        Some(a) => a.parse()?,
        None => 1,
    };
    Ok((from, to))
}

/// Fetch + enrich every day window offline (no LLM, no DB) and write the
/// restaurant-relevant, deduped items to `<dir>/items/<date>.json` for a local
/// LLM to read. Days run concurrently; already-dumped days are skipped.
async fn dump(args: &[String]) -> Result<()> {
    let dir = PathBuf::from(args.first().map(String::as_str).unwrap_or(DEFAULT_DATA_DIR));
    let (from_days_ago, to_days_ago) = parse_days(args.get(1), args.get(2))?;
    let items_dir = dir.join("items");
    fs::create_dir_all(&items_dir)?;
    write_extract_guide(&dir)?;

    let client = fda_mumbai_tracker::http_client();
    let today = Utc::now().date_naive();
    let seen: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>> =
        Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));
    let day_concurrency = std::env::var("FDA_DAY_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DUMP_DAY_CONCURRENCY);
    let sem = Arc::new(Semaphore::new(day_concurrency));
    let mut tasks = Vec::new();

    for d in (to_days_ago..=from_days_ago).rev() {
        let date = today - ChronoDuration::days(i64::from(d));
        let path = items_dir.join(format!("{date}.json"));
        if path.exists() {
            println!("day {date}: already dumped, skipping");
            continue;
        }
        let permit = sem.clone().acquire_owned().await?;
        let seen = Arc::clone(&seen);
        let items_dir = items_dir.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            let (fetched, kept) = dump_day(client, &seen, &items_dir, date).await;
            (date, fetched, kept)
        }));
    }

    for t in tasks {
        match t.await {
            Ok((date, fetched, kept)) => {
                println!("day {date}: fetched={fetched} dumped={kept}");
            }
            Err(e) => eprintln!("dump task failed: {e}"),
        }
    }

    write_extract_guide(&dir)?;
    Ok(())
}

async fn dump_day(
    client: &'static reqwest::Client,
    seen: &Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
    items_dir: &Path,
    date: chrono::NaiveDate,
) -> (usize, usize) {
    let window = format!("after:{date} before:{}", date + ChronoDuration::days(1));
    let items = match news::fetch_items(client, &window).await {
        Ok(items) => items,
        Err(e) => {
            eprintln!("day {date}: fetch failed: {e:#}");
            return (0, 0);
        }
    };
    let fetched = items.len();
    let seen_snapshot = { seen.lock().await.clone() };
    let fresh = news::enrich(client, items, &seen_snapshot, DUMP_MAX_ITEMS).await;
    {
        let mut guard = seen.lock().await;
        guard.extend(fresh.iter().map(|it| it.url.clone()));
    }
    let fresh: Vec<NewsItem> = fresh
        .into_iter()
        .filter(news::is_restaurant_relevant)
        .collect();
    let kept = fresh.len();

    let doc = json!({
        "date": date.to_string(),
        "count": kept,
        "items": fresh.iter().enumerate().map(|(i, it)| json!({
            "index": i,
            "title": it.title,
            "url": it.url,
            "source": it.source,
            "published": it.published,
            "snippet": it.snippet,
        })).collect::<Vec<_>>(),
    });
    let path = items_dir.join(format!("{date}.json"));
    if let Err(e) = fs::write(
        &path,
        serde_json::to_string_pretty(&doc).unwrap_or_default(),
    ) {
        eprintln!("day {date}: write failed: {e}");
    }
    (fetched, kept)
}

/// Turn `<dir>/actions/<date>.json` (output of a local LLM) into DB rows using
/// the matching `<dir>/items/<date>.json` dumps. Pass `--dry-run` to just print
/// the rows that would be written without touching the database.
async fn ingest(args: &[String]) -> Result<()> {
    let mut args = args;
    let dry_run = args.first().is_some_and(|a| a == "--dry-run");
    if dry_run {
        args = &args[1..];
    }
    let dir = PathBuf::from(args.first().map(String::as_str).unwrap_or(DEFAULT_DATA_DIR));
    let items_dir = dir.join("items");
    let actions_dir = dir.join("actions");

    let mut files: Vec<PathBuf> = fs::read_dir(&actions_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();

    if files.is_empty() {
        println!("no actions files found in {}", actions_dir.display());
        return Ok(());
    }

    let pool = db::pool().await?;
    let mut total = 0usize;
    for f in files {
        let date = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let result = if dry_run {
            preview_day(&items_dir, &f, date)
        } else {
            ingest_one(pool, &items_dir, &f, date).await
        };
        match result {
            Ok(n) => {
                total += n;
                println!(
                    "ingest {date}: +{n} rows{}",
                    if dry_run { " (dry run)" } else { "" }
                );
            }
            Err(e) => eprintln!("ingest {date} failed: {e:#}"),
        }
    }
    println!(
        "{} {total} rows total",
        if dry_run { "would write" } else { "ingested" }
    );
    Ok(())
}

fn load_day(
    items_dir: &Path,
    actions_file: &Path,
    date: &str,
) -> Result<(Vec<NewsItem>, Vec<LlmAction>)> {
    let items_raw = fs::read_to_string(items_dir.join(format!("{date}.json")))
        .with_context(|| format!("missing items dump for {date}"))?;
    let doc: serde_json::Value = serde_json::from_str(&items_raw)?;
    let items: Vec<NewsItem> =
        serde_json::from_value(doc.get("items").context("no items array in dump")?.clone())?;

    let actions_raw = fs::read_to_string(actions_file)?;
    let actions: Vec<LlmAction> = serde_json::from_str(&actions_raw)?;
    Ok((items, actions))
}

fn preview_day(items_dir: &Path, actions_file: &Path, date: &str) -> Result<usize> {
    let (items, actions) = load_day(items_dir, actions_file, date)?;
    let rows = scrape::build_rows(&items, &actions, false);
    for r in &rows {
        println!(
            "  {:<32} {:<16} {:<20} {} | {}",
            r.establishment,
            r.city.as_deref().unwrap_or(""),
            r.action_type,
            r.action_date,
            r.source_url
        );
    }
    Ok(rows.len())
}

async fn ingest_one(
    pool: &sqlx::PgPool,
    items_dir: &Path,
    actions_file: &Path,
    date: &str,
) -> Result<usize> {
    let (items, actions) = load_day(items_dir, actions_file, date)?;

    let rows = scrape::build_rows(&items, &actions, false);
    let inserted = db::insert_actions(pool, &rows).await?;
    let skipped = rows.len().saturating_sub(inserted);
    if skipped > 0 {
        println!("  ({skipped} already present, skipped)");
    }
    db::finish_run(
        pool,
        db::begin_run(pool).await?,
        items.len(),
        items.len(),
        inserted,
        0,
        &json!({"window": date, "origin": "local-llm-ingest"}),
    )
    .await?;
    Ok(inserted)
}

fn write_extract_guide(dir: &Path) -> Result<()> {
    let guide = format!(
        "# Local extraction pass (no Gemini)\n\
         \n\
         For each file in `items/<date>.json`, extract concrete FDA enforcement records and\n\
         write them to `actions/<date>.json` as a JSON array. Send items in chunks of ~20 if\n\
         your model struggles with long inputs; keep `index` values as given.\n\
         \n\
         ## Input format\n\
         \n\
         Each items file is `{{\"date\": ..., \"items\": [{{ \"index\": 0, \"title\": ..., \"url\": ...,\n\
         \"source\": ..., \"published\": ..., \"snippet\": ... }}, ...]}}`. `index` is the position the\n\
         model earned, starting at 0.\n\
         \n\
         ## Output format\n\
         \n\
         `actions/<date>.json` must be a plain JSON array of records. Use ONLY records naming a\n\
         food establishment facing a concrete regulatory action; skip the rest.\n\
         \n\
         ```json\n\
         [{{\"establishment\": \"Noor Mohammadi Hotel\", \"city\": \"Mumbai\", \"actionType\":\n\
         \"licence_suspension\", \"actionDate\": \"2026-07-20\", \"violations\": [\"hygiene lapses\"],\n\
         \"sourceIndex\": 3}}]\n\
         ```\n\
         \n\
         `sourceIndex` MUST match the `index` of the article in the items file. `actionType` is one\n\
         of license_suspension, stop_business, improvement_notice, sealing, seizure, inspection,\n\
         reopened. camelCase keys preferred; snake_case also accepted.\n\
         \n\
         ## Extraction rules\n\
         \n\
         {prompt}\n\
         \n\
         ## Verification (strongly recommended)\n\
         \n\
         Before trusting bulk output, spot-check that every `sourceIndex` points at an article that\n\
         names the establishment and the action. Wrong indices are silently dropped on ingest.\n",
        prompt = llm::SYSTEM_PROMPT
    );
    fs::write(dir.join("EXTRACT.md"), guide)?;
    Ok(())
}
