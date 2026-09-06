use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::analysis::fingerprint::Projection as FingerprintProjection;
use crate::analysis::skeleton_closure::SkeletonSourceClosure;

/// How a source's rows change after they first appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationProfile {
    /// Rows are only ever added; an existing row never changes or disappears.
    AppendOnly,
    /// The table is a mutable snapshot: rows may be updated or deleted in
    /// place with no change history.
    MutableSnapshot,
    /// The source exposes its own change-data feed (CDC/CDF), declared
    /// explicitly (`sources.md` §"`mutation_profile` — the structured
    /// block"). Admitted conservatively as full-input re-derivation — no
    /// live fold over the feed's own delta rows exists yet
    /// (`incremental_models.md` §Known Divergences).
    ChangeFeed,
}

impl MutationProfile {
    /// Single-owner answer to "is this posture mutable?" — every plan-layer
    /// site that widens a value/membership-sensitivity set or a repair
    /// posture to cover both `MutableSnapshot` and `ChangeFeed` consults
    /// this rather than restating the two-variant comparison at each call
    /// site.
    pub fn is_mutable(self) -> bool {
        matches!(
            self,
            MutationProfile::MutableSnapshot | MutationProfile::ChangeFeed
        )
    }
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
///
/// Sensitivity has two independent kinds (`docs/specs/model_properties.md`
/// §"Per-column mutation-sensitivity / column provenance", membership
/// paragraph). **Value sensitivity** (`mutation_sensitivity`) is per-column:
/// which sources' deltas can change a column's *stored value*. **Membership
/// sensitivity** (`membership_sensitivity`) is row-scoped: a mutable source
/// read in row-admission position (a JOIN's `ON` predicate) can retroactively
/// add or remove rows the model already materialized, even when no
/// select-item expression ever reads that source — so it attaches to
/// *every* payload group the admission read governs, not only the groups
/// that happen to read the source's columns. A source can be in one set, the
/// other, both, or neither. A membership-sensitive group must be repaired by
/// a technique that can create and delete rows — the recompute family,
/// never a column-scoped merge (`docs/specs/incremental_models.md`
/// §"The plan matrix").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnGroup {
    pub columns: Vec<String>,
    /// Value sensitivity: sources whose *post-creation* deltas can change
    /// these columns' stored values. Empty = never mutated after creation
    /// (pass-through columns, or a pure function of stored columns).
    pub mutation_sensitivity: BTreeSet<String>,
    /// Membership sensitivity: mutable sources read in row-admission
    /// position (a JOIN's `ON` predicate) whose deltas can add or remove
    /// rows this group covers. Empty = no admission read of a mutable
    /// source governs this group's rows.
    pub membership_sensitivity: BTreeSet<String>,
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
    /// One row per `(key, clock)` pair, patched via neighbour `LEAD`/`LAG`
    /// (`docs/specs/incremental_shapes.md` §"The succession grain"). Never
    /// declared — admitted only by the keyed-succession leaf classifier
    /// (`analysis::succession::classify_keyed_succession`) over an
    /// undeclared-grain `refresh: incremental` model.
    Succession {
        key_cols: Vec<String>,
        clock_col: String,
    },
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

/// The `contract.cells[].on` / `maintenance.cells[].on` addressing string a
/// cell's own [`Trigger`] resolves to — `"backfill"` for `Trigger::Backfill`,
/// the mutated source's address for `Trigger::NewData`/
/// `Trigger::UpstreamMutation`, and `None` for `Trigger::ColumnAdded` (a
/// definition-change trigger has no per-source address to match against a
/// declared cell override). Single-owned so `smelt-runtime` (building a
/// cell's effective contract point for the property profile) and
/// `smelt-cli` (the same resolution for `--json`'s `contract_point`) agree
/// by construction rather than by two independently maintained matches.
pub fn cell_trigger_address(trigger: &Trigger) -> Option<String> {
    match trigger {
        Trigger::NewData { source } | Trigger::UpstreamMutation { source } => Some(source.clone()),
        Trigger::Backfill => Some("backfill".to_string()),
        Trigger::ColumnAdded { .. } => None,
    }
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
///
/// `Serialize`: consumed directly as [`crate::maintenance::choice::
/// ChosenTechnique`]'s payload and, via the `smelt-runtime` diagnostics
/// builder, as a technique preview's own label
/// (`docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime builder") —
/// a plain additive derive, no variant or admission logic changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
    /// The repair family's per-group recompute (`incremental_models.md`
    /// §"The repair family"): a full-input read, targeted-write technique
    /// (the `ColumnMerge` corner) that recomputes and writes only the
    /// affected key groups a retraction/mutation delta names — `DELETE` the
    /// stored rows for those keys, `INSERT` their bounded recompute, never a
    /// whole-table operation. Admitted by [`repair::admit_per_group_recompute`]
    /// and emitted by [`emit::emit_per_group_recompute`].
    PerGroupRecompute,
    /// The succession grain's own technique
    /// (`docs/specs/incremental_shapes.md` §"The succession grain"): each
    /// window's non-delete events are inserted, delete events are recorded
    /// in the tombstone ledger, and each event's immediate neighbours are
    /// patched over the union of presented rows, ledger, and batch. Requires
    /// [`availability::StateStructure::TombstoneLedger`]
    /// (`availability::required_state_structure`); downgrades to
    /// [`Technique::DeleteInsert`] when unavailable
    /// (`availability::recompute_equivalent`).
    SuccessionPatch,
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
    /// The derived write footprint (`model_properties.md` §"Footprint
    /// reflection / bounded write footprint"), or `None` when the footprint
    /// question was never posed (a bare keyed output with no declared
    /// event-time axis). Populated once, at clamp construction
    /// (`derive::project_source_link`), from
    /// [`crate::analysis::footprint::reflect_footprint`]'s own
    /// `FootprintResult::Bounded{before, after}` value — never re-derived
    /// from this clamp's own read-side `before`/`after` margins.
    pub write_footprint: Option<(
        crate::analysis::source_bounds::Seconds,
        crate::analysis::source_bounds::Seconds,
    )>,
}

impl ScanClamp {
    /// The derived write footprint, or `None` when it was never posed
    /// (`write_footprint`'s own doc comment). A consumer reading `None`
    /// widens rather than treating an absent claim as zero-width
    /// (`model_properties.md` §"Footprint reflection / bounded write
    /// footprint").
    pub fn footprint(
        &self,
    ) -> Option<(
        crate::analysis::source_bounds::Seconds,
        crate::analysis::source_bounds::Seconds,
    )> {
        self.write_footprint
    }
}

/// A cell's region row identity (P2, `model_properties.md` §"Region row
/// identity"): what a conditional write joins stored rows to candidate rows
/// on. Precedence is declared `unique_key` → proven grain key (the walk's
/// `PropertyVector.grain`) → the identity-free `WholeRow` fallback, derived
/// once by [`derive::row_identity`] and carried here as plain data — no
/// consumer re-derives it (`CLAUDE.md` §"Maintenance-plan purity").
/// Fail-closed: a proven key that does not cover the output (a fan-out join)
/// is never trusted, even as a partial key — `WholeRow` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RowIdentity {
    /// Rows are addressed individually by this key.
    Key(Vec<String>),
    /// No usable key: rows are addressed as a whole — a conditional write
    /// degenerates to a multiset diff (delete-the-disappeared,
    /// insert-the-appeared), never a targeted update.
    WholeRow,
}

