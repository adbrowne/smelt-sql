use serde::{Deserialize, Serialize};

/// A single snapshot entry: records the expanded logical SQL that built a physical table
/// and (optionally) the last-computed fingerprint hex for diagnostics / caching.
///
/// Reuse correctness never depends on the stored fingerprint — the evaluator recomputes
/// fingerprints fresh from `source_sql` at decision time (`run_state.md` §"Snapshot reuse").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// Workspace-relative address of the model (e.g. `analytics.orders`).
    pub model: String,
    /// The environment name this entry belongs to.
    pub environment: String,
    /// The physical table name backing this model in this environment.
    pub physical_table: String,
    /// The expanded logical SQL that was used to build the physical table.
    pub source_sql: String,
    /// Cached output fingerprint hex (diagnostic only — never the reuse key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint_hex: Option<String>,
}

/// The `(environment, model) → physical table` map with source SQL for each entry.
///
/// Drives fingerprint-keyed reuse lookup for virtual environments
/// (`virtual_environments.md` §"Reuse decision"; `run_state.md` §"Snapshot and environment store").
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotStore {
    #[serde(default)]
    entries: Vec<SnapshotEntry>,
}

impl SnapshotStore {
    /// Insert or overwrite the entry for `(environment, model)`.
    pub fn upsert(&mut self, entry: SnapshotEntry) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.environment == entry.environment && e.model == entry.model)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Look up the entry for an exact `(environment, model)` pair.
    pub fn get(&self, environment: &str, model: &str) -> Option<&SnapshotEntry> {
        self.entries
            .iter()
            .find(|e| e.environment == environment && e.model == model)
    }

    /// Remove the entry for `(environment, model)`. No-op if absent.
    pub fn remove(&mut self, environment: &str, model: &str) {
        self.entries
            .retain(|e| !(e.environment == environment && e.model == model));
    }

    /// Find the best candidate entry for reuse of `model` in target environment `target_env`,
    /// among entries whose stored `fingerprint_hex` equals `current_fingerprint_hex`.
    ///
    /// Candidate-precedence rule (`virtual_environments.md` §"Reuse decision"):
    /// 1. Target environment `target_env` itself (already-correct; no repoint needed).
    /// 2. Base/production environment (`"prod"` or `"production"`).
    /// 3. Any other environment, in lexicographic order by environment name.
    ///
    /// Returns `None` when no fingerprint-equal entry exists.
    ///
    /// Note: entries without a stored `fingerprint_hex` are not matched. The caller is
    /// responsible for recomputing fingerprints from `source_sql` for those entries.
    pub fn find_candidate<'a>(
        &'a self,
        model: &str,
        current_fingerprint_hex: &str,
        target_env: &str,
    ) -> Option<&'a SnapshotEntry> {
        let mut candidates: Vec<&SnapshotEntry> = self
            .entries
            .iter()
            .filter(|e| e.model == model)
            .filter(|e| {
                e.fingerprint_hex
                    .as_deref()
                    .is_some_and(|h| h == current_fingerprint_hex)
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        candidates.sort_by(|a, b| {
            let tier = |e: &SnapshotEntry| {
                let env = e.environment.as_str();
                if env == target_env {
                    0u8
                } else if env == "prod" || env == "production" {
                    1u8
                } else {
                    2u8
                }
            };
            tier(a)
                .cmp(&tier(b))
                .then_with(|| a.environment.cmp(&b.environment))
        });

        candidates.into_iter().next()
    }

    /// Iterate over all entries.
    pub fn iter(&self) -> impl Iterator<Item = &SnapshotEntry> {
        self.entries.iter()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the store has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(env: &str, model: &str, table: &str, fp: &str) -> SnapshotEntry {
        SnapshotEntry {
            model: model.to_string(),
            environment: env.to_string(),
            physical_table: table.to_string(),
            source_sql: format!("SELECT 1 -- {env}/{model}"),
            fingerprint_hex: Some(fp.to_string()),
        }
    }

    // ── find_candidate ────────────────────────────────────────────────────────

    #[test]
    fn find_candidate_single_matching_entry() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("prod", "orders", "orders__prod", "fp_abc"));

        let result = store.find_candidate("orders", "fp_abc", "dev");
        assert!(result.is_some());
        assert_eq!(result.unwrap().environment, "prod");
    }

    #[test]
    fn find_candidate_prefers_target_env() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("prod", "orders", "orders__prod", "fp_abc"));
        store.upsert(entry("dev", "orders", "orders__dev", "fp_abc"));

        let result = store.find_candidate("orders", "fp_abc", "dev");
        assert_eq!(result.unwrap().environment, "dev");
    }

    #[test]
    fn find_candidate_prefers_prod_over_other_envs() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("alpha", "orders", "orders__alpha", "fp_abc"));
        store.upsert(entry("beta", "orders", "orders__beta", "fp_abc"));
        store.upsert(entry("prod", "orders", "orders__prod", "fp_abc"));

        // target "gamma" does not exist — should get prod
        let result = store.find_candidate("orders", "fp_abc", "gamma");
        assert_eq!(result.unwrap().environment, "prod");
    }

    #[test]
    fn find_candidate_lex_tiebreak_when_no_prod() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("beta", "orders", "orders__beta", "fp_abc"));
        store.upsert(entry("alpha", "orders", "orders__alpha", "fp_abc"));

        // target "gamma" doesn't exist, no prod → lex: "alpha" < "beta"
        let result = store.find_candidate("orders", "fp_abc", "gamma");
        assert_eq!(result.unwrap().environment, "alpha");
    }

    #[test]
    fn find_candidate_target_env_already_correct() {
        // Target env itself is fingerprint-equal → return it (already correct, no repoint needed)
        let mut store = SnapshotStore::default();
        store.upsert(entry("dev", "orders", "orders__dev", "fp_abc"));
        store.upsert(entry("prod", "orders", "orders__prod", "fp_abc"));

        let result = store.find_candidate("orders", "fp_abc", "dev");
        assert_eq!(result.unwrap().environment, "dev");
    }

    #[test]
    fn find_candidate_returns_none_on_fingerprint_mismatch() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("prod", "orders", "orders__prod", "fp_old"));

        let result = store.find_candidate("orders", "fp_new", "dev");
        assert!(result.is_none());
    }

    #[test]
    fn find_candidate_returns_none_when_no_fingerprint_stored() {
        // An entry without a stored fingerprint is not matched by find_candidate.
        let mut store = SnapshotStore::default();
        store.upsert(SnapshotEntry {
            model: "orders".to_string(),
            environment: "prod".to_string(),
            physical_table: "orders__prod".to_string(),
            source_sql: "SELECT 1".to_string(),
            fingerprint_hex: None,
        });

        let result = store.find_candidate("orders", "fp_abc", "dev");
        assert!(result.is_none());
    }

    // ── upsert / get / remove ────────────────────────────────────────────────

    #[test]
    fn upsert_and_get_roundtrip() {
        let mut store = SnapshotStore::default();
        let e = entry("dev", "customers", "customers__dev", "fp_x");
        store.upsert(e.clone());

        let got = store.get("dev", "customers").unwrap();
        assert_eq!(got.physical_table, "customers__dev");
        assert_eq!(got.fingerprint_hex.as_deref(), Some("fp_x"));
    }

    #[test]
    fn upsert_overwrites_existing() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("dev", "orders", "orders__dev_v1", "fp_old"));
        store.upsert(entry("dev", "orders", "orders__dev_v2", "fp_new"));

        assert_eq!(store.len(), 1);
        assert_eq!(
            store.get("dev", "orders").unwrap().physical_table,
            "orders__dev_v2"
        );
    }

    #[test]
    fn remove_deletes_entry() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("dev", "customers", "customers__dev", "fp_x"));
        store.remove("dev", "customers");
        assert!(store.get("dev", "customers").is_none());
    }

    #[test]
    fn remove_noop_when_missing() {
        let mut store = SnapshotStore::default();
        store.remove("dev", "nonexistent"); // should not panic
    }

    // ── serialization roundtrip ──────────────────────────────────────────────

    #[test]
    fn json_roundtrip() {
        let mut store = SnapshotStore::default();
        store.upsert(entry("prod", "orders", "orders__prod", "fp_abc"));
        store.upsert(SnapshotEntry {
            model: "customers".to_string(),
            environment: "dev".to_string(),
            physical_table: "customers__dev".to_string(),
            source_sql: "SELECT * FROM raw.customers".to_string(),
            fingerprint_hex: None,
        });

        let json = serde_json::to_string_pretty(&store).unwrap();
        let loaded: SnapshotStore = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 2);
        let e = loaded.get("prod", "orders").unwrap();
        assert_eq!(e.physical_table, "orders__prod");
        assert_eq!(e.fingerprint_hex.as_deref(), Some("fp_abc"));

        let e2 = loaded.get("dev", "customers").unwrap();
        assert!(e2.fingerprint_hex.is_none());
    }

    #[test]
    fn empty_store_roundtrip() {
        let store = SnapshotStore::default();
        let json = serde_json::to_string(&store).unwrap();
        let loaded: SnapshotStore = serde_json::from_str(&json).unwrap();
        assert!(loaded.is_empty());
    }
}
