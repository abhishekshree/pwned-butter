use std::fmt;

pub(crate) const DEFAULT_OPENROUTER_MODEL: &str = "openrouter/free";
pub(crate) const DEFAULT_OPENROUTER_FALLBACKS: &[&str] = &["inclusionai/ling-3.2-flash:free"];

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
