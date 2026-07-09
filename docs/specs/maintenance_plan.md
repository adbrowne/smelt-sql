---
feature: maintenance_plan
status: experimental
last_reviewed: 2026-07-07
owners: [andrew]
---

# The Maintenance Plan

> **What this is.** A normative spec for the **maintenance plan**: the derived, per-model object that says how each part of a model's output is kept current under each kind of change — a matrix indexed by `(output-column-group × trigger)` whose cells choose maintenance techniques — and the **graph layer** built on it: given what landed upstream, which partitions of which downstream models must run (forward propagation), and given a requested output period, which upstream slices must exist (backward resolution). Out of scope: the equivalence invariant, the algebraic ladder, the horizon, and validator-not-chooser (`model_maintenance.md` — this spec *consumes* that contract); the properties a model's SQL can be proven to have (`model_properties.md`); the physical transform mechanisms themselves (`model_transforms.md`); the `refresh:` axis and declaration law (`models.md`); source world-fact declarations (`sources.md`); per-mode surface (`batched_models.md`, `keyed_models.md`, `versioned_models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

### The plan (derived, reported)

Every non-`full` model has a **maintenance plan**: a set of **cells**, each keyed by
`(output-column-group × trigger)` and carrying:

- the **corner** of the read-scope × write-scope 2×2 the cell occupies (§Semantics);
- the **technique** that realizes it (`DELETE`+`INSERT` region recompute, keyed fold `MERGE`,
  column-scoped `MERGE`, in-place `UPDATE`);
- the **derived scan clamps** — per read source, the `(partition_col, before, after)` window the
  cell reads, anchored to the output region;
- the **partition-locality verdict** per source (§Semantics);
- the cell's **obligations** and any **traded guarantees** (per-column, two-dimensional:
  equivalence contract × settle bound).

The plan is **derived, never declared**. What stays declared is the model's output shape and
grain (`models.md`) — validated against the plan, an error on mismatch, never a silent flip.
`smelt explain` prints the plan: every cell, its clamps and locality verdicts, the per-column
guarantee ledger, and — at the graph level — the model's inbound edges.

### Triggers

Four trigger classes index the plan's columns:

- **creation** — new rows arrived in the driving source;
- **mutation** — a post-creation delta in a source some column group is mutation-sensitive to;
- **definition change** — the model gained output fields while sources stood still;
- **backfill** — an explicit region recompute from replayable input.

### Frontmatter

```yaml
maintenance:
  defaults:
    prefer: recompute | fold | auto        # per-model soft default (auto = cost model)
  cells:
    - columns: [<col>, ...]                # names any member of a derived column group
      on: <source-address> | backfill      # the trigger this cell handles
      prefer: fold | recompute             # soft per-cell bias (cost model still refines)
      technique: fold | recompute | rederive_columns   # hard per-cell pin (bypasses cost model)
  scan_bounds:
    require: partition_local | none        # default: partition_local
    on_violation: error | warn             # default: error
    per_source:
      <source-address>:
        max_lookback: '<interval>'         # ceiling on the derived scan span for this source
        allow_full_scan: true              # named acceptance of a full read of this source
```

- The override ladder is `defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
  scope winning; `technique:` alone bypasses the cost model. Almost every model sets none.
- `cells[].columns` naming columns that span two derived groups is an error (it would silently
  re-partition the plan).
- `scan_bounds` is **check-only**: it never modifies a clamp; it only refuses (or warns) when the
  derived plan exceeds the stated expectation. A project-level default in `smelt.yml` sets the
  baseline; per-model blocks refine it.

### CLI

- `smelt explain <model>` — prints the plan (cells, clamps, locality, guarantee ledger, edges).
- `smelt run --since-upstream` — forward propagation: compute per-source deltas from the recorded
  state, run exactly the propagated per-edge regions with their trigger cells. Opt-in; the
  intended default posture once trusted. Prints the dirty set before acting.
- `smelt build <model> --period <start>..<end> --include-upstreams` — backward resolution: print
  the per-ancestor required slices and build order; optionally execute the bounded build.
- `smelt bakeoff <model> [--cells ...]` — materialise each admissible technique for a cell over a
  representative window and report measured cost; `--pin` writes the choice as a `cells[]` entry.

### Diagnostics (the `Maintenance*` family; catalogued in `diagnostics.md`)

- `MaintenanceNoAdmissibleTechnique` — no technique survives a cell's admission; names the cell.
- `MaintenanceReachNotDerivable` — a required scan bound is neither derivable nor declared.
- `MaintenanceScanUnbounded` — the K8 guardrail: a scan/footprint cannot be partition-bounded (or
  exceeds a declared `max_lookback`) and no `allow_full_scan` acceptance exists.
- `MaintenanceUnboundedFootprint` — a targeted write was requested for a cell whose write
  footprint is unbounded (e.g. a stored trajectory under late data).
- `MaintenanceSkeletonColumnAdded` — a field was added in a skeleton position: a grain change,
  refused as a column backfill.
- `MaintenanceGraphUnsupportedNode` — a keyed-grain or self-referential node in the propagation
  graph (refused fail-loud; §Semantics).

## Semantics

### The plan matrix

The plan factors the output columns into **column groups** by shared mutation-sensitivity
(`model_properties.md` §"Per-column mutation-sensitivity / column provenance" — the proof and its
degenerate-collapse rule are defined there; this spec consumes the resulting groups as the plan's
column axis). Creation is shared by every column (all columns of a new row are computed
together); mutation is what partitions them.

Each `(group × trigger)` cell picks a corner of the 2×2 of **read scope** (delta+state vs the
region's full upstream input) × **write scope** (targeted addresses vs region overwrite):

|              | write: targeted | write: region-overwrite |
|---|---|---|
| **read: delta+state** | fold-a-delta | read-modify-write region |
| **read: full-input** | column-scoped re-derivation | recompute-a-region |

Recompute-a-region is contract-agnostic and unconditionally valid over replayable input; the
fold corner is contract-specific (it needs a combiner algebra — the ladder,
`model_maintenance.md`). Where the interchangeability conditions below hold, a recompute of a
region **supersedes** and resets what folds had written there.

"Unconditionally valid" is a correctness claim, not an admission or cost claim — it holds even in
the degenerate case where no partition bound exists and the region is the whole table (a
whole-table recompute is exactly a region taken to its limit). Whether that degenerate recompute
is *admitted* into the plan at all is a separate question, gated by the partition-locality
guardrail: see **"Partition-local maintenance (the K8 guardrail)"** below.

### Per-cell admission

A technique enters a cell's plan space only when all of its obligations discharge (fail-closed;
an unrecognised construct refuses, never defaults). The obligations, each with its owner:

1. **Replayable input** (recompute family) — the source is re-readable at its current processed
   set; declared posture, `sources.md`.
2. **Faithful fold** (fold family) — the fold's two independent conditions (source posture ×
   combiner algebra) hold (`model_properties.md` §"Faithful-fold conditions"); a replayable feed
   carrying retractions into a non-invertible combiner passes the first condition and fails the
   second, and either failure alone refuses the fold family for this cell.
