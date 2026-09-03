use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

/// Interval tracking for a single model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelIntervals {
    /// SHA-256 hash of the model SQL at last run.
    pub model_hash: String,
    /// Sorted, non-overlapping covered intervals.
    pub covered_intervals: Vec<Interval>,
    /// Per-`contract.cells[]` maintained frontier — keyed by
    /// `smelt_logical::contract::deferral::cell_address`, valued by the
    /// recorded interval end date (`"%Y-%m-%d"`). `#[serde(default)]` so a
    /// ledger JSON written before this field existed deserialises with an
    /// empty map, needing no migration
    /// (`docs/outcomes/20260815-definition-delta-migrate/phases/12-plan.md`).
    /// Absent from this map means "never recorded" — the same "first run,
    /// never skip" posture `run_license`'s `None` frontier already grants at
    /// model granularity.
    #[serde(default)]
    pub cell_frontiers: HashMap<String, String>,
}

/// A contiguous time interval [start, end).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Interval {
    pub start: String,
    pub end: String,
}

/// The full interval store: model name -> intervals.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntervalStore {
    #[serde(flatten)]
    pub models: HashMap<String, ModelIntervals>,
}

/// A gap in coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl ModelIntervals {
    pub fn new(model_hash: String) -> Self {
        Self {
            model_hash,
            covered_intervals: Vec::new(),
            cell_frontiers: HashMap::new(),
        }
    }

    /// Record `end` as `cell`'s maintained frontier, advancing only that
    /// cell — sibling cells and `covered_intervals` are untouched
    /// (`docs/outcomes/20260815-definition-delta-migrate/phases/12-plan.md`
    /// test 4).
    pub fn record_cell_frontier(&mut self, cell: &str, end: &str) {
        self.cell_frontiers
            .insert(cell.to_string(), end.to_string());
    }

    /// `cell`'s recorded maintained frontier, or `None` if never recorded.
    pub fn cell_frontier(&self, cell: &str) -> Option<&str> {
        self.cell_frontiers.get(cell).map(String::as_str)
    }

    /// Record that [start, end) has been covered. Merges with adjacent/overlapping intervals.
    pub fn record_interval(&mut self, start: &str, end: &str) {
        self.covered_intervals.push(Interval {
            start: start.to_string(),
            end: end.to_string(),
        });
        self.merge_intervals();
    }

    /// Find gaps in coverage within [query_start, query_end).
    pub fn find_gaps(&self, query_start: &str, query_end: &str) -> Vec<Gap> {
        find_gaps_in(&self.covered_intervals, query_start, query_end)
    }

    /// Get the earliest covered date, if any.
    pub fn earliest_date(&self) -> Option<NaiveDate> {
        self.covered_intervals
            .first()
            .and_then(|i| NaiveDate::parse_from_str(&i.start, "%Y-%m-%d").ok())
    }

    /// Get the latest covered date, if any.
    pub fn latest_date(&self) -> Option<NaiveDate> {
        self.covered_intervals
            .last()
            .and_then(|i| NaiveDate::parse_from_str(&i.end, "%Y-%m-%d").ok())
    }

    /// Sort and merge overlapping/adjacent intervals.
    fn merge_intervals(&mut self) {
        merge_intervals_in(&mut self.covered_intervals);
    }

    /// Invalidate all intervals (e.g., when model hash changes).
    pub fn invalidate(&mut self) {
        self.covered_intervals.clear();
    }
}

impl IntervalStore {
    /// Get or create intervals for a model, checking model hash.
    /// If the hash has changed, invalidates existing intervals.
    pub fn get_or_create(&mut self, model_name: &str, model_hash: &str) -> &mut ModelIntervals {
        let entry = self
            .models
            .entry(model_name.to_string())
            .or_insert_with(|| ModelIntervals::new(model_hash.to_string()));

        if entry.model_hash != model_hash {
            entry.invalidate();
            entry.model_hash = model_hash.to_string();
        }

        entry
    }

    /// Get intervals for a model (read-only).
    pub fn get(&self, model_name: &str) -> Option<&ModelIntervals> {
        self.models.get(model_name)
    }
}

/// Sort and merge overlapping/adjacent intervals in place.
///
/// Free function so [`crate::landed_deltas`] can reuse the exact same
/// merge semantics `ModelIntervals` uses, without needing a
/// `ModelIntervals`-shaped wrapper (source landing has no `model_hash`
/// invalidation concept).
pub fn merge_intervals_in(intervals: &mut Vec<Interval>) {
    if intervals.len() <= 1 {
        return;
    }

    // Lexicographic sort is correct for ISO 8601 date strings (YYYY-MM-DD).
    intervals.sort_by(|a, b| a.start.cmp(&b.start));

    let mut merged = Vec::with_capacity(intervals.len());
    let mut current = intervals[0].clone();

    for interval in &intervals[1..] {
        // Overlap or adjacent: current.end >= interval.start
        if current.end >= interval.start {
            if interval.end > current.end {
                current.end = interval.end.clone();
            }
        } else {
            merged.push(current);
            current = interval.clone();
        }
    }
    merged.push(current);
    *intervals = merged;
}

