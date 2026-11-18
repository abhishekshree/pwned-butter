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
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StoreOutcome {
    pub inserted: usize,
    pub skipped: usize,
}
