use crate::intervals::IntervalStore;
use crate::landed_deltas::LandedDeltaStore;
use crate::reconciliation::ReconciliationStore;
use crate::schema_tracking::DeployedSchema;
use crate::snapshot_store::SnapshotStore;
use crate::RunManifest;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::warn;

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

    fn reconciliation_path(&self) -> PathBuf {
        self.state_dir.join("reconciliation.json")
    }

    fn landed_deltas_path(&self) -> PathBuf {
        self.state_dir.join("landed_deltas.json")
    }

    fn snapshots_path(&self) -> PathBuf {
        self.state_dir.join("snapshots.json")
    }

    fn schemas_dir(&self) -> PathBuf {
        self.state_dir.join("schemas")
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

    /// Load run manifests, sorted by run_id (newest first).
    ///
    /// If `limit` is `Some(n)`, only the most recent `n` manifests are returned.
    /// Files are sorted by name (descending) before loading, so with a limit
    /// only the newest files are read from disk.
    pub fn load_runs(&self, limit: Option<usize>) -> Result<Vec<RunManifest>> {
        let runs_dir = self.runs_dir();
        if !runs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut paths: Vec<_> = std::fs::read_dir(&runs_dir)
            .with_context(|| format!("Failed to read runs directory: {:?}", runs_dir))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();

        // Sort descending by filename (run_id encodes timestamp, so newest first)
        paths.sort_by(|a, b| b.cmp(a));

        if let Some(n) = limit {
            paths.truncate(n);
        }

        let mut manifests = Vec::new();
        for path in &paths {
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str::<RunManifest>(&content) {
                    Ok(manifest) => manifests.push(manifest),
                    Err(e) => {
                        warn!("failed to parse run manifest {:?}: {}", path, e);
                    }
                },
                Err(e) => {
                    warn!("failed to read {:?}: {}", path, e);
                }
            }
        }

        // Already sorted by filename, but re-sort by run_id for correctness
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

    // --- Reconciliation Ledger ---

    /// Load the reconciliation ledger store from disk (one ledger per
    /// model). Returns default if the file doesn't exist — a model with no
    /// ledger has never had a plan-managed fold/recompute recorded.
    pub fn load_reconciliation_store(&self) -> Result<ReconciliationStore> {
        let path = self.reconciliation_path();
        if !path.exists() {
            return Ok(ReconciliationStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read reconciliation ledger: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse reconciliation ledger: {:?}", path))?;
        Ok(store)
    }

    /// Save the reconciliation ledger store to disk.
    pub fn save_reconciliation_store(&self, store: &ReconciliationStore) -> Result<()> {
        self.init()?;
        let path = self.reconciliation_path();
        let json = serde_json::to_string_pretty(store)
            .with_context(|| "Failed to serialize reconciliation ledger")?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write reconciliation ledger: {:?}", path))?;
        Ok(())
    }

    // --- Landed-delta store ---

    /// Load the per-source landed-delta store from disk (`docs/specs/sources.md`
    /// §"World-facts admission consumes"). Returns default if the file
    /// doesn't exist — a source with no entry has never had a landing
    /// recorded.
    pub fn load_landed_deltas(&self) -> Result<LandedDeltaStore> {
        let path = self.landed_deltas_path();
        if !path.exists() {
            return Ok(LandedDeltaStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read landed-delta store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse landed-delta store: {:?}", path))?;
        Ok(store)
    }

    /// Save the per-source landed-delta store to disk.
    pub fn save_landed_deltas(&self, store: &LandedDeltaStore) -> Result<()> {
        self.init()?;
        let path = self.landed_deltas_path();
        let json = serde_json::to_string_pretty(store)
            .with_context(|| "Failed to serialize landed-delta store")?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write landed-delta store: {:?}", path))?;
        Ok(())
    }

    // --- Snapshot / Environment Store ---

    /// Load the snapshot store from disk. Returns an empty store if the file doesn't exist.
    pub fn load_snapshot_store(&self) -> Result<SnapshotStore> {
        let path = self.snapshots_path();
        if !path.exists() {
            return Ok(SnapshotStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read snapshot store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse snapshot store: {:?}", path))?;
        Ok(store)
    }

    /// Save the snapshot store to disk.
    pub fn save_snapshot_store(&self, store: &SnapshotStore) -> Result<()> {
        self.init()?;
        let path = self.snapshots_path();
        let json = serde_json::to_string_pretty(store)
            .with_context(|| "Failed to serialize snapshot store")?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write snapshot store: {:?}", path))?;
        Ok(())
    }

    // --- Schema Tracking ---

    /// Save a deployed schema for a model.
    pub fn save_schema(&self, schema: &DeployedSchema) -> Result<()> {
        let dir = self.schemas_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create schemas directory: {:?}", dir))?;
        let path = dir.join(format!("{}.json", schema.model));
        let json = serde_json::to_string_pretty(schema)
            .with_context(|| "Failed to serialize deployed schema")?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write schema: {:?}", path))?;
        Ok(())
    }

    /// Load the deployed schema for a model. Returns None if not found.
    pub fn load_schema(&self, model_name: &str) -> Result<Option<DeployedSchema>> {
        let path = self.schemas_dir().join(format!("{}.json", model_name));
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read schema: {:?}", path))?;
        let schema = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse schema: {:?}", path))?;
        Ok(Some(schema))
    }

    /// List all model names that have deployed schemas.
    ///
    /// Returns the file stems from `.smelt/schemas/*.json`.
    /// Returns an empty vec if the schemas directory doesn't exist.
    pub fn list_deployed_model_names(&self) -> Vec<String> {
        let dir = self.schemas_dir();
        if !dir.exists() {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Delete the deployed schema for a model.
    ///
    /// No-ops silently if the file does not exist. Called by the build lifecycle
    /// after a successful run to remove orphan entries for deleted model files.
    pub fn delete_schema(&self, model_name: &str) -> Result<()> {
        let path = self.schemas_dir().join(format!("{}.json", model_name));
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to delete schema: {:?}", path))?;
        }
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
    use crate::schema_tracking::DeployedColumn;
    use crate::snapshot_store::SnapshotEntry;
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

        let runs = store.load_runs(None).unwrap();
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
    fn test_landed_deltas_roundtrip() {
        use crate::landed_deltas::LandedDeltaStore;

        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let mut deltas = LandedDeltaStore::default();
        let delta = deltas
            .get_or_create("sources.orders")
            .record_landing("2026-01-01", "2026-01-10");
        assert!(!delta.is_empty());

        store.save_landed_deltas(&deltas).unwrap();

        let loaded = store.load_landed_deltas().unwrap();
        assert!(loaded.get("sources.orders").is_some());
        assert_eq!(
            loaded
                .get("sources.orders")
                .unwrap()
                .covered_intervals
                .len(),
            1
        );
    }

    #[test]
    fn test_landed_deltas_empty_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());
        let loaded = store.load_landed_deltas().unwrap();
        assert!(loaded.sources.is_empty());
    }

    #[test]
    fn test_empty_store() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let runs = store.load_runs(None).unwrap();
        assert!(runs.is_empty());

        let intervals = store.load_intervals().unwrap();
        assert!(intervals.models.is_empty());
    }

    #[test]
    fn test_schema_save_and_load() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let schema = DeployedSchema {
            model: "daily_revenue".to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "sha256:abc".to_string(),
            columns: vec![
                DeployedColumn {
                    name: "order_date".to_string(),
                    data_type: "DATE".to_string(),
                    nullable: false,
                },
                DeployedColumn {
                    name: "total".to_string(),
                    data_type: "DECIMAL(10,2)".to_string(),
                    nullable: true,
                },
            ],
        };

        store.save_schema(&schema).unwrap();

        let loaded = store.load_schema("daily_revenue").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.model, "daily_revenue");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.columns.len(), 2);
    }

    #[test]
    fn test_schema_not_found() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let loaded = store.load_schema("nonexistent").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_delete_schema_removes_file() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        let schema = DeployedSchema {
            model: "stg_orders".to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "sha256:abc".to_string(),
            columns: vec![],
        };
        store.save_schema(&schema).unwrap();
        assert!(store.load_schema("stg_orders").unwrap().is_some());

        store.delete_schema("stg_orders").unwrap();
        assert!(store.load_schema("stg_orders").unwrap().is_none());

        // list_deployed_model_names should no longer include it
        let names = store.list_deployed_model_names();
        assert!(!names.contains(&"stg_orders".to_string()));
    }

    #[test]
    fn test_delete_schema_noop_when_missing() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path());

        // Deleting a non-existent schema should not error
        store.delete_schema("nonexistent").unwrap();
    }

    #[test]
    fn test_snapshot_store_roundtrip() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path());

        let mut snap = SnapshotStore::default();
        snap.upsert(SnapshotEntry {
            model: "orders".to_string(),
            environment: "prod".to_string(),
            physical_table: "orders__prod".to_string(),
            source_sql: "SELECT * FROM raw.orders".to_string(),
            fingerprint_hex: Some("fp_abc123".to_string()),
        });
        snap.upsert(SnapshotEntry {
            model: "customers".to_string(),
            environment: "dev".to_string(),
            physical_table: "customers__dev".to_string(),
            source_sql: "SELECT * FROM raw.customers".to_string(),
            fingerprint_hex: None,
        });

        file_store.save_snapshot_store(&snap).unwrap();

        let loaded = file_store.load_snapshot_store().unwrap();
        assert_eq!(loaded.len(), 2);

        let e = loaded.get("prod", "orders").unwrap();
        assert_eq!(e.physical_table, "orders__prod");
        assert_eq!(e.fingerprint_hex.as_deref(), Some("fp_abc123"));

        let e2 = loaded.get("dev", "customers").unwrap();
        assert!(e2.fingerprint_hex.is_none());
    }

    #[test]
    fn test_snapshot_store_empty_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path());

        let loaded = file_store.load_snapshot_store().unwrap();
        assert!(loaded.is_empty());
    }
}
