use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result as AnyResult};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};

use crate::domain::{Candidate, EventKey};
use crate::report::{RunReport, StoreOutcome};

/// A file-backed stand-in for Postgres, so local runs never touch the live DB.
#[derive(Clone)]
pub struct LocalWriter {
    path: PathBuf,
}

impl LocalWriter {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn runs_path(&self) -> PathBuf {
        self.path.with_extension("runs.jsonl")
    }

    fn read_records(path: &Path) -> AnyResult<Vec<Value>> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).with_context(|| format!("open {}", path.display())),
        };
        BufReader::new(file)
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let context = || format!("{} line {}", path.display(), index + 1);
                match line {
                    Ok(line) if line.trim().is_empty() => None,
                    Ok(line) => Some(serde_json::from_str(&line).with_context(context)),
                    Err(error) => Some(Err(error).with_context(context)),
                }
            })
            .collect()
    }

    fn timestamp(record: &Value, field: &str) -> Option<DateTime<Utc>> {
        record
            .get(field)
            .and_then(Value::as_str)
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|date| date.with_timezone(&Utc))
    }

    fn append(&self, path: &Path, record: &Value) -> AnyResult<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open {} for append", path.display()))?;
        serde_json::to_writer(&mut file, record)
            .with_context(|| format!("write {}", path.display()))?;
        writeln!(file).with_context(|| format!("write {}", path.display()))
    }

    pub fn seen_urls(&self, cutoff: DateTime<Utc>) -> AnyResult<HashSet<String>> {
        Ok(Self::read_records(&self.path)?
            .into_iter()
            .filter(|record| {
                // First-stored time, not publication time; old files lack created_at.
                Self::timestamp(record, "created_at")
                    .or_else(|| Self::timestamp(record, "published_at"))
                    .is_none_or(|date| date > cutoff)
            })
            .filter_map(|record| {
                record
                    .get("source_url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    fn near_event(record: &Value, date: NaiveDate, key: &EventKey) -> bool {
        record
            .get("action_date")
            .and_then(Value::as_str)
            .and_then(|raw| NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok())
            .is_some_and(|stored| (stored - date).num_days().abs() <= 10)
            && record
                .get("establishment")
                .and_then(Value::as_str)
                .is_some_and(|name| {
                    key.matches(&EventKey::from_row(
                        name,
                        record.get("city").and_then(Value::as_str),
                        record.get("area").and_then(Value::as_str),
                    ))
                })
    }

    // shortcut: no transactionality, O(N*M) full-file scan per batch, same-source re-stores append a newest row; debug backend only, upgrade if used for backfills.
    pub fn upsert_actions(&self, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
        let mut records = Self::read_records(&self.path)?;
        let mut outcome = StoreOutcome::default();
        for row in rows {
            let key =
                EventKey::from_row(row.establishment(), Some(row.city().as_str()), row.area());
            let action_type = row.action_type().to_string();
            let mut same_source = false;
            let mut other_source = false;
            for record in &records {
                if !Self::near_event(record, row.action_date(), &key) {
                    continue;
                }
                if record.get("source_url").and_then(Value::as_str) == Some(row.source_url()) {
                    same_source = true;
                } else if record.get("action_type").and_then(Value::as_str)
                    == Some(action_type.as_str())
                {
                    other_source = true;
                }
            }
            if !same_source && other_source {
                outcome.skipped += 1;
                continue;
            }
            let mut record = serde_json::to_value(row).context("serialize candidate")?;
            if let Some(obj) = record.as_object_mut() {
                obj.insert("created_at".into(), json!(Utc::now().to_rfc3339()));
            }
            self.append(&self.path, &record)?;
            records.push(record);
            if same_source {
                outcome.updated += 1;
            } else {
                outcome.inserted += 1;
            }
        }
        Ok(outcome)
    }

    pub fn begin_run(&self) -> AnyResult<i64> {
        let runs = Self::read_records(&self.runs_path())?;
        let id = runs
            .iter()
            .filter_map(|record| record.get("id").and_then(Value::as_i64))
            .max()
            .unwrap_or(0)
            + 1;
        self.append(&self.runs_path(), &json!({"id": id, "status": "running"}))
            .map(|()| id)
    }

    pub fn finish_run(&self, run_id: i64, report: &RunReport, details: &Value) -> AnyResult<()> {
        let mut record = report.to_details(details);
        if let Some(obj) = record.as_object_mut() {
            obj.insert("id".into(), json!(run_id));
            obj.insert("status".into(), json!("ok"));
            obj.insert("articles_seen".into(), json!(report.articles_seen));
            obj.insert("articles_new".into(), json!(report.articles_new));
            obj.insert("actions_upserted".into(), json!(report.actions_upserted));
            obj.insert("llm_calls".into(), json!(report.metrics.total_calls()));
        }
        self.append(&self.runs_path(), &record)
    }

    pub fn fail_run(&self, run_id: i64, error: &str) -> AnyResult<()> {
        self.append(
            &self.runs_path(),
            &json!({"id": run_id, "status": "error", "error": error}),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::env::temp_dir;
    use std::fs::remove_file;
    use std::process::id;

    use chrono::Duration;

    use super::*;
    use crate::domain::Article;
    use crate::models::{ActionType, LlmAction, SanitizedAction};

    const OLD_DETAILS: &str =
        "licence suspended over cockroach infestation found during inspection";
    const NEW_DETAILS: &str =
        "licence suspended after repeat inspections found live cockroaches in the kitchen";

    fn candidate_with_details(
        url: &str,
        establishment: &str,
        date: &str,
        details: &str,
    ) -> Candidate {
        let action = LlmAction {
            establishment: establishment.into(),
            city: Some("Mumbai".into()),
            action_type: ActionType::Inspection,
            action_date: Some(date.into()),
            details: Some(details.into()),
            ..Default::default()
        };
        let article = Article {
            title: "headline".into(),
            url: url.into(),
            source: None,
            published: None,
            snippet: None,
        };
        Candidate::try_new(SanitizedAction::new(action), &article).expect("valid candidate")
    }

    fn candidate_on(url: &str, establishment: &str, date: &str) -> Candidate {
        candidate_with_details(url, establishment, date, OLD_DETAILS)
    }

    fn candidate(url: &str, establishment: &str) -> Candidate {
        candidate_on(url, establishment, "2026-08-11")
    }

    #[test]
    fn run_ids_increment_once_per_run() {
        let path = temp_dir().join(format!("fda-local-runs-{}.jsonl", id()));
        let writer = LocalWriter::new(path);
        let _ = remove_file(writer.runs_path());

        let first = writer.begin_run().expect("first run");
        let second = writer.begin_run().expect("second run");

        assert_eq!((first, second), (1, 2));
        let _ = remove_file(writer.runs_path());
    }

    #[test]
    fn same_source_restore_appends_newest_record() {
        let path = temp_dir().join(format!("fda-local-same-source-{}.jsonl", id()));
        let _ = remove_file(&path);
        let writer = LocalWriter::new(path.clone());

        let first = writer
            .upsert_actions(&[candidate("https://example.test/1", "Domino's Vile Parle")])
            .expect("first write");
        let second = writer
            .upsert_actions(&[candidate_with_details(
                "https://example.test/1",
                "Domino's Vile Parle",
                "2026-08-11",
                NEW_DETAILS,
            )])
            .expect("second write");
        let records = LocalWriter::read_records(&path).expect("read back");
        let seen = writer
            .seen_urls(Utc::now() - Duration::days(1))
            .expect("read back");

        assert_eq!((first.inserted, second.updated), (1, 1));
        assert_eq!(records.len(), 2, "append-only: the old row stays");
        assert_eq!(
            records
                .first()
                .and_then(|r| r.get("details"))
                .and_then(Value::as_str),
            Some(OLD_DETAILS)
        );
        assert_eq!(
            records
                .get(1)
                .and_then(|r| r.get("details"))
                .and_then(Value::as_str),
            Some(NEW_DETAILS),
            "newest row carries re-stored details"
        );
        assert!(records.iter().all(|r| r.get("created_at").is_some()));
        assert!(seen.contains("https://example.test/1"));

        let _ = remove_file(&path);
    }

    #[test]
    fn dedups_one_event_across_urls_and_nearby_dates() {
        let path = temp_dir().join(format!("fda-local-cross-{}.jsonl", id()));
        let _ = remove_file(&path);
        let writer = LocalWriter::new(path.clone());

        let first = writer
            .upsert_actions(&[candidate_on(
                "https://example.test/1",
                "National Sports Club of India",
                "2026-08-11",
            )])
            .expect("first write");
        let second = writer
            .upsert_actions(&[candidate_on(
                "https://example.test/2",
                "National Sports Club of India (NSCI)",
                "2026-08-13",
            )])
            .expect("second write");

        assert_eq!(
            (first.inserted, second.skipped),
            (1, 1),
            "a re-report by another outlet is one event, as in the Postgres store"
        );
        let _ = remove_file(&path);
    }

    #[test]
    fn seen_window_prefers_created_at_with_published_fallback() {
        let path = temp_dir().join(format!("fda-local-seen-{}.jsonl", id()));
        let _ = remove_file(&path);
        let writer = LocalWriter::new(path.clone());

        writer
            .append(
                &path,
                &json!({
                    "source_url": "https://example.test/old-stored",
                    "created_at": "2026-01-01T00:00:00Z",
                    "published_at": "2026-10-01T00:00:00Z"
                }),
            )
            .expect("write old-stored");
        writer
            .append(
                &path,
                &json!({
                    "source_url": "https://example.test/new-stored",
                    "created_at": "2026-10-01T00:00:00Z",
                    "published_at": "2026-01-01T00:00:00Z"
                }),
            )
            .expect("write new-stored");
        writer
            .append(
                &path,
                &json!({
                    "source_url": "https://example.test/legacy",
                    "published_at": "2026-10-01T00:00:00Z"
                }),
            )
            .expect("write legacy");

        let cutoff = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .expect("cutoff")
            .with_timezone(&Utc);
        let seen = writer.seen_urls(cutoff).expect("read back");

        assert!(!seen.contains("https://example.test/old-stored"));
        assert!(seen.contains("https://example.test/new-stored"));
        assert!(seen.contains("https://example.test/legacy"));

        let _ = remove_file(&path);
    }

    #[test]
    fn finish_run_retains_details() {
        let path = temp_dir().join(format!("fda-local-finish-{}.jsonl", id()));
        let writer = LocalWriter::new(path);
        let _ = remove_file(writer.runs_path());
        let report = RunReport {
            articles_seen: 3,
            articles_new: 2,
            actions_upserted: 1,
            ..Default::default()
        };

        writer
            .finish_run(7, &report, &json!({"model": "test-model"}))
            .expect("finish run");
        let records = LocalWriter::read_records(&writer.runs_path()).expect("read back");
        let record = records.first().expect("one run record");

        assert_eq!(record.get("id").and_then(Value::as_i64), Some(7));
        assert_eq!(
            record.get("model").and_then(Value::as_str),
            Some("test-model")
        );
        assert_eq!(record.get("articles_seen").and_then(Value::as_u64), Some(3));

        let _ = remove_file(writer.runs_path());
    }
}
