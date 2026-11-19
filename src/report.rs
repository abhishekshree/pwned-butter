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

/// Call/failure counters returned as data so a degraded run cannot be persisted unnoticed.
#[derive(Debug, Default, Clone, Copy)]
pub struct Usage {
    pub triage_calls: usize,
    pub extraction_calls: usize,
    pub triage_failures: usize,
    pub extraction_failures: usize,
    pub rejected: usize,
}

impl From<Usage> for RunMetrics {
    fn from(usage: Usage) -> Self {
        Self {
            triage_calls: usage.triage_calls,
            extraction_calls: usage.extraction_calls,
            triage_failures: usage.triage_failures,
            extraction_failures: usage.extraction_failures,
            rejected: usage.rejected,
        }
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

#[derive(Debug, Default, Clone, Copy)]
pub struct StoreOutcome {
    pub inserted: usize,
    pub skipped: usize,
}
