//! Per-model migration approval recording (`docs/specs/definition_deltas.md`
//! §Surface "`smelt migrate`": "Approval is of a plan hash ... so what was
//! approved is exactly what runs"). `smelt migrate <model>` records the plan
//! hash it just derived and printed; `smelt migrate <model> --apply` compares
//! the freshly re-derived hash against the recorded one and refuses on
//! absence or mismatch (`docs/specs/definition_deltas.md` §Surface "Approve
//! and apply").

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One model's most recently recorded plan-hash approval. Only one approval
/// is ever live per model — recording a new hash replaces the previous one,
/// matching `--apply`'s "the most recently printed plan" semantics
/// (`docs/specs/definition_deltas.md` §Surface "Approve and apply").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationApproval {
    pub plan_hash: String,
    pub recorded_at: DateTime<Utc>,
}

/// The full migration-approval store: model name -> its most recently
/// recorded plan hash.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrationApprovalStore {
    #[serde(flatten)]
    pub approvals: BTreeMap<String, MigrationApproval>,
}

impl MigrationApprovalStore {
    /// Record `plan_hash` as the live approval for `model`, replacing
    /// whatever was previously recorded for it.
    pub fn record(&mut self, model: &str, plan_hash: String, recorded_at: DateTime<Utc>) {
        self.approvals.insert(
            model.to_string(),
            MigrationApproval {
                plan_hash,
                recorded_at,
            },
        );
    }

    /// The live approval recorded for `model`, if any.
    pub fn get(&self, model: &str) -> Option<&MigrationApproval> {
        self.approvals.get(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_a_hash_replaces_the_previous_one_for_that_model() {
        let mut store = MigrationApprovalStore::default();
        let t1 = DateTime::parse_from_rfc3339("2026-08-16T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t2 = DateTime::parse_from_rfc3339("2026-08-16T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        store.record("orders_summary", "sha256:aaaaaaaaaaaa".to_string(), t1);
        store.record("orders_summary", "sha256:bbbbbbbbbbbb".to_string(), t2);

        assert_eq!(store.approvals.len(), 1);
        let approval = store.get("orders_summary").unwrap();
        assert_eq!(approval.plan_hash, "sha256:bbbbbbbbbbbb");
        assert_eq!(approval.recorded_at, t2);
    }

    #[test]
    fn get_on_unrecorded_model_is_none() {
        let store = MigrationApprovalStore::default();
        assert!(store.get("orders_summary").is_none());
    }
}