3. **Combiner algebra class** — derived, fail-closed (`model_properties.md` discriminants); a
   holistic or unrecognised combiner leaves only the recompute family.
4. **Bounded reach** — the cell's scan bound `(clock_col, before, after)` per source is derived
   (`model_properties.md` §"Unified bound / reach derivation") or declared-and-checked; absent
   both, full-input techniques only (`MaintenanceReachNotDerivable` when the trigger requires a
   bound).
5. **Bounded footprint** (targeted writes) — the write-scope reflection of the scan bound is
   bounded (`model_properties.md` §"Footprint reflection / bounded write footprint"); a
   trajectory column's unbounded forward footprint fails this
   (`MaintenanceUnboundedFootprint`).
6. **Well-defined groups** — the mutation-sensitivity partition is computable
   (`model_properties.md` §"Per-column mutation-sensitivity / column provenance"); degenerate
   collapse is surfaced, never silent.

**Interchangeability and choice.** Two techniques may serve one cell interchangeably iff, at a
fixed processed-input set `S`, they produce identical state on the columns that decide which rows
exist (the `S`-indexed refinement of `model_maintenance.md`'s invariant; `S` is a **per-input
vector** once the plan factors). For faithful idempotent columns the choice is bit-preserving;
for additive columns it is state-preserving **modulo the ledger**, whose real obligation is
*never fold a delta already reflected in the state* — fold-then-recompute is safe (the recompute
resets the region's ledger), recompute-then-refold double-counts. Technique choice among
proven-interchangeable techniques is the cost model's (or the operator's, via `prefer`/
`technique`); it may change only *which `S` is reflected* (freshness), never observable bits at a
fixed `S` — this is how per-cell choice stays inside validator-not-chooser.

### Partition-local maintenance (the K8 guardrail)

