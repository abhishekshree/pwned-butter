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
        let writer = self.clone();
        tokio::task::spawn_blocking(move || writer.seen_urls(cutoff)).await?
    }

    async fn upsert_actions(&self, rows: &[Candidate]) -> AnyResult<StoreOutcome> {
        let writer = self.clone();
        let rows = rows.to_vec();
        tokio::task::spawn_blocking(move || writer.upsert_actions(&rows)).await?
    }

    async fn begin_run(&self) -> AnyResult<i64> {
        let writer = self.clone();
        tokio::task::spawn_blocking(move || writer.begin_run()).await?
    }

    async fn finish_run(&self, run_id: i64, report: &RunReport, details: &Value) -> AnyResult<()> {
        let writer = self.clone();
        let report = report.clone();
        let details = details.clone();
        tokio::task::spawn_blocking(move || writer.finish_run(run_id, &report, &details)).await?
    }

    async fn fail_run(&self, run_id: i64, error: &str) -> AnyResult<()> {
        let writer = self.clone();
        let error = error.to_string();
        tokio::task::spawn_blocking(move || writer.fail_run(run_id, &error)).await?
    }
}
