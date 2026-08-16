use serde_json::{json, Value};
use vercel_runtime::{Error, Request, Response};

use fda_mumbai_tracker::scrape::run_scrape;
use fda_mumbai_tracker::{env, env_or, json_response, serve};

#[tokio::main]
async fn main() -> Result<(), Error> {
    serve(handler).await
}

async fn handler(req: Request) -> Result<Response<Value>, Error> {
    if !authorized(&req) {
        return json_response(401, json!({ "error": "unauthorized" }));
    }
    let gemini_key = env("GEMINI_API_KEY")?;
    let model = env_or("GEMINI_MODEL", "gemini-2.5-flash");

    match run_scrape(&gemini_key, &model).await {
        Ok((run_id, r)) => json_response(
            200,
            json!({
                "ok": true,
                "runId": run_id,
                "articlesSeen": r.articles_seen,
                "articlesNew": r.articles_new,
                "actionsUpserted": r.actions_upserted,
                "llmCalls": r.llm_calls,
            }),
        ),
        Err(e) => {
            eprintln!("scrape failed: {e:#}");
            json_response(500, json!({ "ok": false, "error": format!("{e:#}") }))
        }
    }
}

fn authorized(req: &Request) -> bool {
    if is_vercel_cron(req) {
        return true;
    }
    match std::env::var("CRON_SECRET") {
        Ok(secret) if !secret.is_empty() => req
            .headers()
            .get("x-cron-secret")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s == secret),
        _ => false,
    }
}

fn is_vercel_cron(req: &Request) -> bool {
    req.headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| ua.contains("vercel-cron"))
}
