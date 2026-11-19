use std::fmt;
use std::path::PathBuf;

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
