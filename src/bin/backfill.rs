use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};

use fda_mumbai_tracker::scrape::run_with_window;

const BACKFILL_DAYS: u32 = 15;
const SEEN_DAYS: i64 = 45;
const MAX_ITEMS: usize = 100;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let gemini_key = std::env::var("GEMINI_API_KEY")?;
    let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-latest".into());

    let today = Utc::now().date_naive();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (from_days_ago, to_days_ago) = match args.as_slice() {
        [] => (BACKFILL_DAYS, 1),
        [a] => (a.parse()?, 1),
        [a, b] => (a.parse()?, b.parse()?),
        _ => {
            eprintln!("usage: backfill [from_days_ago [to_days_ago]]");
            std::process::exit(2);
        }
    };

    for d in (to_days_ago..=from_days_ago).rev() {
        let date = today - ChronoDuration::days(i64::from(d));
        let window = format!("after:{date} before:{}", date + ChronoDuration::days(1));
        println!("\n=== day {date} ({window}) ===");
        match run_with_window(&gemini_key, &model, &window, SEEN_DAYS, MAX_ITEMS, true).await {
            Ok((run_id, report)) => println!(
                "day {date}: run {run_id} ok: seen={} new={} upserted={} llm_calls={}",
                report.articles_seen,
                report.articles_new,
                report.actions_upserted,
                report.llm_calls
            ),
            Err(e) => eprintln!("day {date} failed: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    if args.is_empty() {
        println!("\n=== wide pass when:15d ===");
        match run_with_window(&gemini_key, &model, "when:15d", SEEN_DAYS, MAX_ITEMS, true).await {
            Ok((run_id, report)) => println!(
                "wide: run {run_id} ok: seen={} new={} upserted={} llm_calls={}",
                report.articles_seen,
                report.articles_new,
                report.actions_upserted,
                report.llm_calls
            ),
            Err(e) => eprintln!("wide pass failed: {e:#}"),
        }
    }

    Ok(())
}
