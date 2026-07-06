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
//! - **Column groups are supplied by the caller**, not derived — the
//!   mutation-sensitivity partitioning is the single largest new derivation
//!   and is deliberately deferred; every research-loop cell likewise worked
//!   with hand-known groups.
//! - **Skeleton roles are supplied by the caller** (`OutputSpec::skeleton_columns`),
//!   not extracted from the SQL.
//! - Scan bounds are derived (`analysis::source_bounds`), combiner algebra is
//!   derived (`analysis::discriminants`), additive-only column adds are
//!   proven (`analysis::model_diff`) — the derivations that exist are
//!   consumed, the ones that don't are inputs.
//!
//! Nothing here is wired into diagnostics, planning, or execution; the module
//! is pure data + pure functions (Salsa-purity compatible by construction).

pub mod derive;
pub mod emit;

use std::collections::BTreeSet;

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
/// (`01-framework.md` §5). v0: supplied by the caller, not derived.
#[derive(Debug, Clone)]
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
}

/// The derived maintenance plan: admitted cells plus fail-loud refusals.
#[derive(Debug, Clone, Default)]
pub struct MaintenancePlan {
    pub cells: Vec<PlanCell>,
    pub refusals: Vec<Refusal>,
}

impl MaintenancePlan {
    /// The admitted cell for `trigger`, if any (v0 plans hold at most one
    /// cell per trigger × group).
    pub fn cell_for(&self, trigger: &Trigger) -> Option<&PlanCell> {
        self.cells.iter().find(|c| &c.trigger == trigger)
    }
}
