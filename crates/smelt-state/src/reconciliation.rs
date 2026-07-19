//! The reconciliation ledger (`docs/specs/incremental_models.md` §"The
//! reconciliation ledger"): the maintenance plan's bookkeeping of what has
//! already been folded into a model's stored output, keyed
//! `(output-region × column-group)`.
//!
//! **Ledger schema derived from the plan, not declared.** The `group` key
//! is a plain `String` shaped to match
//! `smelt_logical::maintenance::PlanCell::group` — the plan's own
//! `{col1, col2}` / `{*}` display convention for a column group — so a
//! consumer that owns a derived `MaintenancePlan` can key ledger entries
//! directly off `PlanCell::group` and a trigger's source without inventing a
//! second, independent schema. `smelt-state` sits below `smelt-logical` in
//! the crate layering and does not depend on it; the shape is kept
//! structurally compatible by convention, matching how `intervals.rs`
//! already keys off a bare model-name string rather than a typed handle.
//!
//! **Storage is graded by algebra** (spec, same section): an **additive**
//! group's combiner double-counts on a repeat fold, so its entries record
//! **delta identities** — every input delta already folded, per input —
//! and a repeat is refused. An **idempotent** group's combiner is harmless
//! to re-apply, so its entries record only a **frontier** watermark — the
//! furthest point reached per input — and folding is refused only for a
//! delta that does not advance that watermark.
//!
//! **Two operations.** *Fold* consults the entry's processed state and
//! refuses a delta already reflected in it (`docs/specs/incremental_models.md`
//! §"Constraints & Invariants" — "Never fold a delta already reflected in
//! the state"); otherwise it combines and extends. *Recompute-reset*
//! replaces every ledger entry whose region intersects a region recompute
//! with exactly the input state that recompute read — never emptied (the
//! recompute consumed real input), never left stale/accumulated on top of
//! whatever was there before.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// An output region the ledger tracks: a half-open interval `[start, end)`
/// on the model's own output axis. String-encoded like
/// `intervals.rs::Interval` — lexicographic comparison is correct for ISO
/// 8601 date/timestamp strings, the only encoding the ledger assumes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Region {
    pub start: String,
    pub end: String,
}

impl Region {
    pub fn new(start: impl Into<String>, end: impl Into<String>) -> Self {
        Self {
            start: start.into(),
            end: end.into(),
        }
    }

    /// Whether this region and `other` overlap as half-open intervals.
    pub fn intersects(&self, other: &Region) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Storage grading by combiner algebra (spec: "Storage is graded by
/// algebra").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grade {
    /// Re-folding an already-applied delta is harmless; only the furthest
    /// point reached per input is retained.
    Idempotent,
    /// Re-folding double-counts; every delta identity already folded per
    /// input must be retained — never-fold-twice needs exact identity, not
    /// just a high-water mark.
    Additive,
}

/// The processed-input vector `S_{i,g}` of one `(region, group)` ledger
/// entry (spec: "each entry records the processed-input vector `S_{i,g}`"),
/// shaped per the entry's `Grade`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Processed {
    /// `Grade::Idempotent`: input name -> furthest delta identity reached.
    /// A delta advances the frontier iff it sorts strictly after the
    /// current watermark (delta identities must be consistently orderable
    /// for a given input, e.g. ISO date/timestamp strings).
    Frontier(BTreeMap<String, String>),
    /// `Grade::Additive`: input name -> set of delta identities already
    /// folded.
    DeltaIdentities(BTreeMap<String, BTreeSet<String>>),
}

impl Processed {
    fn empty(grade: Grade) -> Self {
        match grade {
            Grade::Idempotent => Processed::Frontier(BTreeMap::new()),
            Grade::Additive => Processed::DeltaIdentities(BTreeMap::new()),
        }
    }

    /// The grade this processed state was constructed under.
    pub fn grade(&self) -> Grade {
        match self {
            Processed::Frontier(_) => Grade::Idempotent,
            Processed::DeltaIdentities(_) => Grade::Additive,
        }
    }
}

/// A fold was refused because `delta` is already reflected in the entry's
/// processed state for `input` (never-fold-twice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldRefused {
    pub input: String,
    pub delta: String,
}

impl std::fmt::Display for FoldRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "delta {:?} from input {:?} is already reflected in the ledger entry",
            self.delta, self.input
        )
    }
}

impl std::error::Error for FoldRefused {}

/// One `(region, group)` ledger entry: its processed-input vector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub processed: Processed,
}

impl LedgerEntry {
    /// A fresh entry instantiated at `S = ∅` under `grade` (spec: "Schema
    /// evolution is a ledger operation: adding a group instantiates its
    /// entries at `S = ∅`").
    pub fn empty(grade: Grade) -> Self {
        Self {
            processed: Processed::empty(grade),
        }
    }

    /// Fold-precondition check + apply: refuse if `delta_id` from `input`
    /// is already reflected in this entry's processed state, otherwise
    /// combine and extend it.
    pub fn fold(&mut self, input: &str, delta_id: &str) -> Result<(), FoldRefused> {
        match &mut self.processed {
            Processed::Frontier(watermarks) => {
                let advances = match watermarks.get(input) {
                    Some(current) => delta_id > current.as_str(),
                    None => true,
                };
                if !advances {
                    return Err(FoldRefused {
                        input: input.to_string(),
                        delta: delta_id.to_string(),
                    });
                }
                watermarks.insert(input.to_string(), delta_id.to_string());
                Ok(())
            }
            Processed::DeltaIdentities(identities) => {
                let set = identities.entry(input.to_string()).or_default();
                if !set.insert(delta_id.to_string()) {
                    return Err(FoldRefused {
                        input: input.to_string(),
                        delta: delta_id.to_string(),
                    });
                }
                Ok(())
            }
        }
    }
}

