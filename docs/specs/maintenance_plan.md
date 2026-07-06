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

The plan factors the output columns by **shared mutation-sensitivity**: for each column, which
sources' *post-creation* deltas can change its value. A reference to the row's own immutable
skeleton at creation time contributes no sensitivity. Columns with identical sensitivity sets
form one **column group**; a projection mutation-sensitive to two sources merges their groups
(fail-closed — in the degenerate limit one group covers the table and the plan collapses to a
whole-model story). Creation is shared by every column (all columns of a new row are computed
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

### Per-cell admission

A technique enters a cell's plan space only when all of its obligations discharge (fail-closed;
an unrecognised construct refuses, never defaults). The obligations, each with its owner:

1. **Replayable input** (recompute family) — the source is re-readable at its current processed
   set; declared posture, `sources.md`.
2. **Faithful fold** (fold family) — the delta stream *partitions* the input (append-only or a
   retraction-free feed; declared + tripwired), and the combiner's fold over any sub-multiset
   equals the batch aggregate (derived from the combiner class). These are independent
   conditions; a replayable feed carrying retractions into a non-invertible combiner passes the
   first and fails the second.
3. **Combiner algebra class** — derived, fail-closed (`model_properties.md` discriminants); a
   holistic or unrecognised combiner leaves only the recompute family.
4. **Bounded reach** — the cell's scan bound `(clock_col, before, after)` per source is derived
   from an explicit predicate on that source's partition column, or declared-and-checked; absent
   both, full-input techniques only (`MaintenanceReachNotDerivable` when the trigger requires a
   bound).
5. **Bounded footprint** (targeted writes) — the reflection of the scan bound maps an input delta
   to a bounded set of output addresses; a trajectory column's unbounded forward footprint fails
   this (`MaintenanceUnboundedFootprint`).
6. **Well-defined groups** — the mutation-sensitivity partition is computable; degenerate
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

A cell is **partition-local in source `i`** when its scan clamp projects onto a bounded interval
of every read source's partition column *and* its footprint projects onto a bounded interval of
the output's. The verdict is derived per `(cell × source)`; the emitted maintenance SQL must
carry the partition predicate on **both** the scan and the merge/overwrite target (a bound stated
only on a non-partition column is one the storage layer cannot prune by). A **cross-axis** source
(its partition column is not the output's) is linked only by an explicit, derivable predicate on
its partition column — smelt must not guess how an undeclared timestamp relates to a partition
column, and the zero-margin fallback of "no predicate found" is the absence of a link, not a
zero-cost one. Under the default `scan_bounds` (`require: partition_local`, `on_violation:
error`), a non-local cell refuses (`MaintenanceScanUnbounded`) unless the source carries
`allow_full_scan: true`; `max_lookback` additionally refuses a derived span wider than the
operator's stated expectation. The guardrail never modifies a clamp.

### The definition-change trigger

A model gaining output fields is a trigger of its own kind: the added group's processed-input
vector is `∅` over every existing region, and its backfill advances `∅ → current`, touching only
the new group. Rules:

- A field added in a **skeleton** position (identity / grouping / dedup / ordering) is a **grain
  change**, refused as a column backfill (`MaintenanceSkeletonColumnAdded`) — the honest plan is
  a recompute, effectively a new model.
- A payload field lands in the 2×2's **targeted-write column** by what it reads: a pure function
  of stored columns backfills as an in-place `UPDATE` (no upstream read, admitted only under the
  additive-only model-diff proof); a field re-deriving from upstream backfills as a column-scoped
  `MERGE`, keyed where the source is keyed, inheriting each read source's partition-locality
  verdict unchanged.
- Fields added together factor by shared mutation-sensitivity, one backfill op per group. The
  backfill of a newly-added group is **always full-input**, even for a column whose ongoing
  algebra folds — there is no prior state of that column to fold onto.
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

- **The plan is specified-and-unwired.** A v0 of the datatype, derivation, emission, and both
  propagation directions exists as a tracer (`crates/smelt-logical/src/maintenance/`), exercised
  by the tracer test suites and the property-discovery DuckDB legs, but nothing in production
  consumes it: `resolve_strategy` still returns a constant, diagnostics/`smelt explain`/the CLI
  flags do not exist. Migration ordering: `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md` §2.8 (M1–M6).
- **Never-fold-twice is specified and unenforced** — a **confirmed live violation**: the keyed
  `cumulative_aggregate`/`merge_into` run path re-folds an already-merged window and
  double-counts (no watermark/ledger consultation exists). Pinned by property-discovery cell
  G-12 (`crates/smelt-cli/tests/property_discovery/g_12_keyed_merge_reprocessed_window.rs`;
  ledger entry in `docs/research/property-discovery/ledger.md`). The fix is the ledger's
  fold-refusal operation, not a spot check.
- **The three hardest derivations are unbuilt** and hand-supplied in the tracer:
  mutation-sensitivity column grouping, skeleton-role extraction, and cross-model payload/column
  provenance (the latter also gates column-group-scoped dirt, which today coarsens to
  whole-partition — safe, over-running). `09-spec-readiness.md` §2.
- **The ledger substrate is the degenerate case**: `smelt-state`'s interval store is
  frontier-only and per-model; per-delta grading and `(region × group)` keying do not exist.
- **Grain checking is unbuilt**; the tracer takes edge-declared grains (a shortcut ratified away
  by P3). Hour granularity is declared surface (`timeseries.granularity`) but the propagation
  layer is day-ordinal; sub-day axes are deferred.
- **Keyed-grain hops and self-referential nodes refuse** in the graph (by design, P7/P8); keyed
  dirt-sets and time-unrolled self-edges are designed (`10-dependency-propagation.md` §6, S12)
  and unbuilt.
- **Delta detection** is committed as an interface (per-source partition-interval deltas recorded
  in the state store); the v1 mechanism is append-only landing/interval diff only — CDF offsets
  and snapshot diffing follow per `mutation_profile` (P10).
- **Straddle attribution without locality** (a per-key footprint chaining across history) is
  scoped out of the ledger's v1: locality-or-explicit-footprint only (01 §8's own caveat).
