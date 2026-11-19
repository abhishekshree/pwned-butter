use std::env;

use anyhow::{Context, Result as AnyResult, bail};
use fda_mumbai_tracker::init;
use tracing::info;

// POSTs to the dashboard's /api/revalidate route to clear the cached
// queries after a backfill or fix. Usage:
//   REVALIDATE_URL=https://<domain>/api/revalidate REVALIDATE_SECRET=... cargo run --bin cache_bust [tag...]
// URL can also be passed as the first arg when it ends with /api/revalidate.
#[tokio::main]
async fn main() -> AnyResult<()> {
    init();
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    let mut url = env::var("REVALIDATE_URL").unwrap_or_default();
    if args.first().is_some_and(|a| a.contains("/api/revalidate")) {
        url = args.remove(0);
    }
    if url.is_empty() {
        bail!("set REVALIDATE_URL env or pass the /api/revalidate URL as first arg");
    }
    let secret = env::var("REVALIDATE_SECRET")
        .context("set REVALIDATE_SECRET env (must match Vercel project env)")?;
    let tags = if args.is_empty() {
        vec!["actions".to_string()]
    } else {
        args
    };

    let res = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({ "secret": secret, "tags": tags }))
        .send()
        .await?;
    let status = res.status();
    let body = res.text().await?;
    info!("{status}: {body}");
    if !status.is_success() {
        bail!("revalidate failed: {status}");
    }
    Ok(())
}
