---
feature: maintenance_plan
status: experimental
last_reviewed: 2026-07-10
owners: [andrew]
---

# The Maintenance Plan

> **What this is.** A normative spec for **model maintenance**: the contract every maintained (non-`full`) model upholds — the equivalence invariant, the algebraic ladder, the windowed-scan/horizon contract, validator-not-chooser, and the composition contract the shape-profile specs build on — and the **maintenance plan**: the derived, per-model object that says how each part of a model's output is kept current under each kind of change — a matrix indexed by `(output-column-group × trigger)` whose cells choose maintenance techniques — and the **graph layer** built on it: given what landed upstream, which partitions of which downstream models must run (forward propagation), and given a requested output period, which upstream slices must exist (backward resolution). Out of scope: the properties a model's SQL can be proven to have (`model_properties.md`); the physical transform mechanisms themselves (`model_transforms.md`); the `refresh:` axis, the declaration law, and the litmus rule (`models.md`); source world-fact declarations (`sources.md`); per-profile surface (`batched_models.md`, `keyed_models.md`, `versioned_models.md`, `materialized_view.md`); backend capability flags (`multi_backend.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

### The composition contract

This section is **system surface**: its callers are the shape-profile specs (`batched_models.md`, `keyed_models.md`, `versioned_models.md`, `materialized_view.md`) and the planner/analysis layer, not the modeller directly.

A maintained model is a **composition** of three kinds of thing:

- **Properties** — what a model's SQL can be proven (or declared) to be: the monotonicity trace, the algebraic discriminants, partition alignment, and the rest (`model_properties.md`).
- **Transforms** — the physical mechanisms a property licenses: keyed `merge_into`, source-filter pushdown, partition DELETE+INSERT, and the rest (`model_transforms.md`).
- **Output shape** — declared via `grain:` (`models.md` §"Refresh axis"): partition-addressed (a complete table with a `partition_column`) or key-addressed (one row per key; optionally time-partitioned under key temporal locality — `keyed_models.md`).
- **Scope maps** — the per-input dispatch: for each input of a model, the derived mapping from that input's delta to the affected output addresses and the transform that runs for it. The driving source's delta engages the windowed fold; a mutable dimension's delta engages the delta-driven probe + dimension-driven horizon-bounded MERGE; a self-edge engages ordered execution; a model-definition diff engages the targeted column backfill (all `model_transforms.md`). Which map applies follows from input-delta discovery (`model_properties.md`) and the input's declared world-facts (`sources.md`); a run is the union of its inputs' scope maps — "what runs when *this* input changes" is a first-class, per-input answer, surfaced by `smelt explain`.

Every shape-profile spec must present a **composition table** stating, for that profile: the properties it requires, the world-facts it consumes, the transforms it drives — differentiated per input class where they differ (the profile's scope maps) — and its output shape. A profile spec's normative content is exactly (a) that composition table, referencing shared capabilities **by name**, plus (b) the profile's own **local** machinery, defined in full. It must not re-specify a capability that a capability spec owns.

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
  With `--show-sql`, additionally prints each cell's emitted maintenance statements — the same
  emitters' output a run executes (§"Statement emission (single owner)"; flag surface in
  `cli.md` §"`smelt explain <model>` maintenance-plan report").
- `smelt run --since-upstream --source <address> --landed <start>..<end>` (`--source`/`--landed`
  repeatable, one pair per source) — forward propagation: the runner (or an external poller)
  declares what landed for each source since it last propagated; the graph reflects those
  declared per-source deltas through the edges and runs exactly the propagated per-edge regions
  with their trigger cells. No per-invocation delta is computed automatically — a source named
  without a matching `--landed` delta propagates nothing for that invocation. Opt-in; the
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

### The equivalence invariant

This is the parent contract of the whole family. Every maintained (non-`full`) model upholds **one** invariant, stated over an abstract **processed-input set**: **an incremental run produces the result a full refresh would, restricted to the inputs it has processed so far.** Formally, for the processed input set `S`, `incremental_state(S) == full_refresh(source | input ∈ S)`. `S` is a set of *source rows/partitions the run has scanned*, not necessarily a clock-addressed partition set — the **partition-set form** (`source | partition_col ∈ S`, the form used throughout the rest of this spec and the profile specs) is the **clocked specialisation** of this invariant, available whenever the driving source carries a `timeseries:` clock; an unclocked (snapshot) source has no partition set to slice by, and its specialisation is stated per shape profile (e.g. `keyed_models.md` §"End-state equivalence" states it over "keys present in the current snapshot").

**Order/set-determinacy is a corollary, and it holds for every shape profile — the partition grain included.** The right-hand side depends only on the *set* `S`, never the order it was processed, so any conforming profile is order-independent. This is not special to the key-addressed shapes: a partition-grain model's partitions are disjoint, so its combiner is disjoint union (a commutative monoid) and the property is trivial — but it is present.

The shape profiles differ not in *which* equivalence they satisfy but in **how the output is addressed for update** — the axis that actually drives the physical transform and the identity requirement:

- **Partition-addressed** (identity-free — `grain: partition`): output is addressed by `partition_column`; a source partition maps to an output partition rewritten wholesale (DELETE+INSERT), no row identity needed. Here equivalence is additionally checkable slice-by-slice — *per-partition equivalence* — a **strengthening** of the one invariant, available because each output slice depends only on its own source partition (partition-local).
- **Key-addressed** (identity-requiring — `grain: key`, `versioning: interval`, `refresh: materialized_view`): output is addressed by a key; each processed input contributes a delta merged into the keyed state (`merge_into`). The write reaches stored rows **by key, wherever they live** — it is *not* bounded by the incoming data's time window. The interval-versioned profile (SCD2, `versioning: interval`) is the sharp case: admitting a new value for a key requires closing the previously-open version, a row whose timestamp lies arbitrarily far outside the current input window — which is exactly why a key-addressed output cannot be maintained as a per-partition rewrite. Equivalence is checked on the end-state.

Key-addressing admits a **derived refinement**: a key-addressed output that also carries a `timeseries:` partition column, admitted when **key temporal locality** is established — every stored row a run's deltas can touch provably lies within a derived slice of the output's time axis (`keyed_models.md` §"Key temporal locality"). The write is still a keyed `merge_into`; locality licenses pruning the merge's *target scan* to the slice, and makes **per-slice equivalence** — the keyed analogue of per-partition equivalence — available as the same kind of strengthening. SCD2's close-out is why this is a per-model *established fact*, not a key-grain default: some key-addressed writes intrinsically escape every time window.

So per-partition equivalence is not a peer of some separate "end-state equivalence" — it is a strengthening of the single invariant that partition-addressed, partition-local output enjoys. The key-addressed shapes discharge the *same* invariant on their end-state because their writes are keyed rather than partition-local. Every property is proven in service of this invariant; every transform is licensed **because it preserves it**. For the smelt-driven shapes the invariant is discharged by the generative equivalence oracle (§References), the family's regression net; for `materialized_view` it is discharged by the **engine's** native IVM, not the smelt oracle (smelt runs no combiner for that shape — §"Validator, not chooser").

**The replayability split.** Full equivalence — an executable `full_refresh` oracle a test can actually run — holds only for **replayable inputs**: a set `S` the model can re-evaluate its own SQL over (a clocked source's processed partitions; a snapshot's keys currently present). v1 admits **only** combinations whose oracle is executable this way (this is exactly what `keyed_models.md` §"Admission matrix" enforces per column). The designed-but-unshipped **third column** for the combinations that are not admitted — a non-replayable input under a partitioned output, or a fold family that would need to have observed history it cannot replay — is an **observer / prefix-consistency contract**: a different, weaker equivalence (a property of the *observation sequence*, not a re-runnable full refresh) that a future opt-in could state and admit explicitly, rather than being smuggled in under the executable-oracle invariant this spec states. It is not specified here; each shape profile's Known Divergences records where it would apply.

**Two named carve-outs.** Every admitted keyed model's executable oracle carries exactly two carve-outs, both **named consequences of the executable-oracle requirement, not gaps in it**:

- **Retained departed keys** under an unclocked (snapshot-reconcile) posture: a key present in the stored state but absent from the current snapshot is retained, not deleted, so the stored table is *the oracle's rows plus retained departed keys* — a documented divergence from a hypothetical delete-on-absence oracle (`keyed_models.md` §"The two run shapes", §"End-state equivalence").
- **Ordering-key ties** on an order-monotone overwrite column (`MAX_BY`/`MIN_BY`): equivalence holds up to ties on the ordering expression, because the classifier cannot statically prove ordering-key uniqueness (`keyed_models.md` §"Ordering ties").

The interval-versioned profile's oracle is its end-state equivalence in the **interval-keyed specialisation** — the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same processed snapshots, independent of merge order (`versioned_models.md` §Semantics).

### The algebraic maintenance ladder

What a key-addressed model can maintain is fixed by the **algebra of its combiners**, not by any backend feature. The ladder is a partial order whose ordering criterion **is** invertibility → maintainability — which is why it lives here (with the invariant) and not in `model_properties.md`: the *discriminants* it reads (is-monoid, needs-inverse, decomposable, value-vs-order-monotone) are raw properties of the SQL and are owned by `model_properties.md`; the ladder — the ordering *and* the maintainable-vs-delegated cutoff — is the maintenance consequence and is owned here. The equivalence invariant holds unconditionally on every rung; only the state representation and its size change across rungs, never the fidelity of the user value.

1. **Direct monoid.** The stored column *is* the answer; the combiner is a commutative monoid (associative, commutative, identity = empty partition): `SUM`/`COUNT` (`+`, 0), `MIN`/`MAX` (±∞), `BOOL_*`, `BIT_*`.
2. **Decomposed monoid.** The user value is `π(state)` for a richer monoid element and a pure presentation map `π`: `AVG` = `(sum, count)` presented `sum/count`; variance = a Welford triple; approximate distinct = an HLL register vector. Kept in a state table, exposed through a presentation view.
3. **Group.** When inputs can change (corrections, reprocessing, deletes) the combiner must be **invertible** — a commutative group (`SUM`, `COUNT`, `BIT_XOR`). Monoids that are not groups (`MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR`) cannot un-see a contribution and so cannot be reprocessed without a full refresh.
4. **Opt-in bounded-domain multiset.** Holistic aggregates needing all rows (exact `MEDIAN`/`PERCENTILE`/`MODE`/quantiles, exact `COUNT(DISTINCT)`, and `DISTINCT`-modified aggregates) are maintained by storing the per-key value→count multiset (a bounded-domain Z-set). Its **signed** (Z-set) form makes retraction free even for the otherwise-irreversible `MIN`/`MAX` — the multiset carries the underlying values a bare monoid discards. **Opt-in and fail-loud**: state is `O(active domain)`, so an unbounded-state aggregate is default-refused (suggesting the approximate form or `refresh: full`) unless the modeller supplies a bounded-domain budget, and the runtime caps the multiset with a full-refresh fallback.

The ladder is the boundary: rungs 1–4 are what smelt maintains itself (a `merge_into` loop, optionally with a presentation view). Beyond it — general-operator retraction over joins, unbounded non-additive state — is **not** smelt-driven-maintainable and is delegated to the engine's native incremental-view maintenance via `refresh: materialized_view`.

### Windowed maintenance and the horizon

Maintenance runs over a **bounded input window by default** — a full scan is the degenerate fallback, not the baseline. A run reasons about two windows, always with `scan ⊇ write`:

- the **write window** — the partitions or keys written this run;
- the **scan window** — the input rows read to produce that write window correctly.

The scan window is bounded **where the model carries a `timeseries:` clock**: input-delta discovery is window-forward, so only the new window (plus a lookback) is read and stored state stands in for history. Without a clock the source can only be snapshot-diffed, so the scan degrades to a full read (`models.md` §"Input-consumption axis"). This is orthogonal to output addressing: a clocked *key-addressed* model still windows its **scan** even though its **write** reaches back by key outside that window (the SCD2 close-out above). Bounding the scan never weakens the invariant — the engine evaluates the model, joins included, over the widened scan window and the write is **clamped** to the exact write window (`model_transforms.md`, "widened scan + exact clamp"), leaving join optimisation to the engine rather than smelt hand-computing a minimal delta.

The **horizon**, as a **write-eligibility clamp** (a bound on which keys/partitions a run may *write to*), is a concept that applies only to the **partition grain**: the far edge of the maintained window, the point past which inputs are no longer folded in. It is **derived**, never trusted from a declaration: the clamp bounds are computed from the model's own reach (its lookback, window frames, and join contribution — `model_properties.md`), because a declared horizon smaller than the true reach would make the clamp drop rows that should have been rewritten. A modeller **may** declare a horizon *ceiling* (frontmatter key `horizon_ceiling:`, e.g. `horizon_ceiling: '30 days'`) — smelt warns at compile time when the derived horizon would exceed it — but the clamp always uses the derived value.

Because the horizon is *derived*, the clamp is the model's own SQL: a genuinely late arrival — one that lands after its natural partition has passed the horizon — is **silently excluded** from the maintenance run, not diagnosed. smelt cannot fail loud on a row it never scans; the invariant's "inputs processed so far" is exactly the scan window bounded by the derived horizon, and rows outside it are outside "so far" by construction. **Surfacing lateness is therefore a model-author concern, not a maintenance guarantee.** The available pattern is to fold the late row into the current partition — re-stamping its partition time — carrying a lateness/validity flag so its *data still flows*, and let a data-quality check raise on the flagged rows while valid data passes through. The maintenance layer clamps; it does not police lateness.

**The key grain has no write-eligibility clamp.** Unlike the partition grain, a `grain: key` run merges **every** delta row it scans, into whatever key it names, however old that key is — there is no bound on which keys a run may touch (`keyed_models.md` §"No write-eligibility clamp"). A **derived forward reach** is still computed and reported (via `smelt explain`) for observability, but it never gates admission and never bounds a write. This is a deliberate difference from the partition-grain horizon above, not an oversight: the keyed write is proportional to delta size regardless of how far back the touched keys live, so a write clamp buys nothing for correctness and would silently drop scanned inputs — the one thing the equivalence invariant forbids. What a keyed clamp would buy (settled-key GC, a bounded working set) is deferred optimisation that, if ever introduced, must ship together with late-fact accounting (`docs/research/20260705-keyed-collapse-application.md` D6). The narrow principle beneath both stances: **only proofs prune; a declared bound is admitted only checked (fail-loud on violation); no unproven bound ever refuses a write.** Target-scan slice pruning under established key temporal locality (`keyed_models.md` §"Key temporal locality") conforms — the derived routes prune by proof, the declared key-recurrence route prunes only under a transactional runtime check, and every scanned delta row still merges. Two `model_transforms.md`-catalogued transforms read a **derived** (never declared) forward reach without being write clamps: the dimension-driven horizon-bounded MERGE (a *scan/recompute* bound on the enrichment recompute, not the write) and the horizon settled-delay/tail-rewrite mechanism, which remains partition-grain forward-reach machinery.

### Validator, not chooser

The machinery **validates** the declared `refresh:`/`grain:` shape against the derived properties and rejects (fail-loud) when the SQL cannot uphold the shape's contract. It **never chooses or silently switches** the shape. A full refresh is the honest fallback surfaced as a diagnostic, never an automatic downgrade. (Per-cell technique choice among proven-interchangeable techniques — §"Per-cell admission" — operates strictly inside this rule: it may change freshness, never observable bits at a fixed processed-input set.)

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
fold corner is contract-specific (it needs a combiner algebra — §"The algebraic maintenance
ladder"). Where the interchangeability conditions below hold, a recompute of a
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
exist (the `S`-indexed refinement of §"The equivalence invariant"; `S` is a **per-input
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

### Statement emission (single owner)

The physical statements a run executes for a cell — the region `DELETE`+`INSERT` pair, the keyed
fold `MERGE`, the column-scoped `MERGE`, the in-place `UPDATE`, the first-run `CREATE TABLE … AS`
— are produced by pure emitter functions in the maintenance layer (`smelt-logical`): the plan's
statement-level counterpart of "one derivation, many consumers". An emitter is a pure function
from plain data — target table, region literals, key columns, combiner-rendered set expressions,
the compiled/clamped SELECT body, a dialect tag — to an ordered statement group plus its
transactional requirement (a paired `DELETE`+`INSERT` is one transaction: a failed `INSERT` must
roll back its `DELETE`). Backends *execute* emitted statements (connections, transactions,
blocking dispatch) and never author maintenance-statement text of their own; dialect differences
(e.g. a `MERGE … UPDATE SET *` requiring a full-row source projection versus an explicit
column-list `SET`) live in the emitters as dialect-keyed variants, not in backend string
construction.

Two deliberate exclusions: the reconciliation ledger's DDL/DML (§"The reconciliation ledger") is
state bookkeeping owned per dialect by `smelt-state` — it is *interleaved* transactionally with
an emitted fold statement but is not itself a maintenance statement; and non-maintenance SQL
(introspection, seed loading, schema-evolution DDL) is outside this rule.

Single ownership is what makes maintenance SQL *observable*: the same emitters serve execution,
the conformance equivalence gates, and `smelt explain <model> --show-sql`, so printed SQL cannot
drift from executed SQL.

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

- The equivalence invariant, ladder, horizon, and validator-not-chooser are owned above
  (§Semantics); the plan's per-cell theorem is the `S`-vector refinement of the invariant, and
  per-cell choice operates strictly inside the validator-not-chooser rule.
- Output shape/grain declaration and the refresh trichotomy are owned by `models.md`; the plan
  validates against them. The **declaration law and litmus rule** (`models.md` §Design) — whether
  a fact is declared, derived, or implied, and whether a proposed combination earns a new peer
  shape — are likewise owned there; this spec consumes them.
- **Input-consumption** (`models.md` §"Input-consumption axis"): which input rows are new is a
  derived, cross-cutting axis (mutation-profile world-fact → input-delta-discovery proof in
  `model_properties.md` → re-scan/probe transform in `model_transforms.md`). Moving along it never
  changes the equivalence contract, only what is scanned. The **default** is windowed (clocked
  source → window-forward); full scan is the fallback for a clockless snapshot source — see
  §"Windowed maintenance and the horizon".
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

**One invariant, not two; addressing is the real axis.** An earlier cut split the contract into
"per-partition equivalence" (partition grain) and "end-state equivalence" (key grain), one per
output shape. That split was miscast: order/set-determinacy falls out of the single invariant for
*every* shape (the partition grain included), and per-partition equivalence is a *strengthening*
of that one invariant, not a peer of it. What actually distinguishes the shapes — and drives the
physical transform — is how the output is **addressed for update**: partition-addressed
(identity-free, whole-partition rewrite) versus key-addressed (identity-requiring `merge_into`,
writes reaching stored rows by key outside the input window). SCD2 is the proof that this is an
*addressing* distinction, not a source-clock one: its close-out write escapes the input
time-window intrinsically, so it can never be a per-partition rewrite regardless of whether its
source is clocked.

**Addressing stays binary; locality is a refinement, not a third pole.** Partition-addressed vs
key-addressed remains the load-bearing distinction (identity-free rewrite vs identity-requiring
merge). Key temporal locality does not change how output is addressed — the write is still a
keyed `merge_into` — it adds a proof about *where* addressed rows can live, licensing target
pruning, a time-partitioned keyed output, and per-slice equivalence. Promoting it to a third axis
pole was rejected: it would suggest a different write primitive and identity requirement where
there is none, and it would misplace a per-model derived/declared fact as a shape property
(`docs/research/20260705-keyed-time-superset.md`).

**Scope maps name the per-input dispatch.** Without the name, the run shape reads as a property
of the *model*, hiding that different inputs changing engage different targeted recomputes (a
fact delta folds forward; a dimension delta probes and horizon-merges; a definition diff
backfills columns; a self-edge forces ordering). Naming the dispatch makes "what runs when this
input changes" an explainable, per-input answer, and gives the per-input world-fact verdicts and
any future multi-clock driving-source work a stable home
(`docs/research/20260705-keyed-time-superset.md` §5).

**Windowed by default; full scan is the fallback.** Treating full-table recomputation as the
baseline and windowing as a per-shape optimisation inverts the real economics: a clocked model
can always be maintained over a bounded scan window, and only the absence of a clock forces a
wider read. Making windowing the default and full scan the *surfaced* fallback keeps the common
case scalable and pushes join optimisation to the engine over a safe widened scan, rather than
smelt hand-computing minimal deltas. Output addressing (partition vs key) is orthogonal to scan
windowing: a key-addressed model windows its scan yet writes back by key.

**The horizon is derived, not declared.** Trusting a declared horizon risks an under-estimate
that silently corrupts the clamp — dropping rows still within the model's reach. Deriving it from
the model's reach keeps clamps correct by construction; a declaration is admitted only as a
*ceiling* that warns when the derived value would exceed it. Because the derived clamp *is* the
model's SQL, a late arrival beyond the horizon is silently excluded rather than diagnosed —
surfacing lateness is a model-author + data-quality-check concern, not a maintenance guarantee
(§"Windowed maintenance and the horizon"). This can be softened later if a legitimate need to
widen beyond the derived reach appears, but the safe default is derive-for-correctness —
consistent with derive-else-declare (`models.md` §Design).

**Validator, never chooser.** Auto-selecting or silently downgrading the declared shape was
rejected: it reproduces dbt's `strategy:` footgun where the effective contract is invisible. The
declared shape is authoritative; the machinery only proves or refuses it.

**Placement is definitional, not consumer-counted.** A capability whose verdict is stateable
**without naming a shape profile** lives in a capability spec (`model_properties.md` /
`model_transforms.md`); a capability meaningful **only inside a profile** lives in that profile's
spec. (So pushdown-depth is a SQL property and lives in `model_properties.md`; backfill chunking,
meaningless outside partition-grain execution, stays in `batched_models.md`.) This gives every
capability exactly one home — what lets `smelt:validate` catch drift — without a mechanical
≥N-consumer rule; because these capabilities are broadly useful, building one before a second
consumer exists is fine. The invariant and ladder live *here* because every shape profile cites
them as its contract; keeping them inside one profile's spec would force the others to reach into
a sibling for their own contract. `keyed_models.md` remains the reference implementation of the
key-addressed maintenance path (retraction, reprocessing, presentation-purity) with its
column-family catalogue — see its §Surface and §Semantics for a worked composition-contract
example.

**Rejected alternatives**, briefly: a `strategy:` sub-knob (dbt's invisible-contract footgun); a
new `smelt-maintenance` crate (the derivation needs the tightest coupling to the sibling
classifiers; module boundary kept extraction-mechanical instead — `08-code-placement.md` §2.1);
qualifying the output clamp to a resolved inner alias (answers a question the output clamp must
never ask — `03-design-forks.md` F1); a third addressing pole for locality (it changes no write
primitive — §"Addressing stays binary" above); per-edge grain declarations (two declarations can
disagree; one per node cannot). Deeper rationale:
`docs/research/20260705-refresh-as-maintenance-plan/` (parts 01–10, with ratification records in
09 §1 and 10 §11).

## Constraints & Invariants

- The **equivalence invariant** holds for every non-`full` model and on every ladder rung; a
  transform that cannot preserve it for a given model is refused, never applied approximately.
  Order/set-determinacy is a corollary of it for **every** shape (the partition grain included);
  per-partition equivalence is a *strengthening* of it, not a separate contract.
- **Output addressing** is the load-bearing axis: partition-addressed shapes (identity-free)
  rewrite whole partitions; key-addressed shapes (identity-requiring) `merge_into` by key and may
  write outside the input time-window. This distinction is intrinsic to the shape (SCD2's
  retroactive close-out), independent of the source clock. Key temporal locality, where
  established, refines key-addressing with a derived slice bound — target-scan pruning and
  per-slice equivalence — without changing the addressing or the write primitive
  (`keyed_models.md` §"Key temporal locality").
- Maintenance is **windowed by default** where the model is clocked; a full scan is a surfaced
  fallback, never the silent baseline. Always `scan window ⊇ write window`.
- The **horizon is derived** from the model's reach; a declared horizon is a warning ceiling only
  and never relaxes the clamp. Because the derived clamp is the model's SQL, late arrivals beyond
  the horizon are silently excluded — surfacing them is a model-author + data-check concern, not
  a maintenance guarantee.
- **One home per capability and per rule.** The invariant, ladder, composition contract, and the
  plan are owned here; properties in `model_properties.md`, transforms in `model_transforms.md`,
  the declaration law and litmus rule in `models.md`. No spec re-specifies another's.
- **Proofs are fail-closed** (owned in `model_properties.md`, relied on here): an undecidable
  construct rejects; a declared escape hatch may only *widen* eligibility, never substitute for a
  proof's default reject.
- The declared **`refresh:` + `grain:` shape is the only declared strategy surface**;
  input-consumption is derived from the source, never declared per model. No `strategy:`
  sub-knob. The machinery **validates, never chooses** the shape; a fallback to full refresh is a
  surfaced diagnostic, never an automatic switch.
- **The plan is pure data, derived by pure functions, in one place** (`smelt-logical`);
  consumers — diagnostics, planner application, runtime lowering, the graph layer — never
  re-derive it. (Also recorded as an invariant in `architecture.md`.)
- **Maintenance statements have one author.** Every maintenance statement a run executes is the
  output of a pure emitter in the maintenance layer (§"Statement emission (single owner)");
  backends execute, never author. Printed (`--show-sql`), gate-verified, and executed SQL are the
  same emitters' output by construction.
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
- **Statement emission is not yet single-owner.** The emitters in
  `crates/smelt-logical/src/maintenance/emit.rs` are exercised only by the tracer/conformance
  tests; production runs author their maintenance SQL elsewhere — the region `DELETE`+`INSERT`
  inside the backends (`smelt-backend-duckdb`'s `delete_and_insert_transactional`,
  `smelt-backend-spark`'s `sql.rs`), the keyed fold `MERGE` in
  `smelt-runtime::cumulative::build_cumulative_merge_sql` (combiner-aware, unlike the
  additive-only `emit_keyed_fold`), the column-scoped `MERGE` in the backends' `merge_into`
  (DuckDB's `UPDATE SET *` full-row-projection shape, unlike `emit_column_scoped_merge`'s
  column-list shape), and the first-run `CREATE TABLE … AS` in
  `smelt-runtime::maintenance_driver`. Consequently the conformance suite's
  technique-equivalence legs (`crates/smelt-logical/tests/maintenance_plan_conformance.rs`)
  prove the *emitters* equivalent to full refresh, not the production statements, and
  `--show-sql` does not exist yet. No live plan cell lowers to `emit_in_place_update` today (the
  schema-evolution column backfill's `UPDATE … FROM` in `smelt-runtime::backfill` is a separate
  surface). Unification is tracked in `docs/plans/20260710-emit-unification.md`.
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
- **Delta detection for `--since-upstream` is explicit, not automatic, for v1.** The runner (or an
  external poller) supplies each source's landed delta directly on the command line
  (`--source <address> --landed <start>..<end>`, §CLI); the graph layer reflects exactly the
  supplied intervals through the edges. No persisted "last propagated through" watermark exists,
  and no invocation independently diffs a source's current coverage against a prior propagation to
  discover its own delta — a second `--since-upstream` call has no way to know what changed unless
  the caller tells it. This sidesteps `smelt_state::landed_deltas` (built for v1 as a byproduct of
  an ordinary model run — an append-only clocked source's landing is interval-diffed against prior
  coverage; a `mutable_snapshot` or unclocked source always resolves to the whole-table delta) and
  `change_feed` offset-based delta detection and snapshot diffing (not yet built), neither of which
  the graph layer consumes today. An automatic, watermark-diffed `--since-upstream` with no
  required flags is a possible future extension (§Future Extensions) once a persisted per-source
  watermark lands in `smelt-state`; the explicit form does not block on it.
- **Straddle attribution without locality** (a per-key footprint chaining across history) is
  scoped out of the ledger's v1: locality-or-explicit-footprint only (01 §8's own caveat).
- **The refresh-axis cut has landed.** `RefreshStrategy` (`crates/smelt-core/src/config.rs`)
  accepts only `full` / `incremental` / `materialized_view`; the removed strategy names
  (`batched`/`keyed`/`cumulative`/`versioned`) are hard errors with a fix-it pointing at
  `refresh: incremental` + the matching `grain:` (`models.md` §Known Divergences). A proposed
  `on_column_add: backfill | leave_null | recompute` policy knob is noted, not yet surface.
- **Windowed-by-default and the derived horizon are contract, partially built.** The stance
  (§"Windowed maintenance and the horizon") is normative. The per-source reach used to derive
  the horizon (`model_properties.md`'s `derive_model_bounds`) and the horizon *ceiling*
  declaration (`horizon_ceiling:`) with its compile-time warning are surfaced; a model-wide
  derived-horizon proof composing every source's reach into one number remains under
  construction, as does the model-author lateness-flag pattern's data-quality check. Tracked by
  `docs/plans/20260704-model-updates.md`.
- **Key temporal locality, the time-partitioned keyed output, and the scope-map explain surface
  are specified but unbuilt.** The locality gate and slice-pruned merge live in
  `keyed_models.md` (§Known Divergences there — refused unconditionally today via
  `KeyedForbidsTimeseries`); the per-input `smelt explain` scope-map rows are likewise unbuilt.
  Design derivation: `docs/research/20260705-keyed-time-superset.md`.
- **User docs describe the trichotomy + grain surface; the plan's own CLI surface is now partly
  covered.** The `docs-site/` pages consistently describe
  `refresh: full | incremental | materialized_view` and `grain: partition | key |
  key_per_partition`, seeded from the worked example catalogue
  (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`).
  `docs-site/docs/reference/cli.md` now documents `--since-upstream` (forward propagation) and
  `--include-upstreams` (backward resolution) under `smelt run`/`smelt build`. What is still not
  yet covered — because the underlying surface doesn't exist yet — is the `maintenance:`
  frontmatter block, `smelt explain`'s cell/clamp/ledger output, and `smelt bakeoff`.
