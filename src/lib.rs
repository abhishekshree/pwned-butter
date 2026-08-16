pub mod db;
pub mod llm;
pub mod models;
pub mod news;
pub mod scrape;

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;

static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

pub fn http_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(news::USER_AGENT)
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build shared reqwest client")
    })
}
