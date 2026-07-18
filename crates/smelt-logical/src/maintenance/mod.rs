//! Maintenance plan — v0 tracer bullet.
//!
//! A model's incremental maintenance as a plan indexed by
//! `(output-column-group × trigger)`, each cell landing in a corner of the
//! read-scope × write-scope 2×2. This is the datatype proposed by
//! `docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` (§3,
//! §5) placed per `08-code-placement.md` §2.1, built here as a **tracer
//! bullet**: enough machinery to derive plans and emit maintenance SQL for
//! the catalogue's key examples (EX-02, EX-07, EX-13, EX-24, EX-36–40 of
//! `07-example-catalogue.md`) and prove equivalence against a full refresh.
//!
//! Honest v0 boundaries (see `09-spec-readiness.md` §2):
//! - Column groups (`ColumnGroup`) and skeleton columns
//!   (`OutputSpec::skeleton_columns`) may now be **derived** —
//!   [`grouping::derive_column_groups`] and [`skeleton::skeleton_columns`] —
//!   or still hand-supplied by a caller that needs a shape outside their v0
//!   scope (a CTE/set-operation-composed model); `derive_maintenance_plan`
//!   itself is agnostic to which and keeps taking `ColumnGroup`/
//!   `skeleton_columns` as plain data.
//! - Scan bounds are derived (`analysis::source_bounds`), combiner algebra is
//!   derived (`analysis::discriminants`), additive-only column adds are
//!   proven (`analysis::model_diff`) — the derivations that exist are
//!   consumed, the ones that don't are inputs.
//!
//! Nothing here is wired into diagnostics, planning, or execution; the module
//! is pure data + pure functions (Salsa-purity compatible by construction).

pub mod choice;
pub mod derive;
pub mod emit;
pub mod granularity;
pub mod grouping;
pub mod locality;
pub mod propagate;
pub mod skeleton;

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::walk::{ColumnComparability, Comparability};

/// How a source's rows change after they first appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationProfile {
    /// Rows are only ever added; an existing row never changes or disappears.
    AppendOnly,
    /// The table is a mutable snapshot: rows may be updated or deleted in
    /// place with no change history.
    MutableSnapshot,
}

/// The facts about one input source that admission consults.
#[derive(Debug, Clone)]
pub struct SourceFacts {
    /// Name as it appears in the model SQL's `smelt.sources.<name>` ref and
    /// in `ColumnGroup::mutation_sensitivity`.
    pub name: String,
    pub mutation: MutationProfile,
    /// The source's partition column, when it is clocked (a timeseries).
    /// `None` = an unclocked lookup — reads of it cannot be partition-pruned.
    pub partition_col: Option<String>,
    /// Key columns for targeted (keyed) writes driven by this source's
    /// changes. Empty when the source is not keyed.
    pub unique_key: Vec<String>,
    /// K8's named per-source escape (`04-knobs.md`): the operator accepts
    /// that maintenance driven by this source is a full-table operation.
    /// Without it, a non-partition-local op refuses (the ratified default).
    pub allow_full_scan: bool,
}

/// A named group of output columns sharing mutation-sensitivity
/// (`01-framework.md` §5), derived by [`grouping::derive_column_groups`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnGroup {
    pub columns: Vec<String>,
    /// Sources whose *post-creation* deltas can change these columns'
    /// values. Empty = never mutated after creation (pass-through columns,
    /// or a pure function of stored columns).
    pub mutation_sensitivity: BTreeSet<String>,
}

impl ColumnGroup {
    /// Display name of the group, e.g. `{converted}`.
    pub fn name(&self) -> String {
        format!("{{{}}}", self.columns.join(", "))
    }
}

/// Output grain: what a stored row is and how it is addressed
/// (`01-framework.md` §10 — declared-and-checked, never derived).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grain {
    /// One region per partition value; regions are rewritten wholesale or
    /// patched column-scoped.
    Partition { partition_col: String },
    /// One row per key; rows are addressed individually (keyed end-state).
    Key { unique_key: Vec<String> },
}

