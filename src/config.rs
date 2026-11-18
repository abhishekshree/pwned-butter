use anyhow::{Context, Result as AnyResult};

pub const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";
pub const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
pub const DEFAULT_OPENROUTER_FALLBACKS: &[&str] = &["inclusionai/ling-3.2-flash:free"];

#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub key: String,
    pub model: String,
    pub fallbacks: Vec<String>,
}

/// Every environment read in the crate happens here, once, at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub gemini_key: String,
    pub gemini_model: String,
    pub openrouter: Option<OpenRouterConfig>,
}

impl Config {
    pub fn from_env() -> AnyResult<Self> {
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
        let gemini_key = std::env::var("GEMINI_API_KEY").context("GEMINI_API_KEY is not set")?;
        let gemini_model =
            std::env::var("GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_GEMINI_MODEL.to_string());
        let openrouter = std::env::var("OPENROUTER_API_KEY")
            .ok()
            .map(|key| OpenRouterConfig {
                key,
                model: std::env::var("OPENROUTER_MODEL")
                    .unwrap_or_else(|_| DEFAULT_OPENROUTER_MODEL.to_string()),
                fallbacks: std::env::var("OPENROUTER_FALLBACKS")
                    .map(|raw| {
                        raw.split(',')
                            .map(str::trim)
                            .filter(|model| !model.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_else(|_| {
                        DEFAULT_OPENROUTER_FALLBACKS
                            .iter()
                            .map(|model| model.to_string())
                            .collect()
                    }),
            });
        Ok(Self {
            database_url,
            gemini_key,
            gemini_model,
            openrouter,
        })
    }
}
