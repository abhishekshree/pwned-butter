use std::time::Duration;

use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};

use fda_mumbai_tracker::db;
use fda_mumbai_tracker::scrape::run_with_window;

const BACKFILL_DAYS: u32 = 30;
const SEEN_DAYS: i64 = 45;
const MAX_ITEMS: usize = 100;
const DAY_TIMEOUT: Duration = Duration::from_secs(600);

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().skip(1).collect();

    let gemini_key = std::env::var("GEMINI_API_KEY")?;
    let model = std::env::var("GEMINI_MODEL")
        .unwrap_or_else(|_| fda_mumbai_tracker::llm::DEFAULT_GEMINI_MODEL.into());
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
