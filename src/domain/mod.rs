//! Validated domain types.

mod types;

pub use types::{Article, Candidate, CoveredCity, EventKey};

#[cfg(test)]
#[path = "domain_test.rs"]
mod tests;
