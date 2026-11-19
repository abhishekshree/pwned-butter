use super::SecretString;

pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
pub(crate) const DEFAULT_OPENROUTER_FALLBACKS: &[&str] = &["nvidia/nemotron-3.5-super-vl:free"];

#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub key: SecretString,
    pub model: String,
    pub fallbacks: Vec<String>,
}
