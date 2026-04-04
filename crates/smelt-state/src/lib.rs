pub mod ddl_duckdb;
pub mod file_store;
pub mod history;
pub mod intervals;
pub mod schema_tracking;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single run manifest recording what smelt did during an execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub models: HashMap<String, ModelRunRecord>,
}

/// Record of a single model's execution within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRunRecord {
    pub strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<TimeRangeRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub partitions_updated: Vec<String>,
    pub row_count: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_safety: Option<String>,
}

/// A time range with start (inclusive) and end (exclusive) dates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeRangeRecord {
    pub start: String,
    pub end: String,
}

/// Generate a run ID from the current time and a random suffix.
pub fn generate_run_id() -> String {
    let now = Utc::now();
    let timestamp = now.format("%Y%m%d-%H%M%S");
    let suffix: u32 = rand_suffix();
    format!("{}-{:06x}", timestamp, suffix)
}

fn rand_suffix() -> u32 {
    use std::time::SystemTime;
    // Use sub-microsecond time bits + process id for sufficient uniqueness
    // in non-concurrent use (only one smelt process runs at a time).
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos ^ std::process::id()) & 0xFFFFFF
}
