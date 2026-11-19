use std::collections::HashSet;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::db;
use crate::domain::Candidate;
use crate::local::LocalWriter;
use crate::report::{RunReport, StoreOutcome};

#[async_trait]
pub trait Store: Send + Sync {
    async fn seen_urls(&self, cutoff: DateTime<Utc>) -> AnyResult<HashSet<String>>;
    async fn upsert_actions(&self, rows: &[Candidate]) -> AnyResult<StoreOutcome>;
    async fn begin_run(&self) -> AnyResult<i64>;
    async fn finish_run(&self, run_id: i64, report: &RunReport, details: &Value) -> AnyResult<()>;
    async fn fail_run(&self, run_id: i64, error: &str) -> AnyResult<()>;
}

#[async_trait]
impl Store for PgPool {
    async fn seen_urls(&self, cutoff: DateTime<Utc>) -> AnyResult<HashSet<String>> {
        db::seen_urls(self, cutoff).await
    }

    async fn upsert_actions(&self, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
        db::upsert_actions(self, rows).await
    }

    async fn begin_run(&self) -> AnyResult<i64> {
        db::begin_run(self).await
    }

    async fn finish_run(&self, run_id: i64, report: &RunReport, details: &Value) -> AnyResult<()> {
        db::finish_run(self, run_id, report, details).await
    }

    async fn fail_run(&self, run_id: i64, error: &str) -> AnyResult<()> {
        db::fail_run(self, run_id, error).await
    }
}

#[async_trait]
impl Store for LocalWriter {
    async fn seen_urls(&self, cutoff: DateTime<Utc>) -> AnyResult<HashSet<String>> {
        LocalWriter::seen_urls(self, cutoff)
    }

    async fn upsert_actions(&self, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
        LocalWriter::upsert_actions(self, rows)
    }

    async fn begin_run(&self) -> AnyResult<i64> {
        LocalWriter::begin_run(self)
    }

    async fn finish_run(&self, run_id: i64, report: &RunReport, _: &Value) -> AnyResult<()> {
        LocalWriter::finish_run(self, run_id, report)
    }

    async fn fail_run(&self, run_id: i64, error: &str) -> AnyResult<()> {
        LocalWriter::fail_run(self, run_id, error)
    }
}