/// The row-identity verdict plus whether a declared key and a proven grain
/// key disagreed while both were present. Declared always wins the
/// precedence, but the disagreement is surfaced here rather than silently
/// dropped, so a caller (`smelt explain`, a future admission audit) can see
/// that the two facts disagree instead of only ever seeing the winner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowIdentityVerdict {
    pub identity: RowIdentity,
    /// `Some(proven)` exactly when a declared key was used *and* the walk
    /// separately proved a different (non-fan-out) grain key for the same
    /// output — the proven key that was overridden by precedence.
    pub proven_mismatch: Option<Vec<String>>,
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
    /// The region row identity a conditional write over this cell would join
    /// on (P2, `model_properties.md` §"Region row identity"). Plain data,
    /// derived once by [`derive::row_identity`] — no emitter or admission
    /// consumes it yet (that is a later phase's scope).
    pub row_identity: RowIdentityVerdict,
    /// The skeleton-source-closure verdict (P1, `model_properties.md`
    /// §"Skeleton-source closure") for this cell's enrichment join, when
    /// one is present. Plain data, derived once by
    /// [`crate::analysis::skeleton_closure::skeleton_source_closure`] —
    /// `None` when the cell has no enrichment join to close over (the
    /// common case for a `{*}` creation cell or a single-source model), not
    /// a refusal. No consumer reads this yet (that is a later phase's
    /// scope, the delta-restricted enrichment join transform).
    pub skeleton_source_closure: Option<SkeletonSourceClosure>,
    /// The fingerprint-projection verdict (P4, `model_properties.md`
    /// §"Fingerprint projection") for each external source this cell reads,
    /// keyed by source name — which of that source's columns feed the
    /// model's output, the column set a row-content fingerprint sidecar
    /// (`sources.md` §"The fingerprint sidecar") would digest instead of
    /// the source's full row. Plain data, derived once per model by
    /// [`crate::analysis::fingerprint::fingerprint_projection`] and shared
    /// across every cell of that model (the projection is a property of
    /// the model's own SQL against a source, not of any one trigger/
    /// technique). Empty for a model-edge cell (`derive::
    /// append_model_edge_cells`) — P4 is defined over external sources, not
    /// upstream maintained models. No consumer reads this yet (that is
    /// F3's sidecar-build/diff-query scope).
    pub fingerprint_projections: BTreeMap<String, FingerprintProjection>,
    /// The key-addressed read restriction (`incremental_models.md`
    /// §"Upstream model edges"), when this cell's technique is
    /// [`Technique::PerGroupRecompute`] over a `KeyedUpsert`-shaped upstream
    /// model edge rather than a partition-interval scan clamp. Additive
    /// alongside `scans` (empty for a key-addressed cell — there is no
    /// partition axis to clamp), the same additive-channel shape phase 5's
    /// keyed dirt-set took over `Propagation`'s interval maps
    /// (`docs/outcomes/20260809-output-delta-typing/outcome.md` 2026-08-10
    /// decision log). `None` for every other cell — unaffected.
    pub key_scope: Option<KeyScope>,
    /// Set by [`availability::resolve_availability`] when this cell's ideal
    /// technique required a [`availability::StateStructure`] with no
    /// available realisation for this project (`state.md` §"The degradation
    /// contract") — `None` for every cell straight out of ideal derivation,
    /// since availability resolution is a distinct, later step no deriver
    /// consults (maintenance-plan purity, `state.md` §"The degradation
    /// contract" step 1 "Ideal derivation").
    pub state_downgrade: Option<crate::maintenance::availability::StateDowngrade>,
}

