use std::fmt;
use std::path::PathBuf;

use anyhow::{Context, Result as AnyResult, bail};

pub(crate) const DEFAULT_GEMINI_MODEL: &str = "gemini-flash-latest";
pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
pub(crate) const DEFAULT_OPENROUTER_FALLBACKS: &[&str] = &["inclusionai/ling-3.2-flash:free"];

/// Where a completed run writes its rows and provenance.
#[derive(Clone)]
pub enum Persistence {
    Postgres { url: String },
    Local { path: PathBuf },
}

impl fmt::Debug for Persistence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres { .. } => f.write_str("Postgres { url: <redacted> }"),
            Self::Local { path } => f.debug_tuple("Local").field(path).finish(),
        }
    }
}

#[derive(Clone)]
pub struct OpenRouterConfig {
    pub key: String,
    pub model: String,
    pub fallbacks: Vec<String>,
}

impl fmt::Debug for OpenRouterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenRouterConfig")
            .field("key", &"<redacted>")
            .field("model", &self.model)
            .field("fallbacks", &self.fallbacks)
            .finish()
    }
}

/// Every environment read in the crate happens here.
#[derive(Clone)]
pub struct Config {
    pub persistence: Persistence,
    pub gemini_key: String,
    pub gemini_model: String,
    pub openrouter: Option<OpenRouterConfig>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("persistence", &self.persistence)
            .field("gemini_key", &"<redacted>")
            .field("gemini_model", &self.gemini_model)
            .field("openrouter", &self.openrouter)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> AnyResult<Self> {
        let persistence = match std::env::var("LOCAL_WRITER_PATH") {
            Ok(path) => Persistence::Local {
                path: PathBuf::from(non_empty("LOCAL_WRITER_PATH", path)?),
            },
            Err(_) => Persistence::Postgres {
                url: non_empty(
                    "DATABASE_URL",
                    std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?,
                )?,
            },
        };
        let gemini_key = std::env::var("GEMINI_API_KEY").context("GEMINI_API_KEY is not set")?;
        let gemini_model =
            std::env::var("GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_GEMINI_MODEL.to_string());
        let openrouter = match std::env::var("OPENROUTER_API_KEY") {
            Ok(key) => Some(OpenRouterConfig {
                key: non_empty("OPENROUTER_API_KEY", key)?,
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
            gemini_key: non_empty("GEMINI_API_KEY", gemini_key)?,
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