- **A group merged across two mutable inputs has no group-merge-provenance policy.** Per-cell
  admission today checks obligations 4/5 (bounded reach/footprint) the same way regardless of
  whether a column group's `mutation_sensitivity` set came from ONE input or several — a
  partition-aligned multi-input merge (e.g. `orders.amount * fx_rates.rate`, both mutable,
  joined on the output's own partition column) is admitted as a targeted `ColumnScopedMerge`
  exactly like a single-input mutable dimension enrichment would be. A stricter
  "partition-local ≠ foldable" policy — forcing region recompute whenever a group's
  provenance spans more than one mutation-sensitive input, even when the read/write happen to
  be individually bounded — is undecided and unbuilt; pinned by
  `crates/smelt-logical/tests/maintenance_coverage_matrix.rs::ex12_multi_input_merge_degenerates_to_recompute`.
- **The trigger-list builder's `explicitly_mutable` scoping misses `change_feed`-declared
  sources entirely, not just clocked ones.** `derive_model_maintenance_plan`
  (`crates/smelt-db/src/queries/maintenance.rs`) only constructs an `UpstreamMutation` trigger
  for a source that is BOTH unclocked AND declares `mutation_profile: mutable_snapshot`
  literally — `change_feed` maps to the stricter `MutableSnapshot` posture for *admission*
  purposes (`source_facts`) but does not satisfy this literal-declaration check, so a
  `change_feed` source (clocked or not) never gets a mutation cell constructed at all, the
  same "no cell to even refuse" gap an append-only enrichment source has (the
  `Trigger::UpstreamMutation` scoping divergence recorded above). Pinned by
  `crates/smelt-cli/tests/property_discovery/coverage_matrix_gaps.rs::ex08_unclocked_change_feed_dimension_scan_unbounded`;
  when a source's own posture (not just its admission fallback) IS threaded through
  (`crates/smelt-logical/tests/maintenance_coverage_matrix.rs::ex14_change_feed_sum_recompute_only`,
  `::ex26_change_feed_latest_writer_recompute_only` construct this directly at the pure-
  derivation level), only full-input re-derivation is admitted — never an invertible-retraction
  or order-monotone-overwrite fold — because no live fold machinery consumes a change feed's
  delta shape yet.
- **`INTERSECT`/`EXCEPT` are unclassified set operations.** `model_properties.md` §Known
  Divergences already records that set-op distribution classifies `UNION ALL` only; this spec
  records the maintenance-plan-level consequence directly: an `INTERSECT`/`EXCEPT` composition
  falls through to the whole-model mutation-sensitivity collapse (same as any unrecognised
  shape), so every admitted cell is `DeleteInsert` region recompute regardless of source
  property — pinned by
  `crates/smelt-cli/tests/property_discovery/coverage_matrix_gaps.rs::ex41_ex42_intersect_no_payload_column_still_delete_insert`.
  A future set-op distribution proof covering `INTERSECT`/`EXCEPT` would need its own
  per-arm-cardinality reasoning (unlike `UNION ALL`'s multiset-union, an `INTERSECT`/`EXCEPT`
  row's presence in the output depends on BOTH arms simultaneously, so no single arm's delta
  alone determines a row's fate) before any targeted technique could ever be admitted for it.

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

- **Automatic, watermark-diffed `--since-upstream`.** Today `--since-upstream` requires the caller
  to supply each source's landed delta explicitly (`§CLI`, `§Known Divergences`). A future
  extension persists a per-source "last propagated through" watermark in `smelt-state` and diffs
  it against the source's current `covered_intervals` on every invocation, so a bare
  `--since-upstream` with no `--source`/`--landed` flags discovers its own delta. This still does
  not solve a raw, never-modeled source's freshness (no `covered_intervals` exists for something
  smelt has never landed) — that remains live backend source-freshness querying, out of scope here
  (no such capability exists in `smelt-backend*` today; sources declare posture in `sources.md`
  rather than being polled for it). The explicit-flag form and the automatic form are not
  exclusive: the automatic form would compute the same `--landed` intervals the explicit form
  takes directly, so it can layer on top without changing the graph layer or the CLI surface
  described in `§CLI`.

## References

- **Code**: `crates/smelt-logical/src/maintenance/{mod,derive,emit}.rs` (the per-cell derivation);
  `crates/smelt-logical/src/maintenance/propagate.rs` (the pure graph-layer composition math —
  `propagate`/`required_inputs`); `crates/smelt-runtime/src/propagation.rs` (the real per-workspace
  graph assembly, `smelt run --since-upstream` planning, and `smelt build --include-upstreams`
  planning — `build_forward_graph`, `plan_since_upstream`, `resolve_build_plan`, all consuming the
  same `Edge` list);
  `crates/smelt-logical/src/analysis/` (the classifiers admission consumes);
  `crates/smelt-runtime/src/{cumulative,maintenance_driver,dimension_horizon_merge,transformer,backfill}.rs`
  (today's technique executors and clamps); `crates/smelt-state/src/intervals.rs` (the degenerate
  ledger); `crates/smelt-backend/src/lib.rs` (technique primitives).
- **Tests**: `crates/smelt-logical/tests/{maintenance_tracer,maintenance_tracer_evolution,maintenance_tracer_propagation,maintenance_propagation_adjoint}.rs`
  (pure derivation-side and graph-composition-math assertions — the regression floor for chains,
  fan-out, diamonds, granularity mapping, and adjointness — `maintenance_propagation_adjoint.rs`
  is the dedicated home for the `forward(backward(P)) ⊇ P` law); `crates/smelt-runtime/tests/
  {tracer_maintenance,tracer_evolution,tracer_propagation,since_upstream_propagation}.rs` (the
  DuckDB equivalence oracles, and the real-workspace propagation-graph assembly tests);
  `crates/smelt-cli/tests/since_upstream.rs` (the CLI-wired forward-propagation suite, including
  the sufficiency-vs-full-refresh equivalence check); `crates/smelt-cli/tests/include_upstreams.rs`
  (the CLI-wired backward-resolution suite: resolved-slices-suffice-vs-full-refresh equivalence
  over a two-hop chain, and an unclocked-ancestor-resolves-to-whole-table case);
  `crates/smelt-maintenance-testkit` (dev-only, `publish = false`; the graduated Link-C in-process
  harness — the real-run-pipeline driver, the model-shape catalogue, the multiset-equality oracle,
  and the mutation-aware run-schedule generator/driver — wired as a dev-dependency of `smelt-cli`);
  `cargo test -p smelt-cli --test property_discovery` is the standing equivalence gate: it runs the
  Link-C schedule suite over the full model-shape catalogue against a real DuckDB backend on every
  `cargo test`, asserting emitted maintenance output equals a full refresh over adversarial
  append/lateness/mutation schedules. The per-cell probe modules under
  `crates/smelt-cli/tests/property_discovery/` that consume the testkit crate remain disposable
  research probes (see `.claude/scripts/property-experimental-gate.sh`); only the shared harness
  graduated. `cargo test -p smelt-logical --test maintenance_plan_conformance ::
  coverage_matrix_is_inhabited` is the standing inventory gate over the research example
  catalogue's coverage matrix (`docs/research/20260705-refresh-as-maintenance-plan/
  07-example-catalogue.md` §"Coverage matrix", plus one `INTERSECT`/`EXCEPT` row this gate adds):
  it encodes the matrix as data and asserts every inhabited `(construct × source-property)` cell
  is accounted for by exactly one of two explicit, disjoint lists — `CLAIMED` (a grounded,
  executable test proves the cell's HOLDS-or-refuses verdict; see
  `crates/smelt-logical/tests/maintenance_coverage_matrix.rs` and
  `crates/smelt-cli/tests/property_discovery/coverage_matrix_gaps.rs` for the cells this phase
  lifted) or `KNOWN_GAPS` (named, not silently omitted). Adding a matrix cell without a matching
  `CLAIMED`/`KNOWN_GAPS` entry fails the test, by construction (additive-only). `CLAIMED` currently
  lifts 9 catalogue ids (EX-02, EX-08, EX-12, EX-14, EX-18, EX-24, EX-26, EX-27, EX-35, plus the
  added EX-41/EX-42 row); the remainder of the matrix's ~100 inhabited cells are named individually
  in `KNOWN_GAPS` (most as "plausibly covered by an existing `G-*`/`SC-*` property-discovery probe,
  not re-verified against this exact catalogue id" — cross-referencing those probes to catalogue
  ids by name is itself unbuilt; a few, like EX-25's LAG/LEAD footprint reflection and EX-29's
  as-of-run-contract gating, need production investigation not yet done). Both lists are per-cell,
  never per-row, so a future change can lift one cell at a time without re-deriving the whole
  inventory.
- **User docs**: `docs-site/docs/index.md`, `docs-site/docs/guide/{incremental-models,sql-models,materializations}.md`,
  `docs-site/docs/concepts/how-it-works.md`, `docs-site/docs/reference/{timeseries,smelt-yml,cumulative-aggregate,cli}.md`
  describe the trichotomy + grain surface; `docs-site/docs/reference/cli.md` also documents
  `--since-upstream` and `--include-upstreams`. The plan itself (the `maintenance:` block,
  `smelt explain`'s cell/clamp/ledger output) is not yet user-documented (see Known Divergences).
- **Plans (history)**: `docs/plans/20260704-model-updates.md`,
  `docs/plans/20260704-model-updates-fundamentals.md` (the L1+L2 substrate),
  `docs/plans/20260705-property-discovery-loop.md` (the empirical engine).
- **Related specs**: `model_properties.md`, `model_transforms.md`,
  `models.md`, `sources.md`, `batched_models.md`, `keyed_models.md`, `versioned_models.md`,
  `materialized_view.md`, `multi_backend.md`, `schema_evolution.md`, `timeseries.md`,
  `diagnostics.md`, `architecture.md`, `cli.md`.
