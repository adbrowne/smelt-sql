//! Per-group recompute admission and cell derivation — the repair family's
//! `ColumnMerge`-corner technique (`docs/specs/incremental_models.md`
//! §"The repair family"): a non-invertible aggregate that receives a
//! retraction is recomputed only over the affected key groups a delta
//! names, instead of refusing to a full refresh.
//!
//! Three admission obligations gate it
//! (`docs/specs/incremental_models.md` §"Per-cell admission" obligations 4,
//! 6, 7):
//! - obligation 6 (derivable group key) and obligation 7 (affected-key
//!   discovery) are both discharged by [`crate::analysis::affected_keys::
//!   derive_affected_keys`] — that proof already fails closed when no grain
//!   is derivable (declared `unique_key` else the walk's proven grain,
//!   fan-out-gated exactly as [`super::derive::row_identity_with_context`])
//!   *and* when the resolved grain can't be projected through the delta's
//!   own row shape.
//! - obligation 4 (bounded per-group read footprint) reuses the existing
//!   reach / partition-locality route [`super::derive::project_source_link`]
//!   already projects for a mutation cell's scan clamp — the same derived
//!   number, not a second, independent bound.
//!
//! Fail-closed on each: a missing/unprovable obligation refuses by name,
//! naming the failing obligation — never a silent downgrade to an
//! unconstrained (whole-table) key set. This module is called standalone
//! (unit-proven, not yet wired into [`super::derive::derive_maintenance_plan`]
//! — that wiring, and the runtime lowering that executes an admitted cell,
//! is a later phase's scope).

use std::collections::BTreeSet;

use super::derive::{project_source_link, LocalityInputs, SourceLink};
use super::{
    Corner, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, ScanClamp, SourceFacts,
    Technique, Trigger,
};
use crate::analysis::affected_keys::{
    derive_affected_keys, AffectedKeyContext, AffectedKeys, DeltaShape,
};
use crate::analysis::fingerprint::{fingerprint_projection, Projection};

/// The admitted per-group repair verdict: the group key to recompute over
/// and the bounded per-group read slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedRepair {
    /// The grain columns the repair recomputes by — [`AffectedKeys::Keys`]'s
    /// own `cols`, verbatim.
    pub key: Vec<String>,
    /// The derived bounded read slice ([`super::derive::project_source_link`]'s
    /// clamp) the per-group recompute's own read is restricted to.
    pub slice: ScanClamp,
    /// Always `true`: [`AffectedKeys::Keys`]'s own contract already
    /// promises a *sound over-approximation* of the truly affected keys is
    /// admissible (`model_properties.md` §"Affected-key discovery") — this
    /// field surfaces that fact on the admitted verdict for a consumer
    /// (`smelt explain`) rather than re-deriving it.
    pub over_approximated: bool,
}

/// Why per-group recompute was refused for this source's delta — names the
/// failing obligation, never a silent downgrade to a wider repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairRefusal {
    /// Obligations 6/7: no derivable grain, or the delta shape cannot
    /// resolve to a finite affected-key set
    /// (`Refusal::RepairKeysNotDiscoverable`).
    KeysNotDiscoverable { source: String, why: String },
    /// Obligation 4: no reach/locality route bounds the per-group read
    /// (`Refusal::RepairSliceUnbounded`).
    SliceUnbounded { source: String, why: String },
}

