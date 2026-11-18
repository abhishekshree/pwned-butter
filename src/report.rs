/// Counters that used to be returned as unlabelled tuple elements.
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

/// One typed result for a scrape run, replacing `ScrapeReport` and its
/// `(run_id, report)` / `usize` companions.
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
