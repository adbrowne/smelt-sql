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

pub mod availability;
pub mod choice;
pub mod derive;
pub mod diff_patch;
pub mod edge_type;
pub mod emit;
pub mod granularity;
pub mod grouping;
pub mod locality;
pub mod probe_cadence;
pub mod propagate;
pub mod repair;
pub mod skeleton;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

pub use crate::analysis::fingerprint::Projection as FingerprintProjection;
pub use crate::analysis::skeleton_closure::{RowPreservation, SkeletonSourceClosure};
use crate::analysis::walk::{ColumnComparability, Comparability};
pub use probe_cadence::{should_dispatch, ProbeDispatch, SkipReason};

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
    pub state_downgrade: Option<availability::StateDowngrade>,
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

/// A fail-loud refusal: the trigger has no admissible technique, or admitting
/// one would be dishonest (`01-framework.md` §10; `06-proof-obligations.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A field was added or changed in a skeleton position — a grain
    /// change, not a column backfill (EX-39).
    SkeletonChanged { column: String },
    /// The model's skeleton *clause* itself changed against a prior
    /// deployed snapshot (a changed `GROUP BY`, a changed `FROM` target, a
    /// changed join shape) — a grain change proven by
    /// [`crate::backbuild::diff::definition_diff`]'s clause-level factoring
    /// rather than by a `Trigger::ColumnAdded` landing in a skeleton
    /// position. Maps onto the same `MaintenanceSkeletonChanged` diagnostic
    /// code as [`Refusal::SkeletonChanged`] — one code, two refusal shapes
    /// (`docs/specs/definition_deltas.md` §"Detection").
    SkeletonClauseChanged { reason: String },
    /// A `grain: partition` model's declared `timeseries.partition_column` —
    /// the address every partition-grain maintenance write targets — differs
    /// from the column recorded in the deployed-schema snapshot at last
    /// deploy. Unlike `SkeletonClauseChanged`, this is not a proof over SQL
    /// text: the address is a world-fact carried on
    /// [`super::derive::ModelInputs::old_partition_col`], compared
    /// case-insensitively against the output's own current
    /// `partition_col`. Maps onto `MaintenancePartitionColumnChanged`
    /// (`docs/specs/incremental_shapes.md` §"The partition grain"). Fails
    /// closed: `old_partition_col: None` (no snapshot, or one written before
    /// this field existed) derives no refusal.
    PartitionColumnChanged { from: String, to: String },
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
    /// A `grain: key` model's route-3 statically-derived recurrence bound
    /// disagrees with a declared `key_recurrence` over the same key
    /// (key-grain rule 16, `docs/specs/incremental_shapes.md` §"Key
    /// temporal locality"). Maps onto the `KeyedRecurrenceDeclarationMismatch`
    /// diagnostic. `message` is the rendered
    /// `locality::LocalityRefusal::RecurrenceDeclarationMismatch` text —
    /// names both values and the driving source.
    KeyedRecurrenceDeclarationMismatch { message: String },
    /// A `grain: key` model declares no top-level `unique_key:` and its own
    /// outermost SELECT's `GROUP BY` derives no key either (empty or
    /// absent) — there is no identity to maintain against. Maps onto the
    /// `GrainAssertionMismatch` diagnostic code, naming the asserted grain
    /// and the empty derived key (`models.md` §"Constraint violations").
    IdentityNotDerivable { message: String },
    /// The repair family's affected-key obligation (P7,
    /// `model_properties.md` §"Affected-key discovery") could not resolve a
    /// finite key set for `source`'s delta — the
    /// `MaintenanceRepairKeysNotDiscoverable` diagnostic. `why` carries
    /// [`crate::analysis::affected_keys::AffectedKeys::NotDiscoverable`]'s
    /// own reason, verbatim.
    RepairKeysNotDiscoverable { source: String, why: String },
    /// The repair family's per-group read could not be bounded to a slice
    /// (no reach / key-temporal-locality route applies) — the
    /// `MaintenanceRepairSliceUnbounded` diagnostic.
    RepairSliceUnbounded { source: String, why: String },
    /// A non-skeleton `Trigger::ColumnAdded` column cannot be backfilled in
    /// place — an unbounded scan for the column-scoped merge, no admissible
    /// technique, an unresolvable expression, or added columns in one group
    /// disagreeing on their definition-change classification. Unlike every
    /// other `Refusal` variant, this one does NOT block the model's ongoing
    /// maintenance plan: a run proceeds, ALTERs the column in, and leaves
    /// historical rows `NULL` until `smelt migrate` backfills them — the
    /// posture `docs/specs/definition_deltas.md` §"Detection" states, and
    /// mirrors `smelt-runtime`'s own run gate, which already exempts a pure
    /// column addition outright. Reported as a Warning
    /// (`MaintenanceColumnAddNotBackfillable`), never an Error.
    DefinitionChangeNotBackfillable { columns: Vec<String>, why: String },
    /// A `grain: key` model folds a retractable enrichment-join contribution
    /// (`analysis::join_shape::join_contribution_monotone` refuses it — a
    /// fanned-out or otherwise non-monotone join feeding a combiner that
    /// cannot undo a retraction) into a source whose repair admission
    /// (`repair::admit_per_group_recompute`) also cannot cover that
    /// retraction with a per-group recompute — the
    /// `KeyedRetractableContribution` diagnostic
    /// (`incremental_shapes.md` §"Enrichment joins"). Names the source, the
    /// affected fold column(s), and why (the join-contribution reason plus
    /// the failing repair obligation, verbatim). Always pushed alongside the
    /// pre-existing `NoAdmissibleTechnique` + `RepairKeysNotDiscoverable`/
    /// `RepairSliceUnbounded` refusals for the same trigger — additive, not
    /// a replacement. Steers toward `refresh: materialized_view` or
    /// composing the enrichment as a separate model.
    KeyedRetractableContribution {
        source: String,
        columns: Vec<String>,
        why: String,
    },
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
    /// The FIRST admitted cell for `trigger`, if any (v0 plans hold at most
    /// one cell per trigger × group, but a trigger commonly has MULTIPLE
    /// sibling cells — one per membership-sensitive group a shared join
    /// admits, `docs/plans/20260808-membership-sensitivity.md` Phase 1).
    ///
    /// **First-match, not "the" cell.** A caller resolving a per-cell
    /// override (`maintenance.cells[].technique`/`prefer`/`write`) must
    /// never use this alone to decide admissibility — the override's own
    /// `columns` may address a DIFFERENT sibling cell than whichever one
    /// this happens to return first. Use [`Self::cells_for`] and match each
    /// sibling's own derived column group instead (`docs/plans/
    /// 20260808-membership-sensitivity.md` Phase 3's own fix — the bug this
    /// doc comment now flags: `smelt-runtime`'s pin-resolution loops used to
    /// call this and evaluate overrides against only the first sibling,
    /// silently never consulting a pin scoped to a later sibling's
    /// columns). Safe call sites for `cell_for` alone are ones that only
    /// ever derive a single cell per trigger for their own shape (e.g. a
    /// `NewData`/`Backfill` whole-row `{*}` trigger, or a keyed model's
    /// single-group recipe) — read the call site's own shape before adding
    /// a new one.
    pub fn cell_for(&self, trigger: &Trigger) -> Option<&PlanCell> {
        self.cells.iter().find(|c| &c.trigger == trigger)
    }

    /// Every admitted cell sharing `trigger` — one per column group the
    /// trigger's mutation source contributes sensitivity to. Iteration
    /// order matches `self.cells`' own derivation order (not
    /// group-name-sorted); a caller that needs to pick ONE specific sibling
    /// must match on the sibling's own derived group/columns, never rely on
    /// this order to mean anything.
    pub fn cells_for<'a, 'b>(
        &'a self,
        trigger: &'b Trigger,
    ) -> impl Iterator<Item = &'a PlanCell> + use<'a, 'b> {
        self.cells.iter().filter(move |c| &c.trigger == trigger)
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

