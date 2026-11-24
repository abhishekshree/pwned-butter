mod openrouter;
mod persistence;
mod secret;

pub(crate) use openrouter::{DEFAULT_OPENROUTER_FALLBACKS, DEFAULT_OPENROUTER_MODEL};
pub use persistence::Persistence;
pub use secret::SecretString;

use std::env::var;
use std::path::PathBuf;

use anyhow::{Context, Result as AnyResult, bail};

pub(crate) const DEFAULT_GEMINI_MODEL: &str = "gemini-3.8-flash";
/// gpt-oss is a reasoning model: on Groq's free tier it spends its token budget
/// thinking and returns no text, so the non-reasoning model leads.
pub(crate) const DEFAULT_GROQ_MODEL: &str = "qwen/qwen3.8-27b";
pub(crate) const DEFAULT_GROQ_FALLBACKS: &[&str] = &["openai/gpt-oss-120b"];

/// The active LLM vendor. Only that vendor's key is read from the environment.
#[derive(Debug, Clone)]
pub enum Llm {
    Gemini {
        key: SecretString,
        model: String,
    },
    OpenRouter {
        key: SecretString,
        model: String,
        fallbacks: Vec<String>,
    },
    Groq {
        key: SecretString,
        model: String,
        fallbacks: Vec<String>,
    },
}

impl Llm {
    pub fn model(&self) -> &str {
        match self {
            Self::Gemini { model, .. }
            | Self::OpenRouter { model, .. }
            | Self::Groq { model, .. } => model,
        }
    }

    /// Groq rejects a top-level array schema, so it gets plain JSON output.
    pub(crate) fn structured_output(&self) -> bool {
        !matches!(self, Self::Groq { .. })
    }
}

/// Every environment read in the crate happens here.
#[derive(Debug, Clone)]
pub struct Config {
    pub persistence: Persistence,
    pub llm: Llm,
}

impl Config {
    pub fn from_env() -> AnyResult<Self> {
        let persistence = match var("LOCAL_WRITER_PATH") {
            Ok(path) => Persistence::Local {
                path: PathBuf::from(non_empty("LOCAL_WRITER_PATH", path)?),
            },
            Err(_) => Persistence::Postgres {
                url: SecretString::new(non_empty(
                    "DATABASE_URL",
                    var("DATABASE_URL").context("DATABASE_URL is not set")?,
                )?),
            },
        };
        let provider = var("LLM_PROVIDER")
            .unwrap_or_else(|_| "gemini".to_string())
            .to_lowercase();
        let llm = match provider.as_str() {
            "gemini" => Llm::Gemini {
                key: required_key("GEMINI_API_KEY")?,
                model: non_empty(
                    "GEMINI_MODEL",
                    var("GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_GEMINI_MODEL.to_string()),
                )?,
            },
            "openrouter" => Llm::OpenRouter {
                key: required_key("OPENROUTER_API_KEY")?,
                model: non_empty(
                    "OPENROUTER_MODEL",
                    var("OPENROUTER_MODEL")
                        .unwrap_or_else(|_| DEFAULT_OPENROUTER_MODEL.to_string()),
                )?,
                fallbacks: fallbacks("OPENROUTER_FALLBACKS", DEFAULT_OPENROUTER_FALLBACKS),
            },
            "groq" => Llm::Groq {
                key: required_key("GROQ_API_KEY")?,
                model: non_empty(
                    "GROQ_MODEL",
                    var("GROQ_MODEL").unwrap_or_else(|_| DEFAULT_GROQ_MODEL.to_string()),
                )?,
                fallbacks: fallbacks("GROQ_FALLBACKS", DEFAULT_GROQ_FALLBACKS),
            },
            other => bail!("unknown LLM_PROVIDER: {other}"),
        };
        Ok(Self { persistence, llm })
    }
}

fn required_key(name: &str) -> AnyResult<SecretString> {
    Ok(SecretString::new(non_empty(
        name,
        var(name).with_context(|| format!("{name} is not set"))?,
    )?))
}

fn fallbacks(name: &str, defaults: &[&str]) -> Vec<String> {
    var(name)
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|_| defaults.iter().map(|model| (*model).to_string()).collect())
}

fn non_empty(name: &str, value: String) -> AnyResult<String> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}
