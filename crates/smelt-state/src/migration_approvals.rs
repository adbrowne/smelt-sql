//! Per-model migration-plan approval store (`docs/specs/definition_deltas.md`
//! §"`smelt migrate`"): `smelt migrate`'s plan step records the plan hash it
//! just printed, so `--apply` can refuse to execute anything the plan step
//! did not itself derive and show a human — a plan whose freshly re-derived
//! hash does not match the recorded one (the model changed since the plan
//! was seen, or nothing was ever planned) is refused rather than applied
//! (`docs/outcomes/20260815-definition-delta-migrate/phases/03-plan.md`).
//!
//! This store's row type is intentionally independent of
//! `smelt-logical::backbuild::MigrationPlan` — this crate sits below
//! `smelt-logical` in the layering (`CLAUDE.md` §"Architectural invariants"
//! — Layered single-ownership), so it cannot name that type. `smelt-cli`
//! converts between the two.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One model's recorded migration-plan approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationApproval {
    /// The plan hash (`smelt_logical::backbuild::plan_hash`) recorded the
    /// last time this model's plan was derived and printed.
    pub plan_hash: String,
    /// `true` while an `--apply` of this plan has started executing
    /// statements but has not yet finished — set before the first statement
    /// and cleared after the last, so an interrupted apply is detectable on
    /// the next invocation.
    pub in_progress: bool,
}

/// The full migration-approval store: canonical model path -> its most
/// recently recorded plan approval.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrationApprovalStore {
    #[serde(flatten)]
    pub models: HashMap<String, MigrationApproval>,
}

impl MigrationApprovalStore {
    /// Record `model`'s most recent plan hash, replacing any previous entry
    /// wholesale — approval is of the *most recent* plan only.
    pub fn record(&mut self, model: &str, plan_hash: String, in_progress: bool) {
        self.models.insert(
            model.to_string(),
            MigrationApproval {
                plan_hash,
                in_progress,
            },
        );
    }

    /// `model`'s recorded approval, or `None` if nothing has been recorded
    /// yet (no plan has ever been derived and printed for it).
    pub fn get(&self, model: &str) -> Option<&MigrationApproval> {
        self.models.get(model)
    }

    /// Remove `model`'s recorded approval entirely — a successfully applied
    /// migration clears its approval, since the delta it approved no
    /// longer exists once applied.
    pub fn clear(&mut self, model: &str) {
        self.models.remove(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_store::FileStore;
    use tempfile::TempDir;

    #[test]
    fn approval_store_round_trips() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let mut approvals = MigrationApprovalStore::default();
        approvals.record("smelt.net_orders", "sha256:abc".to_string(), false);
        store.save_migration_approvals(&approvals).unwrap();

        let loaded = store.load_migration_approvals().unwrap();
        let recorded = loaded.get("smelt.net_orders").unwrap();
        assert_eq!(recorded.plan_hash, "sha256:abc");
        assert!(!recorded.in_progress);
    }

    #[test]
    fn missing_approval_file_loads_empty() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");
        let loaded = store.load_migration_approvals().unwrap();
        assert!(loaded.models.is_empty());
    }

    #[test]
    fn recording_a_new_hash_replaces_the_previous() {
        let mut approvals = MigrationApprovalStore::default();
        approvals.record("smelt.net_orders", "sha256:old".to_string(), false);
        approvals.record("smelt.net_orders", "sha256:new".to_string(), true);

        let recorded = approvals.get("smelt.net_orders").unwrap();
        assert_eq!(recorded.plan_hash, "sha256:new");
        assert!(recorded.in_progress);
    }
}