/// The plan derived when a declared `key_recurrence` disagrees with route
/// 3's statically-derived recurrence bound
/// ([`locality::LocalityRefusal::RecurrenceDeclarationMismatch`],
/// key-grain rule 16): no cells, a single
/// [`Refusal::KeyedRecurrenceDeclarationMismatch`] carrying the rendered
/// message. Bypasses [`derive::derive_maintenance_plan`] entirely — the
/// derived bound is authoritative, so a disagreeing declaration blocks the
/// plan the same way any other locality refusal does.
pub fn recurrence_mismatch_plan(message: String) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::KeyedRecurrenceDeclarationMismatch { message }],
        key_locality: None,
    }
}

/// The plan derived when a `grain: key` model has no derivable identity at
/// all — no declared top-level `unique_key:` and an empty GROUP-BY-derived
/// key. Bypasses [`derive::derive_maintenance_plan`] entirely — there is no
/// identity to fold cells around.
pub fn identity_not_derivable_plan(message: String) -> MaintenancePlan {
    MaintenancePlan {
        cells: Vec::new(),
        refusals: vec![Refusal::IdentityNotDerivable { message }],
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

// -------------------------------------------------------------------------
// Open write-pattern registry (`incremental_models.md` §"Per-cell write
// addressing", §"The write-pattern set is open").
// -------------------------------------------------------------------------

/// A contract fact the output must declare for a write pattern to be a
/// structural candidate — the first factor of the available-addressings
/// rule (`incremental_models.md` §"The available-addressings rule": "What
/// each declared fact gates").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractFact {
    /// A declared `unique_key` (row identity) — gates keyed `MERGE`,
    /// column-scoped `MERGE`, in-place `UPDATE`.
    Identity,
    /// A declared partition axis (`timeseries:`) to delete/scope by — gates
    /// region `DELETE`+`INSERT`.
    PartitionAxis,
}

/// The backend capability a pattern needs to execute at all — the fourth
/// admission factor (`incremental_models.md` §"The pattern set is
/// backend-relative"). `Always` patterns need nothing beyond the structural
/// facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteCapability {
    /// No backend-specific capability required.
    Always,
    /// Requires a keyed `MERGE` primitive.
    Merge,
    /// Requires a column-scoped `MERGE` (a target-row-preserving `MERGE …
    /// UPDATE SET *` over a full-row source projection).
    ColumnScopedMerge,
}