/// The declared output surface the plan maintains.
#[derive(Debug, Clone)]
pub struct OutputSpec {
    /// Physical table name maintenance SQL addresses.
    pub table: String,
    pub grain: Grain,
    /// Columns in membership / grouping / dedup / ordering / identity
    /// positions (`01-framework.md` §6). v0: supplied, not extracted.
    pub skeleton_columns: BTreeSet<String>,
}

/// What changed — the plan's trigger axis (`01-framework.md` §5: creation,
/// mutation, and the definition-change trigger; plus explicit backfill).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// New rows arrived in the driving source (creation).
    NewData { source: String },
    /// An existing row of `source` changed post-creation (mutation).
    UpstreamMutation { source: String },
    /// The model definition gained output fields (definition change).
    ColumnAdded { columns: Vec<String> },
    /// Explicit region recompute from replayable input.
    Backfill,
}

/// One corner of the read-scope × write-scope 2×2 (`01-framework.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    /// delta+state read, targeted write (fold-a-delta).
    FoldDelta,
    /// delta+state read, region-overwrite write (read-modify-write region).
    RmwRegion,
    /// full-input read, targeted write (column-scoped re-derivation).
    ColumnMerge,
    /// full-input read, region-overwrite write (recompute-a-region).
    RecomputeRegion,
}

/// The physical op a cell emits (the technique realizing its corner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Technique {
    /// Region overwrite: `DELETE` the write window, `INSERT` its recompute.
    DeleteInsert,
    /// Keyed fold into stored state: `MERGE` combining the delta into the
    /// stored value (`SET c = c + Δ` for additive combiners).
    KeyedFold,
    /// Column-scoped keyed `MERGE`: re-derive only `columns`, leave skeleton
    /// and siblings in place.
    ColumnScopedMerge,
    /// In-place `UPDATE` from already-stored columns; no upstream read.
    InPlaceUpdate,
}

/// Whether a cell's maintenance is partition-local in each source it reads
/// (`01-framework.md` §5 "Partition-local maintenance").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionLocal {
    /// Scan and footprint both project onto a bounded partition interval.
    Yes,
    /// The footprint spans unbounded partitions; the named source is why.
    No { source: String, why: String },
}

/// The derived scan window on one read source, anchored to the output
/// region: maintaining output partitions `[start, end)` reads this source's
/// partition column over `[start − before, end + after)`. This is the
/// `(partition_col, before, after)` triple of `01-framework.md` §5, carried
/// per cell so the emitted SQL's clamp is the *derived* number, never a
/// hand-typed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanClamp {
    pub source: String,
    /// The source's partition column the clamp predicates on.
    pub column: String,
    pub before: crate::analysis::source_bounds::Seconds,
    pub after: crate::analysis::source_bounds::Seconds,
}

impl ScanClamp {
    /// The reflected footprint (`01-framework.md` §5): a delta of this
    /// source at time `t` writes output over `[t − after, t + before]`.
    /// Scan `(before, after)` and footprint are reflections of each other.
    pub fn footprint(
        &self,
    ) -> (
        crate::analysis::source_bounds::Seconds,
        crate::analysis::source_bounds::Seconds,
    ) {
        (self.after, self.before)
    }
}

/// One `(column-group × trigger)` cell of the plan.
#[derive(Debug, Clone)]
pub struct PlanCell {
    /// Display name of the column group this cell maintains (`{a, b}`), or
    /// `{*}` for whole-row triggers (creation / backfill).
    pub group: String,
    pub trigger: Trigger,
    pub corner: Corner,
    pub technique: Technique,
    pub partition_local: PartitionLocal,
    /// Derived scan windows per read source (empty for reads the derivation
    /// could not bound — those surface in `partition_local` instead).
    pub scans: Vec<ScanClamp>,
    /// True for a definition-change cell: the group's ledger entries start
    /// at `S = ∅` over existing regions and this op catches them up
    /// (`01-framework.md` §8; EX-40's group-convergence rule).
    pub ledger_catch_up: bool,
}