- **`models.md`'s refresh-axis rewrite is pending** (the `batched`/`keyed`/`versioned` strategy
  names are removed outright per ratified decision 5; shape profiles replace them); until it
  lands, mode specs still read as strategy peers. A proposed `on_column_add:
  backfill | leave_null | recompute` policy knob is noted, not yet surface.
- **User docs do not exist yet** for this feature; the `docs-site/` pages land with the
  implementation plan, seeded from the worked example catalogue
  (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`).

## References

- **Code**: `crates/smelt-logical/src/maintenance/{mod,derive,emit,propagate}.rs` (tracer v0);
  `crates/smelt-logical/src/analysis/` (the classifiers admission consumes);
  `crates/smelt-runtime/src/{cumulative,maintenance_driver,dimension_horizon_merge,transformer,backfill}.rs`
  (today's technique executors and clamps); `crates/smelt-state/src/intervals.rs` (the degenerate
  ledger); `crates/smelt-backend/src/lib.rs` (technique primitives).
- **Tests**: `crates/smelt-logical/tests/{maintenance_tracer,maintenance_tracer_evolution,maintenance_tracer_propagation}.rs`;
  `crates/smelt-cli/tests/property_discovery/{tracer_maintenance,tracer_evolution,tracer_propagation,g_12_keyed_merge_reprocessed_window}.rs`
  (equivalence oracles and the G-12 pin).
- **User docs**: none yet (see Known Divergences).
- **Plans (history)**: `docs/plans/20260704-model-updates.md`,
  `docs/plans/20260704-model-updates-fundamentals.md` (the L1+L2 substrate),
  `docs/plans/20260705-property-discovery-loop.md` (the empirical engine).
- **Related specs**: `model_maintenance.md`, `model_properties.md`, `model_transforms.md`,
  `models.md`, `sources.md`, `batched_models.md`, `keyed_models.md`, `versioned_models.md`,
  `schema_evolution.md`, `timeseries.md`, `diagnostics.md`, `architecture.md`, `cli.md`.