/// One entry of the open write-pattern registry: the name a `write:` pin
/// resolves against, the contract facts it structurally requires, and the
/// backend capability that gates it. New patterns plug in by adding an
/// entry here — the admission rule and equivalence gate, not the
/// enumeration, are the durable contract (`incremental_models.md` §"The
/// write-pattern set is open").
#[derive(Debug, Clone, Copy)]
pub struct WritePattern {
    pub name: &'static str,
    pub required_facts: &'static [ContractFact],
    pub capability: WriteCapability,
}

/// Which physical resolution a registry pattern maps to when it is
/// consulted as a `cells[].write` pin by [`choice::resolve_cell_choice`]
/// (`incremental_models.md` §"Per-cell write addressing" → "User pins":
/// the pin constrains technique selection, it does not merely validate and
/// get discarded). This is the single place the open registry's name space
/// meets the closed `Technique` enum a plan cell actually carries — new
/// registry entries must extend [`WritePattern::selects`], which is
/// exhaustive over `WRITE_PATTERN_REGISTRY`'s names (see that fn's `_ =>`
/// arm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteSelection {
    /// `region`/`full_rebuild` both select the always-available whole-region
    /// recompute — the `ChosenTechnique::RegionRecompute` corner.
    RegionRecompute,
    /// A targeted pattern selects the plan cell's own admitted [`Technique`]
    /// of that family. `keyed_conditional`/`staged_candidate` both select
    /// [`Technique::KeyedFold`]: they are alternate write *mechanisms*
    /// within that technique family
    /// ([`choice::KeyedWriteMechanism`]), not distinct plan techniques, so
    /// pinning either only constrains which `Technique` variant the cell
    /// must have admitted — the mechanism-within-the-technique choice stays
    /// [`choice::resolve_keyed_write_mechanism`]'s job.
    Technique(Technique),
    /// `diff_patch` selects the diff-then-patch write pattern
    /// (`incremental_models.md` §"The write-pattern set is open"): compute
    /// the candidate slice, diff against stored state, write only the
    /// difference. Its own admission ([`diff_patch::admit_diff_patch`]) is a
    /// separate proof from any `Technique`'s admission — a selection-name
    /// marker, no payload, matching `RegionRecompute`'s own shape.
    DiffPatch,
}

