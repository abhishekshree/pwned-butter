use serde_json::{json, Value};
use vercel_runtime::{Error, Request, Response};

use fda_mumbai_tracker::db;
use fda_mumbai_tracker::{json_response, serve};

#[tokio::main]
async fn main() -> Result<(), Error> {
    serve(handler).await
}

async fn handler(_req: Request) -> Result<Response<Value>, Error> {
    let pool = db::pool().await.map_err(|e| format!("{e:#}"))?;
    let counts = db::dimension_counts(pool, "brand")
        .await
        .map_err(|e| format!("{e:#}"))?;
    json_response(
        200,
        json!({
            "rows": counts
                .into_iter()
                .map(|(k, n)| json!({ "name": k, "count": n }))
                .collect::<Vec<_>>()
        }),
    )
}