/// Find gaps in `intervals`' coverage within `[query_start, query_end)`.
///
/// Free function underlying [`ModelIntervals::find_gaps`], reused by
/// [`crate::landed_deltas`] to compute the append-only **interval diff**: the
/// sub-intervals of a newly-landed range not already reflected in prior
/// coverage (`docs/specs/sources.md` §"World-facts admission consumes":
/// "for an append-only clocked source it is the interval diff of processed
/// partitions").
pub fn find_gaps_in(intervals: &[Interval], query_start: &str, query_end: &str) -> Vec<Gap> {
    let qs = match NaiveDate::parse_from_str(query_start, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let qe = match NaiveDate::parse_from_str(query_end, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    if qs >= qe {
        return vec![];
    }

    let mut gaps = Vec::new();
    let mut cursor = qs;

    for interval in intervals {
        let is = match NaiveDate::parse_from_str(&interval.start, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                warn!(
                    "malformed interval start date '{}', skipping",
                    interval.start
                );
                continue;
            }
        };
        let ie = match NaiveDate::parse_from_str(&interval.end, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                warn!("malformed interval end date '{}', skipping", interval.end);
                continue;
            }
        };

        // Skip intervals entirely before cursor
        if ie <= cursor {
            continue;
        }

        // Gap between cursor and start of this interval
        if is > cursor {
            let gap_end = is.min(qe);
            if cursor < gap_end {
                gaps.push(Gap {
                    start: cursor,
                    end: gap_end,
                });
            }
        }

        cursor = cursor.max(ie);
        if cursor >= qe {
            break;
        }
    }

    // Gap after last interval
    if cursor < qe {
        gaps.push(Gap {
            start: cursor,
            end: qe,
        });
    }

    gaps
}

/// Compute a SHA-256 hash of model SQL content.
pub fn compute_model_hash(sql: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(sql.as_bytes());
    format!("sha256:{:x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_merge_intervals() {
        let mut mi = ModelIntervals::new("hash1".into());
        mi.record_interval("2026-01-01", "2026-01-10");
        mi.record_interval("2026-01-10", "2026-01-20");
        assert_eq!(mi.covered_intervals.len(), 1);
        assert_eq!(mi.covered_intervals[0].start, "2026-01-01");
        assert_eq!(mi.covered_intervals[0].end, "2026-01-20");
    }

    #[test]
    fn cell_frontiers_default_when_absent_from_an_old_ledger() {
        // A ledger JSON written before `cell_frontiers` existed has no such
        // key — `#[serde(default)]` must deserialise it as an empty map,
        // needing no ledger migration.
        let json = r#"{"model_hash":"hash1","covered_intervals":[]}"#;
        let mi: ModelIntervals = serde_json::from_str(json).expect("deserialises");
        assert!(mi.cell_frontiers.is_empty());
        assert_eq!(mi.cell_frontier("any"), None);
    }

    #[test]
    fn recording_a_cell_frontier_advances_only_that_cell() {
        let mut mi = ModelIntervals::new("hash1".into());
        mi.record_interval("2026-01-01", "2026-01-10");
        mi.record_cell_frontier("a@raw.a", "2026-01-05");

        assert_eq!(mi.cell_frontier("a@raw.a"), Some("2026-01-05"));
        assert_eq!(
            mi.cell_frontier("b@raw.b"),
            None,
            "recording one cell's frontier must leave sibling cells untouched"
        );
        // The model-level covered intervals are untouched by a cell-level
        // record.
        assert_eq!(mi.covered_intervals.len(), 1);
        assert_eq!(mi.covered_intervals[0].end, "2026-01-10");

        mi.record_cell_frontier("a@raw.a", "2026-01-08");
        assert_eq!(mi.cell_frontier("a@raw.a"), Some("2026-01-08"));
    }

    #[test]
    fn test_merge_overlapping() {
        let mut mi = ModelIntervals::new("hash1".into());
        mi.record_interval("2026-01-01", "2026-01-15");
        mi.record_interval("2026-01-10", "2026-01-25");
        assert_eq!(mi.covered_intervals.len(), 1);
        assert_eq!(mi.covered_intervals[0].end, "2026-01-25");
    }

    #[test]
    fn test_gap_detection() {
        let mut mi = ModelIntervals::new("hash1".into());
        mi.record_interval("2026-01-01", "2026-01-10");
        mi.record_interval("2026-01-20", "2026-01-31");

        let gaps = mi.find_gaps("2026-01-01", "2026-01-31");
        assert_eq!(gaps.len(), 1);
        assert_eq!(
            gaps[0],
            Gap {
                start: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
                end: NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            }
        );
    }

    #[test]
    fn test_gap_before_first_interval() {
        let mut mi = ModelIntervals::new("hash1".into());
        mi.record_interval("2026-01-10", "2026-01-20");

        let gaps = mi.find_gaps("2026-01-01", "2026-01-20");
        assert_eq!(gaps.len(), 1);
        assert_eq!(
            gaps[0],
            Gap {
                start: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                end: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            }
        );
    }

    #[test]
    fn test_no_gaps_when_fully_covered() {
        let mut mi = ModelIntervals::new("hash1".into());
        mi.record_interval("2026-01-01", "2026-02-01");

        let gaps = mi.find_gaps("2026-01-01", "2026-01-31");
        assert!(gaps.is_empty());
    }

    #[test]
    fn test_model_hash_invalidation() {
        let mut store = IntervalStore::default();
        {
            let intervals = store.get_or_create("model_a", "hash1");
            intervals.record_interval("2026-01-01", "2026-01-31");
        }
        assert_eq!(store.get("model_a").unwrap().covered_intervals.len(), 1);

        // Different hash → invalidation
        let intervals = store.get_or_create("model_a", "hash2");
        assert!(intervals.covered_intervals.is_empty());
        assert_eq!(intervals.model_hash, "hash2");
    }

    #[test]
    fn test_earliest_latest_dates() {
        let mut mi = ModelIntervals::new("hash1".into());
        mi.record_interval("2026-03-01", "2026-03-10");
        mi.record_interval("2026-01-01", "2026-01-15");

        assert_eq!(
            mi.earliest_date(),
            Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())
        );
        assert_eq!(
            mi.latest_date(),
            Some(NaiveDate::from_ymd_opt(2026, 3, 10).unwrap())
        );
    }
}