impl WritePattern {
    /// The [`WriteSelection`] this pattern maps to. Exhaustive over
    /// [`WRITE_PATTERN_REGISTRY`]'s names — a name reachable through
    /// [`lookup_write_pattern`] with no arm here is a registry bug, not a
    /// runtime input to handle gracefully, hence the `unreachable!`.
    pub fn selects(&self) -> WriteSelection {
        match self.name {
            "region" | "full_rebuild" => WriteSelection::RegionRecompute,
            "keyed" | "keyed_conditional" | "staged_candidate" => {
                WriteSelection::Technique(Technique::KeyedFold)
            }
            "column" => WriteSelection::Technique(Technique::ColumnScopedMerge),
            "update" => WriteSelection::Technique(Technique::InPlaceUpdate),
            "diff_patch" => WriteSelection::DiffPatch,
            other => unreachable!(
                "write-pattern registry entry '{other}' has no WriteSelection mapping — extend \
                 WritePattern::selects when adding a new registry entry"
            ),
        }
    }
}

/// The currently-known write-pattern set (`incremental_models.md` §"Per-cell
/// write addressing"): region rewrite, keyed/column-scoped/in-place targeted
/// writes, full rebuild, and the two conditional (change-suppressed)
/// mechanisms C4/C5 introduced — a `MERGE`-capable keyed conditional write
/// and the merge-less staged-candidate conditional `DELETE`+`INSERT` for a
/// backend without `MERGE` (`docs/specs/model_transforms.md` §"Change-
/// suppressed MERGE and the staged-candidate conditional DELETE+INSERT"). A
/// `write:` pin resolves by name against this table; an unrecognised name,
/// or one the target backend cannot provide, is
/// `MaintenanceWritePatternUnavailable` ([`resolve_write_pin`]).
pub const WRITE_PATTERN_REGISTRY: &[WritePattern] = &[
    WritePattern {
        name: "region",
        required_facts: &[ContractFact::PartitionAxis],
        capability: WriteCapability::Always,
    },
    WritePattern {
        name: "keyed",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Merge,
    },
    WritePattern {
        name: "column",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::ColumnScopedMerge,
    },
    WritePattern {
        name: "update",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Merge,
    },
    WritePattern {
        name: "full_rebuild",
        required_facts: &[],
        capability: WriteCapability::Always,
    },
    WritePattern {
        name: "keyed_conditional",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Merge,
    },
    WritePattern {
        name: "staged_candidate",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Always,
    },
    WritePattern {
        name: "diff_patch",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Always,
    },
];

/// Look up a registry entry by its open name (`maintenance.cells[].write`'s
/// resolution target). `None` for an unrecognised name.
pub fn lookup_write_pattern(name: &str) -> Option<&'static WritePattern> {
    WRITE_PATTERN_REGISTRY.iter().find(|p| p.name == name)
}

/// The output's declared contract facts a cell may draw on for the
/// available-addressings rule — plain data the caller (`smelt-db`,
/// `smelt-runtime`) already holds from the model's declared shape
/// (`ModelMetadata`); this module never re-derives it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputContractFacts {
    pub has_identity: bool,
    pub has_partition_axis: bool,
}

