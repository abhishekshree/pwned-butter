use std::env::var;
use std::path::PathBuf;

use anyhow::{Context, Result as AnyResult, bail};

use super::{Persistence, SecretString};

pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "nvidia/nemotron-3-ultra-550b-a55b:free";
pub(crate) const DEFAULT_OPENROUTER_FALLBACKS: &[&str] =
    &["nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free"];
/// gpt-oss is a reasoning model: on Groq's free tier it spends its token budget
/// thinking and returns no text, so the non-reasoning model leads.
pub(crate) const DEFAULT_GROQ_MODEL: &str = "qwen/qwen3.8-27b";
pub(crate) const DEFAULT_GROQ_FALLBACKS: &[&str] = &["openai/gpt-oss-120b"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenRouter,
    Groq,
}

/// The active LLM vendor. Only that vendor's key is read from the environment.
#[derive(Debug, Clone)]
pub struct Llm {
    pub provider: Provider,
    pub key: SecretString,
    pub model: String,
    pub fallbacks: Vec<String>,
}

impl Llm {
    /// Groq rejects a top-level array schema, so it gets plain JSON output.
    pub(crate) fn structured_output(&self) -> bool {
        self.provider != Provider::Groq
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
        let provider = match var("LLM_PROVIDER")
            .unwrap_or_else(|_| "groq".to_string())
            .to_lowercase()
            .as_str()
        {
            "openrouter" => Provider::OpenRouter,
            "groq" => Provider::Groq,
            other => bail!("unknown LLM_PROVIDER: {other}"),
        };
        let llm = match provider {
            Provider::OpenRouter => Llm {
                provider,
                key: required_key("OPENROUTER_API_KEY")?,
                model: model_var("OPENROUTER_MODEL", DEFAULT_OPENROUTER_MODEL)?,
                fallbacks: fallbacks("OPENROUTER_FALLBACKS", DEFAULT_OPENROUTER_FALLBACKS),
            },
            Provider::Groq => Llm {
                provider,
                key: required_key("GROQ_API_KEY")?,
                model: model_var("GROQ_MODEL", DEFAULT_GROQ_MODEL)?,
                fallbacks: fallbacks("GROQ_FALLBACKS", DEFAULT_GROQ_FALLBACKS),
            },
        };
        Ok(Self { persistence, llm })
    }
}

fn model_var(name: &str, default: &str) -> AnyResult<String> {
    non_empty(name, var(name).unwrap_or_else(|_| default.to_string()))
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