A cell's per-`(cell × source)` locality verdict is the **partition-locality projection**
(`model_properties.md` §"Partition-locality projection" — the proof, including the cross-axis
predicate requirement, is defined there). This section owns only the policy consuming that
verdict: the emitted maintenance SQL must carry the partition predicate on **both** the scan and
the merge/overwrite target (a bound stated only on a non-partition column is one the storage
layer cannot prune by). Under the default `scan_bounds` (`require: partition_local`,
`on_violation: error`), a non-local cell refuses (`MaintenanceScanUnbounded`) unless the source
carries `allow_full_scan: true`; `max_lookback` additionally refuses a derived span wider than the
operator's stated expectation. The guardrail never modifies a clamp.

### The definition-change trigger

A model gaining output fields is a trigger of its own kind: the added group's processed-input
vector is `∅` over every existing region, and its backfill advances `∅ → current`, touching only
the new group. The classification of an added field —
`SkeletonAdd` / `PureBackfill` / `UpstreamRederive` — is the **definition-change column
classification** proof (`model_properties.md` §"Definition-change column classification"); this
section owns only the plan-level policy each classification maps to:

- `SkeletonAdd` (identity / grouping / dedup / ordering) is a **grain change**, refused as a
  column backfill (`MaintenanceSkeletonColumnAdded`) — the honest plan is a recompute,
  effectively a new model.
- `PureBackfill` lands in the 2×2's **targeted-write column** as an in-place `UPDATE` (no
  upstream read); `UpstreamRederive` lands there as a column-scoped `MERGE`, keyed where the
  source is keyed, inheriting each read source's partition-locality verdict unchanged.
