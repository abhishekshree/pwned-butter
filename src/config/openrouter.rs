use super::SecretString;

pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
pub(crate) const DEFAULT_OPENROUTER_FALLBACKS: &[&str] = &["inclusionai/ling-3.2-flash:free"];

#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub key: SecretString,
    pub model: String,
    pub fallbacks: Vec<String>,
}
