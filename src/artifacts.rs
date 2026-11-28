use std::fs::{self, File};
use std::path::Path;

use anyhow::{Context, Result as AnyResult};
use serde::Serialize;

pub fn write_json<T: Serialize>(dir: &Path, name: &str, value: &T) -> AnyResult<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(name);
    let file =
        File::create(&path).with_context(|| format!("create artifact {}", path.display()))?;
    serde_json::to_writer_pretty(file, value)
        .with_context(|| format!("write artifact {}", path.display()))
}