/// Admit (or fail-closed refuse) the per-group recompute technique for
/// `source`'s `delta` against `sql`.
pub fn admit_per_group_recompute(
    sql: &str,
    declared_unique_key: &[String],
    source: &SourceFacts,
    output_partition_col: Option<&str>,
    loc: &LocalityInputs<'_>,
    delta: &DeltaShape,
) -> Result<AdmittedRepair, RepairRefusal> {
    let affected_ctx = AffectedKeyContext {
        unique_key: declared_unique_key.to_vec(),
        join: crate::analysis::join_shape::JoinContext::new(),
    };
    let key = match derive_affected_keys(delta, sql, &affected_ctx) {
        AffectedKeys::Keys { cols } => cols,
        AffectedKeys::NotDiscoverable { reason } => {
            return Err(RepairRefusal::KeysNotDiscoverable {
                source: source.name.clone(),
                why: reason,
            });
        }
    };

    let slice = match project_source_link(output_partition_col, loc, source) {
        SourceLink::Clamp(clamp) => clamp,
        SourceLink::Unclocked => {
            return Err(RepairRefusal::SliceUnbounded {
                source: source.name.clone(),
                why: "unclocked source: no partition column to bound the per-group read"
                    .to_string(),
            });
        }
        SourceLink::Unlinked { why } => {
            return Err(RepairRefusal::SliceUnbounded {
                source: source.name.clone(),
                why,
            });
        }
    };

    Ok(AdmittedRepair {
        key,
        slice,
        over_approximated: true,
    })
}

/// Derive the [`DeltaShape`] a `MutationProfile::MutableSnapshot` source's
/// delta carries: it is a whole-row snapshot diff, so — absent a physical
/// schema at this layer — the delta is taken to carry every column of
/// `facts` the model's own SQL actually reads, resolved via the same
/// walk-backed [`fingerprint_projection`] leaf classifier
/// [`crate::analysis::affected_keys`] already reuses (no new raw-text scan).
/// A fail-closed [`Projection::FullRow`] verdict (the model's
/// reference to `facts` could not be resolved to a concrete column set)
/// yields an empty column set — the delta then carries no columns a repair
/// obligation can find present, which fails the affected-key proof closed
/// rather than guessing a wider set. `keyed` mirrors whether `facts`
/// declares a `unique_key` at all.
pub fn delta_shape_for_source(sql: &str, facts: &SourceFacts) -> DeltaShape {
    let columns = match fingerprint_projection(sql, &facts.name) {
        Projection::Columns(cols) => cols,
        Projection::FullRow { .. } => BTreeSet::new(),
    };
    DeltaShape {
        source: facts.name.clone(),
        columns,
        keyed: !facts.unique_key.is_empty(),
    }
}

/// Build the `ColumnMerge`/`PerGroupRecompute` [`PlanCell`] for an admitted
/// repair verdict. `group` is the display name of the column group this
/// cell maintains (mirrors every other `derive_*` cell-builder's own
/// `group.name()` convention — the caller supplies it rather than this
/// function re-deriving column-group membership, which is not this
/// function's scope). `trigger` is the trigger actually being derived
/// (`derive_new_data`'s key-grain posture leg passes its own `NewData`
/// trigger rather than this function hard-coding `UpstreamMutation`).
pub fn derive_repair_cell(admitted: &AdmittedRepair, trigger: Trigger, group: String) -> PlanCell {
    PlanCell {
        group,
        trigger,
        corner: Corner::ColumnMerge,
        technique: Technique::PerGroupRecompute,
        partition_local: PartitionLocal::Yes,
        scans: vec![admitted.slice.clone()],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::Key(admitted.key.clone()),
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_shape_for_a_mutable_source_carries_its_referenced_columns() {
        let sql = "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
                    GROUP BY customer_id";
        let facts = SourceFacts {
            name: "orders".to_string(),
            mutation: super::super::MutationProfile::MutableSnapshot,
            partition_col: Some("order_date".to_string()),
            unique_key: vec!["order_id".to_string()],
            allow_full_scan: false,
        };
        let delta = delta_shape_for_source(sql, &facts);
        assert_eq!(delta.source, "orders");
        assert!(delta.keyed);
        assert_eq!(
            delta.columns,
            ["customer_id".to_string(), "amount".to_string()]
                .into_iter()
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn delta_shape_fails_closed_to_no_columns_on_full_row_projection() {
        let sql = "SELECT * FROM smelt.sources.orders";
        let facts = SourceFacts {
            name: "orders".to_string(),
            mutation: super::super::MutationProfile::MutableSnapshot,
            partition_col: Some("order_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        };
        let delta = delta_shape_for_source(sql, &facts);
        assert!(!delta.keyed);
        assert!(delta.columns.is_empty());
    }
}
