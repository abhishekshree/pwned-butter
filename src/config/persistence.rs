use std::path::PathBuf;

use super::SecretString;

/// Where a completed run writes its rows and provenance.
#[derive(Debug, Clone)]
pub enum Persistence {
    Postgres { url: SecretString },
    Local { path: PathBuf },
}
