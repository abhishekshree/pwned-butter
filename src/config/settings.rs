use std::env::var;

use anyhow::{Context, Result as AnyResult, bail};

use super::SecretString;

/// gpt-oss is a reasoning model: on Groq's free tier it spends its token budget
/// thinking and returns no text, so the non-reasoning model leads.
pub(crate) const DEFAULT_GROQ_MODEL: &str = "qwen/qwen3.8-27b";
pub(crate) const DEFAULT_GROQ_FALLBACKS: &[&str] = &["openai/gpt-oss-120b"];

/// Groq credentials and model selection.
#[derive(Debug, Clone)]
pub struct Llm {
    pub key: SecretString,
    pub model: String,
    pub fallbacks: Vec<String>,
}

/// Every environment read in the crate happens here.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: SecretString,
    pub llm: Llm,
}

impl Config {
    pub fn from_env() -> AnyResult<Self> {
        let database_url = SecretString::new(non_empty(
            "DATABASE_URL",
            var("DATABASE_URL").context("DATABASE_URL is not set")?,
        )?);
        let llm = Llm {
            key: required_key("GROQ_API_KEY")?,
            model: model_var("GROQ_MODEL", DEFAULT_GROQ_MODEL)?,
            fallbacks: fallbacks("GROQ_FALLBACKS", DEFAULT_GROQ_FALLBACKS),
        };
        Ok(Self { database_url, llm })
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