impl OutputContractFacts {
    fn satisfies(&self, fact: ContractFact) -> bool {
        match fact {
            ContractFact::Identity => self.has_identity,
            ContractFact::PartitionAxis => self.has_partition_axis,
        }
    }
}

/// Backend write-pattern capability, consulted as the fourth admission
/// factor (`incremental_models.md` §"The pattern set is backend-relative").
/// Derived from `smelt_dialect::BackendCapabilities` by the caller — this
/// module only consumes the two booleans it needs, never a concrete backend
/// type, staying below `smelt-backend`/`smelt-dialect` in the layering
/// (`CLAUDE.md` §"Layered single-ownership").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackendWriteCapabilities {
    pub supports_merge: bool,
    pub supports_column_scoped_merge: bool,
}

impl BackendWriteCapabilities {
    /// A backend that can run every registered pattern — used where no
    /// concrete target backend is known yet (e.g. `smelt explain` with no
    /// declared target), so the admissible set reports the structural
    /// answer rather than under-reporting against an assumed backend.
    pub fn all() -> Self {
        BackendWriteCapabilities {
            supports_merge: true,
            supports_column_scoped_merge: true,
        }
    }

    fn provides(&self, capability: WriteCapability) -> bool {
        match capability {
            WriteCapability::Always => true,
            WriteCapability::Merge => self.supports_merge,
            WriteCapability::ColumnScopedMerge => self.supports_column_scoped_merge,
        }
    }
}

/// Whether the target backend can execute `pattern` at all (the fourth
/// admission factor alone).
pub fn pattern_capability_available(
    pattern: &WritePattern,
    backend: BackendWriteCapabilities,
) -> bool {
    backend.provides(pattern.capability)
}

/// Whether the output's declared contract facts satisfy `pattern`'s
/// structural requirements (the first admission factor alone).
pub fn pattern_facts_satisfied(pattern: &WritePattern, facts: OutputContractFacts) -> bool {
    pattern.required_facts.iter().all(|f| facts.satisfies(*f))
}

/// Whether `pattern` is admissible for an output declaring `facts`, on a
/// backend with `backend`'s write capabilities — factors one and four of
/// the available-addressings rule combined (`incremental_models.md`
/// §"The available-addressings rule"). This does not evaluate the
/// trigger/changed-input factor or the per-cell equivalence-invariant
/// factor (three and two): those need a specific cell's proven row
/// identity / column comparability, which this module's registry-level
/// admission check does not have — see [`resolve_write_pin`]'s
/// `cell_can_uphold_equivalence` parameter for where that factor is
/// threaded in by the caller.
pub fn pattern_admissible(
    pattern: &WritePattern,
    facts: OutputContractFacts,
    backend: BackendWriteCapabilities,
) -> bool {
    pattern_facts_satisfied(pattern, facts) && pattern_capability_available(pattern, backend)
}

/// The admissible write-pattern name set for an output declaring `facts`, on
/// a backend with `backend`'s capabilities — every registry entry whose
/// structural facts and capability are satisfied. Feeds `smelt explain`'s
/// admissible-set row and the cost model's candidate pool.
pub fn admissible_write_patterns(
    facts: OutputContractFacts,
    backend: BackendWriteCapabilities,
) -> Vec<&'static str> {
    WRITE_PATTERN_REGISTRY
        .iter()
        .filter(|p| pattern_admissible(p, facts, backend))
        .map(|p| p.name)
        .collect()
}

/// Why a `maintenance.cells[].write` pin was refused
/// (`incremental_models.md` §"User pins"). Never a silent downgrade: a
/// refused pin never resolves to a substituted pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritePinRefusal {
    /// The name is not in the registry, or the target backend cannot
    /// provide the capability it needs.
    PatternUnavailable { pattern: String, backend: String },
    /// The name resolves in the registry and the backend can run it, but
    /// this cell's own facts (or a deeper equivalence proof the caller
    /// supplies) can't uphold its equivalence obligation — e.g. `keyed`
    /// pinned on an identity-free output.
    AddressingRefused {
        cell: String,
        pattern: String,
        why: String,
    },
}

