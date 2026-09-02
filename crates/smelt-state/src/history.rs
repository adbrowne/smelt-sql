use crate::RunManifest;

/// Query run history for a specific model or all models.
pub struct HistoryQuery<'a> {
    manifests: &'a [RunManifest],
}

/// A summary of a model's run within a manifest.
#[derive(Debug, Clone)]
pub struct ModelRunSummary {
    pub run_id: String,
    pub started_at: String,
    pub model_name: String,
    pub strategy: String,
    pub time_range: Option<(String, String)>,
    pub row_count: usize,
    pub duration_ms: u64,
}

impl<'a> HistoryQuery<'a> {
    pub fn new(manifests: &'a [RunManifest]) -> Self {
        Self { manifests }
    }

    /// Get all runs for a specific model, most recent first.
    pub fn for_model(&self, model_name: &str) -> Vec<ModelRunSummary> {
        let mut results: Vec<ModelRunSummary> = self
            .manifests
            .iter()
            .filter_map(|manifest| {
                manifest
                    .models
                    .get(model_name)
                    .map(|record| ModelRunSummary {
                        run_id: manifest.run_id.clone(),
                        started_at: manifest.started_at.to_rfc3339(),
                        model_name: model_name.to_string(),
                        strategy: record.strategy.clone(),
                        time_range: record
                            .time_range
                            .as_ref()
                            .map(|tr| (tr.start.clone(), tr.end.clone())),
                        row_count: record.row_count,
                        duration_ms: record.duration_ms,
                    })
            })
            .collect();

        // Most recent first
        results.sort_by_key(|r| std::cmp::Reverse(r.started_at.clone()));
        results
    }

    /// Get all runs, most recent first.
    pub fn all_runs(&self) -> Vec<&RunManifest> {
        let mut runs: Vec<&RunManifest> = self.manifests.iter().collect();
        runs.sort_by_key(|r| std::cmp::Reverse(&r.started_at));
        runs
    }

    /// Get the last N runs.
    pub fn last_n(&self, n: usize) -> Vec<&RunManifest> {
        let mut runs = self.all_runs();
        runs.truncate(n);
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelRunRecord, TimeRangeRecord};
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_manifest(run_id: &str, model_name: &str) -> RunManifest {
        let mut models = HashMap::new();
        models.insert(
            model_name.to_string(),
            ModelRunRecord {
                strategy: "delete_insert".to_string(),
                time_range: Some(TimeRangeRecord {
                    start: "2026-01-01".to_string(),
                    end: "2026-01-10".to_string(),
                }),
                partitions_updated: vec!["2026-01-01".to_string()],
                row_count: 100,
                duration_ms: 250,
                batch_safety: Some("fully_batch_safe".to_string()),
                outcome: crate::RunOutcomeKind::Success,
                definition_hash: "sha256:abc".to_string(),
                error: None,
                retry_count: 0,
                probes: Vec::new(),
                subsumed: None,
                deferred_cells: Vec::new(),
            },
        );
        RunManifest {
            run_id: run_id.to_string(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            models,
        }
    }

    #[test]
    fn test_query_for_model() {
        let manifests = vec![
            make_manifest("run1", "daily_revenue"),
            make_manifest("run2", "daily_revenue"),
            make_manifest("run3", "other_model"),
        ];

        let query = HistoryQuery::new(&manifests);
        let results = query.for_model("daily_revenue");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_last_n() {
        let manifests = vec![
            make_manifest("run1", "m1"),
            make_manifest("run2", "m2"),
            make_manifest("run3", "m3"),
        ];

        let query = HistoryQuery::new(&manifests);
        let last = query.last_n(2);
        assert_eq!(last.len(), 2);
    }
}
