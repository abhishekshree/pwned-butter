use serde_json::{json, Value};
use vercel_runtime::{Error, Request, Response};

use fda_mumbai_tracker::db::{self, ListParams};
use fda_mumbai_tracker::{json_response, serve};

#[tokio::main]
async fn main() -> Result<(), Error> {
    serve(handler).await
}

async fn handler(req: Request) -> Result<Response<Value>, Error> {
    let params: ListParams = serde_urlencoded::from_str(req.uri().query().unwrap_or_default())
        .map_err(|e| format!("bad query params: {e}"))?;
    let pool = db::pool().await.map_err(|e| format!("{e:#}"))?;
    let (count, rows) = db::list_actions(pool, &params)
        .await
        .map_err(|e| format!("{e:#}"))?;
    json_response(
        200,
        json!({
            "count": count,
            "limit": params.limit.unwrap_or(50).clamp(1, 200),
            "offset": params.offset.unwrap_or(0).max(0),
            "actions": rows,
        }),
    )
}