impl std::fmt::Display for WritePinRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WritePinRefusal::PatternUnavailable { pattern, backend } => write!(
                f,
                "MaintenanceWritePatternUnavailable: write pattern '{pattern}' is unrecognised, \
                 or backend '{backend}' cannot provide it"
            ),
            WritePinRefusal::AddressingRefused { cell, pattern, why } => write!(
                f,
                "MaintenanceWriteAddressingRefused: write pattern '{pattern}' cannot uphold the \
                 equivalence invariant for cell {cell} — {why}"
            ),
        }
    }
}

/// Resolve a `write:` pin against the registry for one cell
/// (`incremental_models.md` §"User pins"). An unrecognised name, or one the
/// target backend cannot execute, is [`WritePinRefusal::PatternUnavailable`];
/// a name that is registry-recognised and backend-capable but whose
/// structural facts this output does not declare (or whose deeper
/// equivalence obligation `cell_can_uphold_equivalence` refuses) is
/// [`WritePinRefusal::AddressingRefused`]. The pin never widens the
/// admissible set: `Ok` only when the named pattern itself is the resolved
/// addressing.
pub fn resolve_write_pin(
    cell_label: &str,
    pin: &str,
    backend_name: &str,
    facts: OutputContractFacts,
    backend: BackendWriteCapabilities,
    cell_can_uphold_equivalence: impl Fn(&WritePattern) -> Result<(), String>,
) -> Result<&'static WritePattern, WritePinRefusal> {
    let Some(pattern) = lookup_write_pattern(pin) else {
        return Err(WritePinRefusal::PatternUnavailable {
            pattern: pin.to_string(),
            backend: backend_name.to_string(),
        });
    };
    if !pattern_capability_available(pattern, backend) {
        return Err(WritePinRefusal::PatternUnavailable {
            pattern: pin.to_string(),
            backend: backend_name.to_string(),
        });
    }
    if !pattern_facts_satisfied(pattern, facts) {
        return Err(WritePinRefusal::AddressingRefused {
            cell: cell_label.to_string(),
            pattern: pin.to_string(),
            why: format!(
                "the output does not declare the contract fact(s) '{pin}' requires \
                 ({:?})",
                pattern.required_facts
            ),
        });
    }
    if let Err(why) = cell_can_uphold_equivalence(pattern) {
        return Err(WritePinRefusal::AddressingRefused {
            cell: cell_label.to_string(),
            pattern: pin.to_string(),
            why,
        });
    }
    Ok(pattern)
}

