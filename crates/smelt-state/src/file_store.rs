use crate::intervals::IntervalStore;
use crate::RunManifest;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// JSON file-backed state store.
///
/// Stores data in `.smelt/` within the project directory:
/// - `.smelt/runs/{run_id}.json` — Run manifests
/// - `.smelt/intervals.json` — Interval tracking
pub struct FileStore {
    state_dir: PathBuf,
}

impl FileStore {
    /// Create a new FileStore rooted at `.smelt/` under the given project directory.
    pub fn new(project_dir: &Path) -> Self {
        Self {
            state_dir: project_dir.join(".smelt"),
        }
    }

    /// Ensure the state directories exist.
    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(self.runs_dir())
            .with_context(|| format!("Failed to create runs directory: {:?}", self.runs_dir()))?;
        Ok(())
    }

    fn runs_dir(&self) -> PathBuf {
        self.state_dir.join("runs")
    }

    fn intervals_path(&self) -> PathBuf {
        self.state_dir.join("intervals.json")
    }

    // --- Run Manifests ---

    /// Save a run manifest to disk.
    pub fn save_run(&self, manifest: &RunManifest) -> Result<()> {
        self.init()?;
        let path = self.runs_dir().join(format!("{}.json", manifest.run_id));
        let json = serde_json::to_string_pretty(manifest)
            .with_context(|| "Failed to serialize run manifest")?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write run manifest: {:?}", path))?;
        Ok(())
    }

    /// Load all run manifests, sorted by run_id (newest first).
    pub fn load_runs(&self) -> Result<Vec<RunManifest>> {
        let runs_dir = self.runs_dir();
        if !runs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut manifests = Vec::new();
        for entry in std::fs::read_dir(&runs_dir)
            .with_context(|| format!("Failed to read runs directory: {:?}", runs_dir))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str::<RunManifest>(&content) {
                        Ok(manifest) => manifests.push(manifest),
                        Err(e) => {
                            eprintln!("Warning: Failed to parse run manifest {:?}: {}", path, e);
                        }
                    },
                    Err(e) => {
                        eprintln!("Warning: Failed to read {:?}: {}", path, e);
                    }
                }
            }
        }

        manifests.sort_by(|a, b| b.run_id.cmp(&a.run_id));
        Ok(manifests)
    }

    /// Load a specific run manifest by ID.
    pub fn load_run(&self, run_id: &str) -> Result<Option<RunManifest>> {
        let path = self.runs_dir().join(format!("{}.json", run_id));
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read run manifest: {:?}", path))?;
        let manifest = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse run manifest: {:?}", path))?;
        Ok(Some(manifest))
    }

    // --- Interval Store ---

    /// Load the interval store from disk. Returns default if file doesn't exist.
    pub fn load_intervals(&self) -> Result<IntervalStore> {
        let path = self.intervals_path();
        if !path.exists() {
            return Ok(IntervalStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read intervals: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse intervals: {:?}", path))?;
        Ok(store)
    }

    /// Save the interval store to disk.
    pub fn save_intervals(&self, store: &IntervalStore) -> Result<()> {
        self.init()?;
        let path = self.intervals_path();
        let json = serde_json::to_string_pretty(store)
            .with_context(|| "Failed to serialize interval store")?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write intervals: {:?}", path))?;
        Ok(())
    }

    /// Check if state directory exists (indicates state tracking has been initialized).
    pub fn exists(&self) -> bool {
        self.state_dir.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelRunRecord, TimeRangeRecord};
    use chrono::Utc;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn test_manifest() -> RunManifest {
        let mut models = HashMap::new();
        models.insert(
            "daily_revenue".to_string(),
            ModelRunRecord {
                strategy: "delete_insert".to_string(),
                time_range: Some(TimeRangeRecord {
                    start: "2026-03-20".to_string(),
                    end: "2026-03-22".to_string(),
                }),
                partitions_updated: vec!["2026-03-20".to_string(), "2026-03-21".to_string()],
                row_count: 1542,
                duration_ms: 230,
                batch_safety: Some("fully_batch_safe".to_string()),
            },
        );
        RunManifest {
            run_id: "20260322-143022-abc123".to_string(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            models,
        }
    }

    #[test]
    fn test_save_and_load_run() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let manifest = test_manifest();
        store.save_run(&manifest).unwrap();

        let loaded = store.load_run(&manifest.run_id).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.run_id, manifest.run_id);
        assert!(loaded.models.contains_key("daily_revenue"));
    }

    #[test]
    fn test_load_all_runs() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let mut m1 = test_manifest();
        m1.run_id = "20260322-100000-aaa".to_string();
        let mut m2 = test_manifest();
        m2.run_id = "20260322-110000-bbb".to_string();

        store.save_run(&m1).unwrap();
        store.save_run(&m2).unwrap();

        let runs = store.load_runs().unwrap();
        assert_eq!(runs.len(), 2);
        // Newest first
        assert_eq!(runs[0].run_id, "20260322-110000-bbb");
    }

    #[test]
    fn test_intervals_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let mut intervals = IntervalStore::default();
        let model = intervals.get_or_create("daily_revenue", "sha256:abc");
        model.record_interval("2026-01-01", "2026-03-22");

        store.save_intervals(&intervals).unwrap();

        let loaded = store.load_intervals().unwrap();
        assert!(loaded.get("daily_revenue").is_some());
        assert_eq!(
            loaded.get("daily_revenue").unwrap().covered_intervals.len(),
            1
        );
    }

    #[test]
    fn test_empty_store() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let runs = store.load_runs().unwrap();
        assert!(runs.is_empty());

        let intervals = store.load_intervals().unwrap();
        assert!(intervals.models.is_empty());
    }
}
