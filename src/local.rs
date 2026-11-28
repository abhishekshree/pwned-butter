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
                let published = record
                    .get("published_at")
                    .and_then(Value::as_str)
                    .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
                    .map(|date| date.with_timezone(&Utc));
                published.is_none_or(|date| date > cutoff)
            })
            .filter_map(|record| {
                record
                    .get("source_url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    // shortcut: no transactionality, seen-window uses published_at; debug backend only, upgrade if used for backfills.
    pub fn upsert_actions(&self, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
        let mut records = Self::read_records(&self.path)?;
        let mut outcome = StoreOutcome::default();
        for row in rows {
            let key = EventKey::from_candidate(row);
            let action_type = row.action_type.to_string();
            let duplicate = records.iter().any(|record| {
                let Some(date) = record
                    .get("action_date")
                    .and_then(Value::as_str)
                    .and_then(|raw| NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok())
                else {
                    return false;
                };
                (date - row.action_date).num_days().abs() <= 10
                    && record.get("action_type").and_then(Value::as_str)
                        == Some(action_type.as_str())
                    && record
                        .get("establishment")
                        .and_then(Value::as_str)
                        .is_some_and(|name| key.matches(&EventKey::from_name(name)))
            });
            if duplicate {
                outcome.skipped += 1;
                continue;
            }
            let record = serde_json::to_value(row).context("serialize candidate")?;
            self.append(&self.path, &record)?;
            records.push(record);
            outcome.inserted += 1;
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

    pub fn finish_run(&self, run_id: i64, report: &RunReport) -> AnyResult<()> {
        let mut record = report.to_details(&json!({}));
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
#[path = "../tests/unit/local.rs"]
mod tests;