/// The per-cell equivalence-invariant proof a `write:` pin's
/// `cell_can_uphold_equivalence` hook ([`resolve_write_pin`]) delegates to —
/// single owner of "does this cell's own derived facts uphold the pattern's
/// equivalence obligation" (`incremental_models.md` §"Per-cell write
/// addressing" → "User pins"). A **compare-based** pattern (`diff_patch`,
/// `keyed_conditional`, `staged_candidate` — the write mechanisms whose
/// physical form diffs a candidate row against prior state before writing)
/// delegates to [`choice::resolve_write_suppression`]'s P2 (row identity)/P3
/// (column comparability) proof and maps a
/// [`choice::WriteSuppression::Unconditional`] verdict to `Err`; every other
/// registry pattern (plain `region`/`keyed`/`column`/`update`/`full_rebuild`,
/// none of which compares against prior state) has no comparability
/// obligation at all and is unconditionally `Ok`.
pub fn cell_equivalence_proof(
    pattern: &WritePattern,
    group_columns: &[String],
    comparability: &[ColumnComparability],
    row_identity: &RowIdentityVerdict,
) -> Result<(), String> {
    match pattern.name {
        "diff_patch" | "keyed_conditional" | "staged_candidate" => {
            match choice::resolve_write_suppression(group_columns, comparability, row_identity) {
                choice::WriteSuppression::Suppressed { .. } => Ok(()),
                choice::WriteSuppression::Unconditional { why } => Err(why),
            }
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod cell_equivalence_proof_tests {
    use super::*;

    fn comparable(col: &str) -> ColumnComparability {
        ColumnComparability {
            output: col.to_string(),
            comparability: Comparability::Comparable,
        }
    }

    fn incomparable(col: &str) -> ColumnComparability {
        ColumnComparability {
            output: col.to_string(),
            comparability: Comparability::Incomparable,
        }
    }

    fn keyed_identity() -> RowIdentityVerdict {
        RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        }
    }

    #[test]
    fn compare_based_pattern_refuses_an_incomparable_group() {
        let pattern = lookup_write_pattern("diff_patch").expect("diff_patch registered");
        let err = cell_equivalence_proof(
            pattern,
            &["amount".to_string()],
            &[incomparable("amount")],
            &keyed_identity(),
        )
        .expect_err("an incomparable compared column must refuse");
        assert!(
            err.contains("amount"),
            "refusal must name the incomparable column: {err}"
        );
    }

    #[test]
    fn compare_based_pattern_accepts_a_fully_comparable_group() {
        let pattern = lookup_write_pattern("keyed_conditional").expect("registered");
        cell_equivalence_proof(
            pattern,
            &["amount".to_string()],
            &[comparable("amount")],
            &keyed_identity(),
        )
        .expect("a fully comparable group over a proven key must be admitted");
    }

    #[test]
    fn region_and_full_rebuild_patterns_need_no_comparability_proof() {
        let region = lookup_write_pattern("region").expect("registered");
        let full_rebuild = lookup_write_pattern("full_rebuild").expect("registered");
        let whole_row = RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        };
        cell_equivalence_proof(region, &[], &[], &whole_row)
            .expect("region has no comparability obligation");
        cell_equivalence_proof(full_rebuild, &[], &[], &whole_row)
            .expect("full_rebuild has no comparability obligation");
    }

    #[test]
    fn compare_based_pattern_refuses_a_whole_row_cell() {
        let pattern = lookup_write_pattern("staged_candidate").expect("registered");
        let whole_row = RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        };
        let err = cell_equivalence_proof(
            pattern,
            &["amount".to_string()],
            &[comparable("amount")],
            &whole_row,
        )
        .expect_err("no proven row identity must refuse regardless of comparability");
        assert!(err.contains("row identity"), "got: {err}");
    }
}

#[cfg(test)]
mod write_pattern_registry_tests {
    use super::*;

    #[test]
    fn every_registered_pattern_resolves_by_name_with_declared_facts() {
        let names: Vec<&str> = WRITE_PATTERN_REGISTRY.iter().map(|p| p.name).collect();
        for expected in [
            "region",
            "keyed",
            "column",
            "update",
            "full_rebuild",
            "keyed_conditional",
            "staged_candidate",
            "diff_patch",
        ] {
            assert!(
                names.contains(&expected),
                "registry must contain '{expected}'; got {names:?}"
            );
            let pattern = lookup_write_pattern(expected)
                .unwrap_or_else(|| panic!("'{expected}' must resolve by name"));
            assert_eq!(pattern.name, expected);
        }
    }

    #[test]
    fn keyed_pattern_not_admissible_without_identity() {
        let no_identity = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        let keyed = lookup_write_pattern("keyed").expect("keyed registered");
        assert!(
            !pattern_admissible(keyed, no_identity, BackendWriteCapabilities::all()),
            "keyed MERGE must not be admissible for an identity-free output"
        );

        let with_identity = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        assert!(pattern_admissible(
            keyed,
            with_identity,
            BackendWriteCapabilities::all()
        ));
    }