/// One `(region, group)` record. Stored as a flat, JSON-friendly `Vec`
/// (like `IntervalStore`'s flattened map) rather than a map keyed by a
/// composite struct, since JSON object keys must be strings and `Region` is
/// not one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub region: Region,
    pub group: String,
    pub entry: LedgerEntry,
}

/// The full reconciliation ledger: every `(region, group)` entry a model's
/// maintenance plan has folded into or recomputed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconciliationLedger {
    #[serde(default)]
    pub records: Vec<LedgerRecord>,
}

impl ReconciliationLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the entry for exactly `(region, group)`.
    pub fn get(&self, region: &Region, group: &str) -> Option<&LedgerEntry> {
        self.records
            .iter()
            .find(|r| &r.region == region && r.group == group)
            .map(|r| &r.entry)
    }

    /// Get or create the entry for exactly `(region, group)`, instantiating
    /// it at `S = ∅` under `grade` if absent.
    pub fn get_or_create(
        &mut self,
        region: &Region,
        group: &str,
        grade: Grade,
    ) -> &mut LedgerEntry {
        if let Some(pos) = self
            .records
            .iter()
            .position(|r| &r.region == region && r.group == group)
        {
            &mut self.records[pos].entry
        } else {
            self.records.push(LedgerRecord {
                region: region.clone(),
                group: group.to_string(),
                entry: LedgerEntry::empty(grade),
            });
            let last = self.records.len() - 1;
            &mut self.records[last].entry
        }
    }

    /// Fold `delta_id` from `input` into the `(region, group)` entry,
    /// creating it under `grade` if absent. Refuses per `LedgerEntry::fold`
    /// — never-fold-twice.
    pub fn fold(
        &mut self,
        region: &Region,
        group: &str,
        grade: Grade,
        input: &str,
        delta_id: &str,
    ) -> Result<(), FoldRefused> {
        self.get_or_create(region, group, grade)
            .fold(input, delta_id)
    }

    /// Region recompute-reset (spec: "a region recompute resets every
    /// intersecting entry to exactly the input it read"). Every existing
    /// entry for `group` whose region intersects `region` is discarded —
    /// including entries that only partially overlap — and replaced by
    /// exactly one entry keyed by `region` holding `processed`, the input
    /// state the recompute actually read.
    pub fn recompute_reset(&mut self, region: &Region, group: &str, processed: Processed) {
        self.records
            .retain(|r| !(r.group == group && r.region.intersects(region)));
        self.records.push(LedgerRecord {
            region: region.clone(),
            group: group.to_string(),
            entry: LedgerEntry { processed },
        });
    }
}

/// The full reconciliation ledger store, one `ReconciliationLedger` per
/// model — the persisted shape `FileStore` round-trips to `.smelt/`,
/// mirroring `intervals.rs::IntervalStore`'s per-model flattened map.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconciliationStore {
    #[serde(flatten)]
    pub models: HashMap<String, ReconciliationLedger>,
}

impl ReconciliationStore {
    /// Get or create the ledger for `model_name`.
    pub fn get_or_create(&mut self, model_name: &str) -> &mut ReconciliationLedger {
        self.models.entry(model_name.to_string()).or_default()
    }

    /// Get the ledger for `model_name`, read-only.
    pub fn get(&self, model_name: &str) -> Option<&ReconciliationLedger> {
        self.models.get(model_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_refuses_repeat_delta_identity() {
        let mut ledger = ReconciliationLedger::new();
        let region = Region::new("2026-01-01", "2026-01-10");
        ledger
            .fold(&region, "{revenue}", Grade::Additive, "orders", "d1")
            .unwrap();
        let err = ledger
            .fold(&region, "{revenue}", Grade::Additive, "orders", "d1")
            .unwrap_err();
        assert_eq!(err.delta, "d1");
        assert_eq!(err.input, "orders");
    }

    #[test]
    fn fold_refuses_non_advancing_frontier() {
        let mut ledger = ReconciliationLedger::new();
        let region = Region::new("2026-01-01", "2026-01-10");
        ledger
            .fold(
                &region,
                "{total}",
                Grade::Idempotent,
                "orders",
                "2026-01-05",
            )
            .unwrap();
        let err = ledger
            .fold(
                &region,
                "{total}",
                Grade::Idempotent,
                "orders",
                "2026-01-03",
            )
            .unwrap_err();
        assert_eq!(err.delta, "2026-01-03");
    }

    #[test]
    fn recompute_reset_replaces_intersecting_entries() {
        let mut ledger = ReconciliationLedger::new();
        let region = Region::new("2026-01-01", "2026-01-10");
        ledger
            .fold(&region, "{revenue}", Grade::Additive, "orders", "d1")
            .unwrap();

        let mut snapshot = BTreeMap::new();
        snapshot.insert("orders".to_string(), BTreeSet::from(["d9".to_string()]));
        ledger.recompute_reset(
            &region,
            "{revenue}",
            Processed::DeltaIdentities(snapshot.clone()),
        );

        let entry = ledger.get(&region, "{revenue}").unwrap();
        assert_eq!(entry.processed, Processed::DeltaIdentities(snapshot));
    }
}
