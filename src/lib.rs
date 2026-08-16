pub mod db;
pub mod llm;
pub mod models;
pub mod news;
pub mod scrape;

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;
use vercel_runtime::{run, service_fn, Error, Request, Response};

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(news::USER_AGENT)
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build shared reqwest client")
    })
}

pub fn json_response(status: u16, value: Value) -> Result<Response<Value>, Error> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(value)
        .map_err(Into::into)
}

pub fn env(name: &str) -> Result<String, Error> {
    std::env::var(name).map_err(|_| format!("missing env var {name}").into())
}

pub fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

pub async fn serve<F, Fut>(handler: F) -> Result<(), Error>
where
    F: Fn(Request) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<Response<Value>, Error>> + Send + 'static,
{
    run(service_fn(handler)).await
}