/// A fail-loud refusal: the trigger has no admissible technique, or admitting
/// one would be dishonest (`01-framework.md` §10; `06-proof-obligations.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A field was added in a skeleton position — a grain change, not a
    /// column backfill (EX-39).
    SkeletonColumnAdded { column: String },
    /// The derived scan/footprint cannot be partition-bounded and the K8
    /// guardrail (`require: partition_local`, the ratified default) refuses
    /// rather than shipping a silent full-table operation.
    ScanUnbounded { source: String, why: String },
    /// No technique survives admission for this trigger — fail loud, never
    /// silently fall back (`06-proof-obligations.md` §1.1).
    NoAdmissibleTechnique { trigger: String, why: String },
    /// An upstream maintained-model edge (`incremental_models.md` §"Upstream
    /// model edges") whose event-time clock cannot be derived — the upstream
    /// declares no `timeseries:` and none is inferable — so its
    /// creation-trigger cell cannot be clamped. Recorded (never a silent
    /// drop), naming the edge (the `MaintenanceReachNotDerivable` refusal).
    ReachNotDerivable { edge: String, why: String },
    /// A declared `grain:` this phase of maintenance-plan derivation does not
    /// yet support (e.g. `key_per_partition`). Names the grain and the plan
    /// tracking the missing support, rather than silently deriving a plan for
    /// a grain shape that was never actually admitted (the
    /// `MaintenanceUnsupportedGrain` refusal).
    UnsupportedGrain {
        grain: String,
        tracking_plan: String,
    },
    /// A `grain: key` model declares a `timeseries:` block but key temporal
    /// locality could not be established — no route applies (§"Key
    /// temporal locality"). `message` is the rendered `KeyedForbidsTimeseries`
    /// diagnostic (`locality::LocalityRefusal::message`): it names all
    /// three routes and the nearest missing fact.
    LocalityNotEstablished { message: String },
}

/// The admitted key-temporal-locality verdict for a `grain: key` model that
/// also declares a `timeseries:` block
/// (`locality::establish_locality`'s admitted result, plus the derived
/// settle bound). Carried on [`MaintenancePlan`] so `smelt-db` and `smelt
/// explain` can fold the already-admitted verdict into `Grain::Key`'s plan
/// shape and the explain surface without re-deriving admission — the
/// single derivation in `locality.rs` is the only place that decides both
/// (`CLAUDE.md` §"Maintenance-plan purity": "derived once by pure
/// functions … consumers never re-derive it").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLocality {
    /// The admitted slice a `merge_into` target scan may be pruned to.
    pub slice: locality::LocalitySlice,
    /// How long a written slice may still change before it is safe to
    /// treat as final (route 2 is honestly [`locality::SettleBound::Never`],
    /// never a large sentinel duration).
    pub settle_bound: locality::SettleBound,
}

/// The derived maintenance plan: admitted cells plus fail-loud refusals.
#[derive(Debug, Clone, Default)]
pub struct MaintenancePlan {
    pub cells: Vec<PlanCell>,
    pub refusals: Vec<Refusal>,
    /// The admitted key-temporal-locality verdict, for a `grain: key` model
    /// whose `timeseries:` block cleared the locality gate. `None` for a
    /// `grain: partition` model, a `grain: key` model with no `timeseries:`
    /// block, or a locality refusal (in which case the plan is
    /// [`locality_refused_plan`]'s no-cells shape instead).
    pub key_locality: Option<KeyLocality>,
}

impl MaintenancePlan {
    /// The admitted cell for `trigger`, if any (v0 plans hold at most one
    /// cell per trigger × group).
    pub fn cell_for(&self, trigger: &Trigger) -> Option<&PlanCell> {
        self.cells.iter().find(|c| &c.trigger == trigger)
    }
}

/// The plan tracking `grain:` shapes maintenance-plan derivation does not yet
/// support (`Refusal::UnsupportedGrain`'s `tracking_plan`).
pub const UNSUPPORTED_GRAIN_TRACKING_PLAN: &str =
    "docs/plans/20260715-composed-axes-conditional-maintenance.md";