    #[test]
    fn region_pattern_not_admissible_without_partition_axis() {
        let no_axis = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let region = lookup_write_pattern("region").expect("region registered");
        assert!(
            !pattern_admissible(region, no_axis, BackendWriteCapabilities::all()),
            "region DELETE+INSERT must not be admissible for a clockless output"
        );

        let with_axis = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        assert!(pattern_admissible(
            region,
            with_axis,
            BackendWriteCapabilities::all()
        ));
    }

    #[test]
    fn column_and_keyed_conditional_gate_on_backend_capability() {
        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let no_merge = BackendWriteCapabilities {
            supports_merge: false,
            supports_column_scoped_merge: false,
        };
        let column = lookup_write_pattern("column").expect("column registered");
        let keyed_conditional =
            lookup_write_pattern("keyed_conditional").expect("keyed_conditional registered");
        assert!(!pattern_admissible(column, facts, no_merge));
        assert!(!pattern_admissible(keyed_conditional, facts, no_merge));

        // staged_candidate is the merge-less realisation — admissible with
        // identity alone, no backend capability required.
        let staged_candidate =
            lookup_write_pattern("staged_candidate").expect("staged_candidate registered");
        assert!(pattern_admissible(staged_candidate, facts, no_merge));
    }

    #[test]
    fn full_rebuild_always_admissible() {
        let facts = OutputContractFacts::default();
        let no_caps = BackendWriteCapabilities::default();
        let full_rebuild = lookup_write_pattern("full_rebuild").expect("full_rebuild registered");
        assert!(pattern_admissible(full_rebuild, facts, no_caps));
    }

    #[test]
    fn unrecognised_pin_refuses_pattern_unavailable() {
        let err = resolve_write_pin(
            "cell-1",
            "not_a_real_pattern",
            "duckdb",
            OutputContractFacts::default(),
            BackendWriteCapabilities::all(),
            |_| Ok(()),
        )
        .expect_err("unrecognised name must refuse");
        assert!(matches!(err, WritePinRefusal::PatternUnavailable { .. }));
        assert!(err
            .to_string()
            .contains("MaintenanceWritePatternUnavailable"));
    }

    #[test]
    fn backend_incapable_pin_refuses_pattern_unavailable() {
        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let no_merge = BackendWriteCapabilities::default();
        let err = resolve_write_pin("cell-1", "keyed", "spark_parquet", facts, no_merge, |_| {
            Ok(())
        })
        .expect_err("a backend without MERGE must refuse keyed as unavailable");
        assert!(matches!(err, WritePinRefusal::PatternUnavailable { .. }));
    }

    #[test]
    fn identity_free_keyed_pin_refuses_addressing_refused_never_downgrades() {
        let no_identity = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        let err = resolve_write_pin(
            "cell-1",
            "keyed",
            "duckdb",
            no_identity,
            BackendWriteCapabilities::all(),
            |_| Ok(()),
        )
        .expect_err("keyed on an identity-free output must refuse addressing");
        match &err {
            WritePinRefusal::AddressingRefused { cell, pattern, .. } => {
                assert_eq!(cell, "cell-1");
                assert_eq!(pattern, "keyed");
            }
            other => panic!("expected AddressingRefused, got {other:?}"),
        }
        assert!(err
            .to_string()
            .contains("MaintenanceWriteAddressingRefused"));
    }

    #[test]
    fn deeper_equivalence_refusal_is_addressing_refused() {
        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let err = resolve_write_pin(
            "cell-1",
            "keyed",
            "duckdb",
            facts,
            BackendWriteCapabilities::all(),
            |_| Err("no proven row identity for this cell".to_string()),
        )
        .expect_err("caller-supplied equivalence refusal must propagate");
        assert!(matches!(err, WritePinRefusal::AddressingRefused { .. }));
    }

    #[test]
    fn valid_pin_resolves_and_never_widens_the_admissible_set() {
        let facts = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        let resolved = resolve_write_pin(
            "cell-1",
            "region",
            "duckdb",
            facts,
            BackendWriteCapabilities::all(),
            |_| Ok(()),
        )
        .expect("region pin resolves for a partition-axis output");
        assert_eq!(resolved.name, "region");
    }
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
