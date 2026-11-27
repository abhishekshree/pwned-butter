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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn writes_named_json_file() {
        let dir =
            std::env::temp_dir().join(format!("pwned-butter-artifacts-{}", std::process::id()));
        let value = json!({"ok": true});
        write_json(&dir, "sample.json", &value).expect("write artifact");
        let read: serde_json::Value =
            serde_json::from_reader(File::open(dir.join("sample.json")).expect("open artifact"))
                .expect("parse artifact");
        assert_eq!(read, value);
        fs::remove_dir_all(dir).ok();
    }
}