- Fields added together factor by shared mutation-sensitivity (`model_properties.md` §"Per-column
  mutation-sensitivity / column provenance"), one backfill op per group. The backfill of a
  newly-added group is **always full-input**, even for a column whose ongoing algebra folds —
  there is no prior state of that column to fold onto.
- **Group convergence**: a field co-sensitive with an *existing* group still instantiates at `∅`
  and forms its own catch-up group; mid-catch-up, a delta folds into the sibling group but is
  refused on the new group's unbackfilled regions (the never-fold-ahead-of-the-entry rule). The
  groups merge only once the new group's processed vector equals its sibling's over every region.

### The reconciliation ledger

The plan's bookkeeping is a `(output-region × column-group)` ledger; each entry records the
processed-input vector `S_{i,g}` of that region-group. Storage is graded by algebra: additive
groups record **delta identities** (never-fold-twice needs them); idempotent groups record only a
**frontier** watermark (re-folding is harmless). The two operations: *fold* (refuse if the delta
is already in the entry's processed set; otherwise combine and extend) and *recompute-reset* (a
region recompute resets every intersecting entry to exactly the input it read). Region↔window
attribution is exact under key temporal locality or explicit footprint tracking; a delta is
attributed to the unique ledger region containing its footprint. Schema evolution is a ledger
operation: adding a group instantiates its entries at `S = ∅` (see above).

### The graph layer

**Edges.** A dependency edge is `downstream reads upstream` under the downstream cell's derived
scan clamp, between two partition axes whose **grain is the declared `timeseries.granularity`**
of each node — never per-edge, never derived from the SQL (the classifier only *checks* the
declaration, e.g. against a `date_trunc` grouping). Clamp margins ceil **outward** to whole
partitions; each hop aligns its result outward to the receiving axis's grain. Outward maps are
monotone, so sufficiency composes; narrowing never does (**widen-never-narrow** is the graph
layer's composition law).

**Forward propagation — what must run.** Runs are driven by **what landed**, per source, as
partition intervals on that source's own axis; a cron tick is only the poller. Processing nodes
in topological order, each node's merged dirt reflects through each outgoing edge — an upstream
delta of `[a, b)` dirties downstream `[a − after, b + before)` — accumulating:

- **per-edge dirt** `(model, upstream) → intervals`: keys the trigger cell — the plan cell for
  that inbound source runs over exactly these regions (recompute for a driving-source delta,
  column-scoped merge for an enrichment delta);
- **per-model dirt** (the union across inbound edges): what that model's own consumers see as
  *their* upstream delta.

Running exactly the per-edge dirty regions with their cells must leave every model equal to a
full refresh (sufficiency); partitions outside the dirty set are never scheduled. A delta on a
source nothing reads, or an empty delta, propagates nothing. A delta on an **unclocked** source
dirties the **whole model** for every mutation-sensitive consumer — never a silent no-op (the
cell was only admitted under `allow_full_scan`, so the full-table run is a declared cost).

**Backward resolution — what must exist.** Given a target model and period `[s, e)` (aligned
outward to the target's grain), walking the ancestor sub-DAG in reverse topological order and
applying each edge's clamp **directly** — `[s, e)` requires upstream `[s − before, e + after)` —
yields, for every ancestor, the partition intervals that must exist (a data prerequisite for a
raw source; a build region for a model) plus the **build order** (ancestor models in dependency
order, target last). This is the bounded test/validation build: stage exactly the resolved
source slices, build bottom-up, and the target period equals a build over complete history. The
required slice of an unclocked source is the whole table. The two directions are **adjoint, not
inverse**: `forward(backward(P)) ⊇ P`.

**Refusals.** The graph refuses fail-loud (`MaintenanceGraphUnsupportedNode`) on: a cyclic edge
set; a **self-referential** model (a table-graph cycle that is a DAG only when time-unrolled —
admissible in principle iff its self-clamp is strictly time-backward, with forward dirt running
to the frontier and backward resolution reaching the model's basis/checkpoint); a **keyed-grain**
node (no partition axis for interval dirt — silently treating it as day-axis would be
wrong-and-quiet).

### Interactions

- The equivalence invariant, ladder, horizon, and validator-not-chooser are owned by
  `model_maintenance.md`; this spec's per-cell theorem is the `S`-vector refinement of that
  invariant, and per-cell choice operates strictly inside its validator-not-chooser rule.
- Output shape/grain declaration and the refresh trichotomy are owned by `models.md`; the plan
  validates against them.
- Source postures (`mutation_profile`, lateness, retention, delta identity, unique keys) are
  declared in `sources.md` and consumed by admission; their runtime tripwires live there.
- The technique primitives (`merge_into`, DELETE+INSERT, column-scoped merge, targeted backfill)
  are catalogued in `model_transforms.md`; the outer output clamp is the subquery wrap over the
  model's output schema defined there.

## Design

**Strategy content is derived; shape and grain stay declared.** The single normative move
(`docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §10, §13): one model is not
one mode — it is simultaneously append-driven, merge-driven, and recompute-driven at different
`(group × trigger)` cells, so the strategy content of a refresh enum is a lossy projection and is
derived per cell. Deriving *shape* was considered and rejected: it reintroduces the silent
contract swap the declaration law exists to prevent (a projection refactor could flip downstream
consumption semantics with no diagnostic). Shape/grain remain declared-and-checked.

**Factoring by mutation-sensitivity, not syntactic provenance.** A column that reads a second
input's *immutable-at-creation* value must not inherit that input's mutation-sensitivity —
otherwise the plan degenerates and the targeted cells are lost. This is exactly what makes the
append-only declaration on a source load-bearing (01 §5).

**Per-edge dirt keys trigger cells.** The trigger taxonomy is per-edge, so a dirty set merged
per model would erase which repair runs where; two sources landing in one tick genuinely drive
different techniques over different regions of the same table
(`10-dependency-propagation.md` §3; ratified P4).

**Widen-never-narrow.** Every approximation in the plan and graph widens: partial-day clamps
ceil outward, coarse grains align outward, whole-partition dirt over-runs, an unclocked delta
dirties everything. Widening costs compute; narrowing costs correctness silently. The declared
guardrails (K8) exist so the widenings are *visible* costs, refused by default when unbounded.

**Grain is declared** (`timeseries.granularity`), consistent with the shape anchor: the
propagation grain governs downstream scheduling, so deriving it from a `date_trunc` projection
would let a refactor silently change scheduling semantics; the declaration is checked instead
(ratified P3).

**The clamp both directions.** Forward reflection and backward resolution are one edge object
run in opposite directions — the scan/footprint duality of 01 §5 lifted to the graph. Keeping
them one object is what makes the test-build story (backward) automatically consistent with the
scheduling story (forward); the adjointness containment is the honest statement of their
relationship (`10` §2).

**Offline cost measurement is first-class.** Because per-cell technique choice is
contract-preserving at fixed `S`, smelt may measure alternative physical plans over real data
offline and pin the cheapest (`smelt bakeoff`) — a capability per-query optimisers structurally
lack (01 §11).

**Rejected alternatives**, briefly: a `strategy:` sub-knob (dbt's invisible-contract footgun); a
new `smelt-maintenance` crate (the derivation needs the tightest coupling to the sibling
classifiers; module boundary kept extraction-mechanical instead — `08-code-placement.md` §2.1);
qualifying the output clamp to a resolved inner alias (answers a question the output clamp must
never ask — `03-design-forks.md` F1); a third addressing pole for locality (it changes no write
primitive — `model_maintenance.md` §Design); per-edge grain declarations (two declarations can
disagree; one per node cannot). Deeper rationale:
`docs/research/20260705-refresh-as-maintenance-plan/` (parts 01–10, with ratification records in
09 §1 and 10 §11).

## Constraints & Invariants

- **The plan is pure data, derived by pure functions, in one place** (`smelt-logical`);
  consumers — diagnostics, planner application, runtime lowering, the graph layer — never
  re-derive it. (Also recorded as an invariant in `architecture.md`.)
- **Never fold a delta already reflected in the state.** Every fold consults the ledger; every
  region recompute resets the entries it overwrote. No path may merge a window twice.
- **Write window = output window**, per cell: the DELETE/merge target and the output clamp range
  over the same output-axis column and the same window, by construction.
- **Only proofs prune.** A declared bound is admitted only checked; a guardrail (`scan_bounds`,
  `horizon_ceiling`) may refuse but never modifies a clamp; no unproven bound drops a scanned
  input.
- **Fail-loud, fail-closed.** Every admission failure, non-local scan, skeleton-position add,
  and unsupported graph node is a named diagnostic; nothing degrades to a silent fallback. The
  graph layer never silently under-runs: unrepresentable dirt widens to whole-model, never to
  nothing.
- **Widen-never-narrow** is the composition law of every interval operation (clamp ceiling,
  grain alignment, footprint reflection, backward widening).
- Out of scope, deliberately: content-aware delta pruning (an engine/CDF concern); file-level
  write-amplification minimisation (the engine's job — the plan guarantees the partition bound);
  cross-*project* propagation (project isolation, `architecture.md`).

## Known Divergences / Open Questions

- **The plan has three live consumers: diagnostics, `smelt explain`, and one execution
  technique.** `derive_maintenance_plan` (`crates/smelt-logical/src/maintenance/derive.rs`) is
  production code, not a tracer: full per-cell admission (`§"Per-cell admission"` obligations
  1–6, including the faithful-fold obligation's two independent conditions and the
  holistic-combiner cutoff), partition-locality verdicts, and the per-cell guarantee ledger
  fields are derived rather than hand-supplied, and `input_delta_discovery`
  (`model_properties.md`'s input-consumption proof stage) is a consumed admission input rather
  than dead code. A thin `maintenance_plan` Salsa query (`crates/smelt-db/src/queries/
  maintenance.rs`) assembles a model's referenced sources, declared output shape, and
  `maintenance:`/`columns.<c>.contract` frontmatter, calls the pure derivation, and folds two of
  the six `Maintenance*` diagnostics — `MaintenanceNoAdmissibleTechnique` and
  `MaintenanceScanUnbounded` — into `file_diagnostics()` (see `diagnostics.md` §Known
  divergences for the remaining four). `smelt explain <model>` reads the same derivation (via
  the non-Salsa `maintenance_plan_report`) and prints every cell's trigger, corner, technique,
  locality verdict, and scan clamps. On the execution side, the creation trigger's write
  strategy is read off the derived plan instead of a hardcoded constant (`smelt-runtime::
  maintenance_driver::resolve_incremental_strategy`), and the column-scoped `MERGE` technique
  is live and callable behind admission: `resolve_cell_technique` turns an admitted cell + the
  `maintenance.cells[].technique` hard pin + a backend capability gate
  (`Backend::supports_column_scoped_merge`) into an executable choice — a pin naming a cell the
  plan did not admit, or a capability gap on the backend, refuses rather than silently falling
  back — and `execute_column_scoped_merge` performs the targeted `MERGE` against a real backend.
  The regular incremental run loop (`smelt-runtime::execute_project`) dispatches into the
  column-scoped `MERGE` automatically on every run once the plan admits a mutation cell for one
  of the model's `explicitly_mutable` sources AND the target table already exists — no explicit
  "a mutation happened" signal is required to reach the technique; `resolve_live_column_scoped_cell`
  re-derives the same plan every run and the batch loop reads its verdict (exercised end-to-end
  in `crates/smelt-runtime/tests/technique_lowering.rs::column_scoped_merge_e2e` against the
  real `examples/timeseries/models/daily_events_enriched.sql` fact+dimension fixture, which
  drives the accepted-full-scan corner below). Two distinct physical corners exist for a live
  cell, chosen by `maintenance_driver::decide_column_merge_dispatch` from the cell's
  `partition_local` verdict: the accepted-full-scan corner (`PartitionLocal::No`, an unclocked
  dimension the operator declared `allow_full_scan` for) is the one currently reachable from any
  shipped example — `execute_column_scoped_merge_full` merges the model's own re-derivation of
  the batch window with no additional clamp. The horizon-clamped corner (`PartitionLocal::Yes`,
  a genuine derived `ScanClamp`, F15's `execute_column_scoped_merge`/`dimension_horizon_merge`,
  further gated on a provably one-to-one join contribution via
  `maintenance_driver::dimension_join_contribution`) is wired into the SAME dispatch path and
  proven end-to-end against a real backend
  (`crates/smelt-runtime/tests/technique_lowering.rs::yes_corner_clamps_the_merge_to_the_horizon_and_leaves_the_rest_untouched`),
  but is not yet reachable through any real workspace: `derive_model_maintenance_plan`'s own
  trigger-list construction (`crates/smelt-db/src/queries/maintenance.rs`) only ever emits a
  `Trigger::UpstreamMutation` for a source with no declared `timeseries` (an unclocked lookup) —
  a clocked mutable source's own scan-bound derivation is deferred, so no real fixture can
  currently derive `PartitionLocal::Yes` for that trigger regardless of how the runtime
  dispatches on it (`crates/smelt-runtime/tests/technique_lowering.rs::real_fixture_daily_events_status_would_admit_partition_local_yes_cell`
  proves the fixture and underlying derivation are correctly shaped for the moment that gate
  lifts). What still does not exist for either corner: nothing yet distinguishes "an upstream
  mutation genuinely happened since the last run" from "this run happens to re-derive the same
  values" — the dispatch fires on every run unconditionally once its preconditions hold; a
  cheaper, change-aware trigger is forward propagation's job (`smelt run --since-upstream`,
  unbuilt). The `defaults.prefer`/`cells[].prefer` soft-bias ladder and
  `scan_bounds.on_violation: warn` are parsed but not yet consumed (every refusal maps to an
  Error today; the cost model between two admissible techniques is also unbuilt). The
  `Trigger::UpstreamMutation` cell the query derives is scoped to `MutableSnapshot` sources
  only; an `AppendOnly` source's own aggregate-window sensitivity (real per
  `model_properties.md`'s mutation-sensitivity proof) has no post-creation mutation of its own
  to trigger a cell for, so no `UpstreamMutation` trigger is constructed for it — the
  `Backfill`/`NewData` triggers are unaffected. Migration ordering:
  `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md` §2.8 (M1–M6);
  `docs/plans/20260707-maintenance-plan-impl.md`.
- **Four of the seven maintenance-plan proofs are unbuilt** and hand-supplied in the tracer:
  footprint reflection, partition-locality projection, faithful-fold conditions, and
  definition-change column classification. Column-group-scoped dirt, gated by provenance, today
  coarsens to whole-partition — safe, over-running. Hour granularity is declared surface
  (`timeseries.granularity`) but the propagation layer is day-ordinal; sub-day axes are deferred.
  **Per-column mutation-sensitivity/column provenance, skeleton-role extraction, and the
  grain-alignment check are built**, as leaf classifiers over a model's own single top-level
  `SELECT` scope (`crates/smelt-logical/src/maintenance/grouping.rs`, `.../skeleton.rs`,
  `.../granularity.rs`): a model composed through a CTE, set operation, derived-table `FROM` item,
  or an unqualified reference ambiguous among more than one joined source is outside what any of
  the three classifiers resolves, and all fail closed on such a shape — mutation-sensitivity
  grouping collapses every non-skeleton column into one group sensitive to every declared source
  rather than guessing, and the caller may still hand-supply `ColumnGroup`/`skeleton_columns` for a
  shape wider than this. The grain-alignment check itself only *checks* the declaration against the
  model's own derived truncation/grouping unit (widen-never-narrow: a declaration coarser than or
  equal to the derived unit is a safe widen, never flagged; strictly finer is refused,
  `MaintenanceGranularityMismatch`) — the graph layer's edges still take the declaration directly,
  never derive it (P3 stands; the check only narrows how much a wrong declaration can go
  unnoticed). Full verdict definitions: `model_properties.md` §Surface "Derived proofs" (the
  `not-yet` rows). Build order and code placement: `docs/plans/20260707-maintenance-plan-impl.md`
  phases MP5 (footprint reflection, partition-locality), MP6 (faithful-fold), and MP14
  (grain-alignment check). Definition-change column classification remains unbuilt.
  `09-spec-readiness.md` §2.
- **The ledger has two storage substrates, one per grading.** `smelt-state`'s
  `smelt_state::reconciliation` module implements the `(output-region × column-group)` keying,
  the two storage gradings (additive groups keep delta identities; idempotent groups keep a
  frontier watermark), and both operations — fold-precondition-checked combine, and
  recompute-reset, which replaces every entry intersecting a recomputed region with exactly the
  input that recompute read — as a `.smelt/`-resident JSON store. A region recompute (the
  DELETE+INSERT batched technique) writes a recompute-reset entry per window under the whole-row
  group through that store, at the same point the legacy per-model frontier-only interval store
  (`smelt_state::intervals`) is written, without regressing that store's own behaviour. The keyed
  `merge_into` fold path additionally consults a second, **warehouse-resident** per-delta ledger
  table (`smelt_state::ddl_duckdb::generate_ledger_table_ddl`/`generate_ledger_insert_sql`) rather
  than the JSON store, because its fold must be transactional with the backend write it guards —
  a JSON file write cannot commit atomically with a database transaction. Every keyed-merge step
  folds its delta identity into that table via `smelt_backend::Backend::fold_ledger_delta`, in the
  same transaction as the step's create-or-merge action; a repeat delta violates the table's own
  `PRIMARY KEY` and refuses the run (`KeyedReprocessedWindow`, `docs/specs/keyed_models.md`
  §"Reprocessing") before the action ever runs a second time. An idempotent-only cell never
  creates this table — only an additive-graded cell needs never-fold-twice enforcement. The
  DuckDB-dialect DDL/DML is the only ledger substrate implemented today; an additive-graded cell
  on a non-DuckDB backend fails loudly (`UnsupportedFeature`) rather than being handed
  DuckDB-flavored SQL it cannot run — a Spark-dialect ledger builder is unbuilt.
- **Keyed-grain hops and self-referential nodes refuse** in the graph (by design, P7/P8); keyed
  dirt-sets and time-unrolled self-edges are designed (`10-dependency-propagation.md` §6, S12)
  and unbuilt.
- **Delta detection** is built for v1: per-source partition-interval deltas are recorded in the
  state store (`smelt_state::landed_deltas`), keyed by source address — an append-only clocked
  source's landing is interval-diffed against prior coverage; a `mutable_snapshot` or unclocked
  source always resolves to the whole-table delta. `change_feed` offset-based delta detection and
  snapshot diffing are not yet built — every source still resolves through the
  append-only-or-whole-table path regardless of a declared `change_feed` profile (P10). Nothing yet
  consumes the recorded deltas — the graph layer's forward propagation (`smelt run
  --since-upstream`) is the first consumer, unbuilt.
- **Straddle attribution without locality** (a per-key footprint chaining across history) is
  scoped out of the ledger's v1: locality-or-explicit-footprint only (01 §8's own caveat).
- **`models.md`'s refresh-axis rewrite is pending** (the `batched`/`keyed`/`versioned` strategy
  names are removed outright per ratified decision 5; shape profiles replace them); until it
  lands, mode specs still read as strategy peers. A proposed `on_column_add:
  backfill | leave_null | recompute` policy knob is noted, not yet surface.
- **User docs describe the trichotomy + grain surface, not yet the plan itself.** The
  `docs-site/` pages consistently describe `refresh: full | incremental | materialized_view`
  and `grain: partition | key | key_per_partition`, seeded from the worked example catalogue
  (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`). What they do
  not yet cover — because the underlying surface doesn't exist yet — is the maintenance plan
  itself: the `maintenance:` frontmatter block, `smelt explain`'s cell/clamp/ledger output,
  `--since-upstream`, `--include-upstreams`, and `smelt bakeoff`.

## Future Extensions

Ideas for widening the plan's admission space beyond what's decided above. Nothing here is
surface — no `maintenance:` field, diagnostic, or technique described in this section may be
relied on until it graduates into `§Surface`/`§Semantics` via its own spec diff and plan.

- **Row-local column derivation.** A recurring real-world shape: a column whose value is a pure
  function of *other columns already present in the same row* — a materialized date truncated
  from a timestamp, a normalized (lower-cased, hyphen-separated) rendering of a GUID column, an
  upper/lower-cased string column. When such a column is **added**, this is already the intended
  shape of the `PureBackfill` verdict (`§"The definition-change trigger"`; classification proof
  in `model_properties.md` §"Definition-change column classification"): per-column provenance
  proves the new expression reads only already-stored columns, so the backfill is an in-place
  `UPDATE` with no upstream read at all — no full-input recompute needed. That path is spec'd and
  tracked as unbuilt in `§Known Divergences` above; it does not need a new idea, only an
  implementation of `classify_definition_change`.
  - **The open extension is the changed-column case, not the added-column case.** The
    definition-change trigger only fires on a pure addition (the additive-only model-diff,
    `model_properties.md` §"Additive-only model-diff vs semantic change"). Redefining an
    *existing* column's expression — e.g. changing how the normalized GUID column is computed —
    has no described plan-level treatment today; it falls to whatever a general model-definition
    change does (unspecified here), which in practice means a full recompute even when the new
    expression is, itself, a pure function of other unchanged stored columns in the same row.
    A future extension could apply the same per-column-provenance test used for `PureBackfill`
    to a **changed** column's new expression: if it proves pure-function-of-stored-columns, admit
    a targeted in-place `UPDATE` over the existing region instead of the region-recompute
    fallback. This would need its own trigger (distinct from the additive-only definition-change
    trigger), its own diagnostic naming for when the provenance test fails closed, and a decision
    on how it composes with the reconciliation ledger (a redefinition invalidates the ledger's
    provenance identity for that group even though no upstream delta occurred).

## References

- **Code**: `crates/smelt-logical/src/maintenance/{mod,derive,emit,propagate}.rs` (tracer v0);
  `crates/smelt-logical/src/analysis/` (the classifiers admission consumes);
  `crates/smelt-runtime/src/{cumulative,maintenance_driver,dimension_horizon_merge,transformer,backfill}.rs`
  (today's technique executors and clamps); `crates/smelt-state/src/intervals.rs` (the degenerate
  ledger); `crates/smelt-backend/src/lib.rs` (technique primitives).
- **Tests**: `crates/smelt-logical/tests/{maintenance_tracer,maintenance_tracer_evolution,maintenance_tracer_propagation}.rs`
  (pure derivation-side assertions); `crates/smelt-runtime/tests/{tracer_maintenance,tracer_evolution,tracer_propagation}.rs`
  (the DuckDB equivalence oracles — moved from `smelt-cli` 2026-07-08, since they only depend on
  `smelt_logical::maintenance::*` + `duckdb`, both already available to `smelt-runtime`);
  `crates/smelt-maintenance-testkit` (dev-only, `publish = false`; the graduated Link-C in-process
  harness — the real-run-pipeline driver, the model-shape catalogue, the multiset-equality oracle,
  and the mutation-aware run-schedule generator/driver — wired as a dev-dependency of `smelt-cli`);
  `cargo test -p smelt-cli --test property_discovery` is the standing equivalence gate: it runs the
  Link-C schedule suite over the full model-shape catalogue against a real DuckDB backend on every
  `cargo test`, asserting emitted maintenance output equals a full refresh over adversarial
  append/lateness/mutation schedules. The per-cell probe modules under
  `crates/smelt-cli/tests/property_discovery/` that consume the testkit crate remain disposable
  research probes (see `.claude/scripts/property-experimental-gate.sh`); only the shared harness
  graduated.
- **User docs**: `docs-site/docs/index.md`, `docs-site/docs/guide/{incremental-models,sql-models,materializations}.md`,
  `docs-site/docs/concepts/how-it-works.md`, `docs-site/docs/reference/{timeseries,smelt-yml,cumulative-aggregate,cli}.md`
  describe the trichotomy + grain surface; the plan itself (the `maintenance:` block, `smelt explain`,
  the propagation CLI) is not yet user-documented (see Known Divergences).
- **Plans (history)**: `docs/plans/20260704-model-updates.md`,
  `docs/plans/20260704-model-updates-fundamentals.md` (the L1+L2 substrate),
  `docs/plans/20260705-property-discovery-loop.md` (the empirical engine).
- **Related specs**: `model_maintenance.md`, `model_properties.md`, `model_transforms.md`,
  `models.md`, `sources.md`, `batched_models.md`, `keyed_models.md`, `versioned_models.md`,
  `schema_evolution.md`, `timeseries.md`, `diagnostics.md`, `architecture.md`, `cli.md`.
