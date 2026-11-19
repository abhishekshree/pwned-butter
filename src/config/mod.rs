mod openrouter;
mod persistence;
mod secret;

pub use openrouter::OpenRouterConfig;
pub(crate) use openrouter::{DEFAULT_OPENROUTER_FALLBACKS, DEFAULT_OPENROUTER_MODEL};
pub use persistence::Persistence;
pub use secret::SecretString;

use std::path::PathBuf;

use anyhow::{Context, Result as AnyResult, bail};

pub(crate) const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";

/// Every environment read in the crate happens here.
#[derive(Debug, Clone)]
pub struct Config {
    pub persistence: Persistence,
    pub gemini_key: SecretString,
    pub gemini_model: String,
    pub openrouter: Option<OpenRouterConfig>,
}

impl Config {
    pub fn from_env() -> AnyResult<Self> {
        let persistence = match std::env::var("LOCAL_WRITER_PATH") {
            Ok(path) => Persistence::Local {
                path: PathBuf::from(non_empty("LOCAL_WRITER_PATH", path)?),
            },
            Err(_) => Persistence::Postgres {
                url: SecretString::new(non_empty(
                    "DATABASE_URL",
                    std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?,
                )?),
            },
        };
        let gemini_key = std::env::var("GEMINI_API_KEY").context("GEMINI_API_KEY is not set")?;
        let gemini_model =
            std::env::var("GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_GEMINI_MODEL.to_string());
        let openrouter = match std::env::var("OPENROUTER_API_KEY") {
            Ok(key) => Some(OpenRouterConfig {
                key: SecretString::new(non_empty("OPENROUTER_API_KEY", key)?),
                model: non_empty(
                    "OPENROUTER_MODEL",
                    std::env::var("OPENROUTER_MODEL")
                        .unwrap_or_else(|_| DEFAULT_OPENROUTER_MODEL.to_string()),
                )?,
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
            }),
            Err(_) => None,
        };
        Ok(Self {
            persistence,
            gemini_key: SecretString::new(non_empty("GEMINI_API_KEY", gemini_key)?),
            gemini_model: non_empty("GEMINI_MODEL", gemini_model)?,
            openrouter,
        })
    }
}

fn non_empty(name: &str, value: String) -> AnyResult<String> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}