/// The plan derived for a `grain:` this phase of derivation does not yet
/// support: no cells, a single [`Refusal::UnsupportedGrain`] naming `grain`
/// and [`UNSUPPORTED_GRAIN_TRACKING_PLAN`]. There is nothing meaningful to
/// derive for an unsupported grain, so this bypasses
/// [`derive::derive_maintenance_plan`] entirely rather than feeding it inputs
/// built from a grain shape it was never taught to admit.
pub fn unsupported_grain_plan(grain: &str) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::UnsupportedGrain {
            grain: grain.to_string(),
            tracking_plan: UNSUPPORTED_GRAIN_TRACKING_PLAN.to_string(),
        }],
        key_locality: None,
    }
}

/// The plan derived when the locality gate
/// ([`locality::establish_locality`]) refuses a keyed model's `timeseries:`
/// block: no cells, a single [`Refusal::LocalityNotEstablished`] carrying
/// the rendered `KeyedForbidsTimeseries` message. Bypasses
/// [`derive::derive_maintenance_plan`] entirely — there is nothing
/// meaningful to derive for a keyed output whose partitioning was never
/// admitted.
pub fn locality_refused_plan(message: String) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::LocalityNotEstablished { message }],
        key_locality: None,
    }
}

/// Fold a model's declared per-column equivalence contract
/// (`columns.<c>.contract:`, `smelt_core::metadata::Contract`) over the
/// walk's derived change-comparability verdict
/// (`analysis::walk::PropertyVector::comparability`,
/// `model_properties.md` §"Change comparability"): a `contract: plausible`
/// column is `Incomparable` regardless of what the walk proved — the walk
/// only sees the query's own SQL and cannot know that a payload column's
/// non-determinism has been accepted by the modeller as an equivalence
/// contract, so the override is applied here, where the derived vector meets
/// the model's declared metadata, not inside the walk itself. Widen-never:
/// a column the walk already proved `Comparable` and that carries no
/// `plausible` declaration passes through unchanged.
///
/// This is plain plumbing — carrying the verdict on a plan-facing type — no
/// admission or emitter reads the result yet.
pub fn column_comparability_with_contract(
    walk_comparability: &[ColumnComparability],
    plausible_columns: &BTreeMap<String, smelt_core::metadata::Contract>,
) -> Vec<ColumnComparability> {
    walk_comparability
        .iter()
        .map(|c| {
            let is_plausible = plausible_columns
                .get(&c.output.to_ascii_lowercase())
                .is_some_and(|contract| *contract == smelt_core::metadata::Contract::Plausible);
            let comparability = if is_plausible {
                Comparability::Incomparable
            } else {
                c.comparability
            };
            ColumnComparability {
                output: c.output.clone(),
                comparability,
            }
        })
        .collect()
}

#[cfg(test)]
mod comparability_contract_tests {
    use super::*;

    #[test]
    fn plausible_contract_forces_incomparable_regardless_of_walk_verdict() {
        let walk_comparability = vec![
            ColumnComparability {
                output: "amount".to_string(),
                comparability: Comparability::Comparable,
            },
            ColumnComparability {
                output: "notes".to_string(),
                comparability: Comparability::Comparable,
            },
        ];
        let mut contracts = BTreeMap::new();
        contracts.insert(
            "notes".to_string(),
            smelt_core::metadata::Contract::Plausible,
        );

        let out = column_comparability_with_contract(&walk_comparability, &contracts);

        assert_eq!(
            out.iter()
                .find(|c| c.output == "notes")
                .map(|c| c.comparability),
            Some(Comparability::Incomparable),
            "a plausible-contract column must be forced Incomparable regardless of \
             the walk's own (Comparable) verdict; got {out:?}"
        );
        assert_eq!(
            out.iter()
                .find(|c| c.output == "amount")
                .map(|c| c.comparability),
            Some(Comparability::Comparable),
            "a column with no plausible declaration must pass through unchanged; got {out:?}"
        );
    }

    #[test]
    fn no_contract_passes_the_walk_verdict_through_unchanged() {
        let walk_comparability = vec![ColumnComparability {
            output: "ts".to_string(),
            comparability: Comparability::Incomparable,
        }];
        let out = column_comparability_with_contract(&walk_comparability, &BTreeMap::new());
        assert_eq!(
            out, walk_comparability,
            "no declared contracts must leave the walk's verdict untouched"
        );
    }
}