/// A key-addressed read restriction: recompute only the rows identified by
/// `keys` (the upstream's own change-feed identity columns), sourced from
/// `from` (the upstream model edge's name). Carried on [`PlanCell`] rather
/// than folded into [`ScanClamp`] — a key set is not an interval, and the
/// existing partition-locality proof (`derive::project_source_link`) is not
/// posed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyScope {
    pub keys: Vec<String>,
    pub from: String,
    /// Which of [`crate::maintenance::repair::admit_key_addressed_recompute`]'s
    /// two discovery routes admitted this cell — plan data, not re-derived by
    /// the runtime (maintenance-plan purity).
    pub discovery: KeyDiscovery,
}

/// The discovery route [`crate::maintenance::repair::admit_key_addressed_recompute`]
/// used to resolve a key-addressed model edge's affected-key set
/// (`docs/specs/incremental_models.md` §"Upstream model edges").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum KeyDiscovery {
    /// The downstream's own grain resolves through the upstream's own
    /// `KeyedUpsert` key columns directly — the group-grain sidecar groups
    /// at the upstream's key columns, and changed upstream keys are
    /// projected forward onto the downstream's own key columns.
    UpstreamKeyed,
    /// The downstream's grain columns are columns of the upstream relation
    /// itself (not the upstream's own key columns) — the group-grain
    /// sidecar groups directly at the downstream's own grain, projected
    /// over the upstream relation, and the diff's own changed-key set is
    /// the downstream's affected-key set with no forward projection.
    DownstreamGrainOverUpstream,
}
