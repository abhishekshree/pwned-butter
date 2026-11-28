use serde_json::{Value, json};

#[derive(Debug, Default, Clone)]
pub struct RunMetrics {
    pub triage_calls: usize,
    pub extraction_calls: usize,
    pub triage_failures: usize,
    pub extraction_failures: usize,
    pub rejected: usize,
}

impl RunMetrics {
    pub fn total_calls(&self) -> usize {
        self.triage_calls + self.extraction_calls
    }
}

#[derive(Debug, Default, Clone)]
pub struct RunReport {
    pub articles_seen: usize,
    pub articles_new: usize,
    pub actions_upserted: usize,
    pub metrics: RunMetrics,
    pub degraded: bool,
}

impl RunReport {
    /// One representation of a run's metrics for the `fetch_runs.details` column.
    pub fn to_details(&self) -> Value {
        json!({
            "triage_calls": self.metrics.triage_calls,
            "extraction_calls": self.metrics.extraction_calls,
            "triage_failures": self.metrics.triage_failures,
            "extraction_failures": self.metrics.extraction_failures,
            "rejected": self.metrics.rejected,
            "degraded": self.degraded,
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StoreOutcome {
    pub inserted: usize,
    pub skipped: usize,
}
