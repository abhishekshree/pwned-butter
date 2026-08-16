use serde_json::Value;
use vercel_runtime::{Error, Request, Response};

use fda_mumbai_tracker::db;
use fda_mumbai_tracker::{json_response, serve};

#[tokio::main]
async fn main() -> Result<(), Error> {
    serve(handler).await
}

async fn handler(_req: Request) -> Result<Response<Value>, Error> {
    let pool = db::pool().await.map_err(|e| format!("{e:#}"))?;
    let body = db::stats(pool).await.map_err(|e| format!("{e:#}"))?;
    json_response(200, body)
}
