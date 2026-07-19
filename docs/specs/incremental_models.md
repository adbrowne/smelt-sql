---
feature: incremental_models
status: experimental
last_reviewed: 2026-07-19
owners: [andrew]
---

# Incremental Models

> **What this is.** The single normative spec for **maintained models** — the contract, the derived plan, the graph built on it, and the declared shapes of `refresh: incremental`. It owns, in order: (a) the **maintenance contract** every maintained (non-`full`) model upholds — the equivalence invariant, the algebraic ladder, the windowed-scan/horizon contract, validator-not-chooser, and the composition contract; (b) the **maintenance plan** — the derived, per-model object that says how each part of a model's output is kept current under each kind of change, a matrix indexed by `(output-column-group × trigger)` whose cells choose maintenance techniques; (c) the **graph layer** built on the plan — given what landed upstream, which partitions of which downstream models must run (forward propagation), and given a requested output period, which upstream slices must exist (backward resolution); and (d) the **shape profiles** of `refresh: incremental` — the partition grain (`grain: partition`), the key grain (`grain: key`), and the key grain's interval-versioning sub-declaration (`versioning: interval`, SCD2).
>
> **One feature, two declared shape facts — the shapes are not competing modes.** A modeller declares `refresh: incremental` plus the output's **shape-defining facts** — its clock (`timeseries:`) and/or identity (`unique_key:`) — and everything else (the `grain` label, technique, **physical write addressing**, clamps, windows, ledgers, propagation edges) is *derived* per `(column-group × trigger × changed-input)` cell. The shapes share one invariant, one plan machinery, and one graph layer; they differ only in the declared facts and each shape's local machinery, all specified here. Physical write addressing — whether a cell rewrites a region or merges by key — is **not** a model-wide verdict: a model can be region-addressed with respect to its main fact table yet keyed-addressed when a *different* input changes (§"Per-cell write addressing"). This spec supersedes and retires four earlier specs (`maintenance_plan.md`, `batched_models.md`, `keyed_models.md`, `versioned_models.md`) whose file-per-shape cut misread as mutually exclusive options.
>
> Out of scope, with their own homes: the properties a model's SQL can be proven to have (`model_properties.md`); the physical transform mechanisms themselves (`model_transforms.md`); the `refresh:` axis, the declaration law, and the litmus rule (`models.md`); source world-fact declarations (`sources.md`); the time-dimension declaration `event_time_column`/`partition_column`/`granularity` (`timeseries.md`); engine-owned maintenance (`materialized_view.md` — the one shape profile that stays a separate spec, because its maintainer is the engine, not smelt); backend capability flags (`multi_backend.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).
>
> **Status: experimental.** The partition grain's DuckDB DELETE+INSERT path is implemented and tested; `refresh: incremental` + `grain: partition` is the live surface (`refresh: batched` is a hard error with a fix-it; the `batched:` sub-block still parses as the profile's local options block — §Known Divergences). The key grain's additive-fold and extremal/lattice-fold families are implemented against the windowed-keyed-maintenance driver; the overwrite, once-write, and plain-overwrite families, the snapshot-reconcile run shape, and the time-partitioned (key temporal locality) output are specified ahead of implementation, and the transactional merge ledger is built on DuckDB only (§Known Divergences). `versioning: interval` is specified ahead of implementation and does not parse today (§Known Divergences).

## Surface

### The declared shape axis

The entire declared shape surface of an incremental model is the two shape-defining facts of the Relation Contract (`models.md` §"The Relation Contract") — its **clock** and its **identity** — plus the optional interval-versioning sub-declaration:

```yaml
refresh: incremental        # the one refresh mode this spec covers
timeseries: { ... }         # the clock: a time axis (event_time_column / partition_column / granularity)
unique_key: [ ... ]         # the identity: makes the output key-addressable (incl. whether partition_column is a member)
versioning: interval        # optional; requires identity + no model clock — keep every version with a validity interval (SCD2)
grain: partition | key | key_per_partition   # optional CHECK-ONLY assertion; drives nothing (§"Grain is a derived label")
```

The four corners these two facts inhabit — and the derived `grain` label each earns — are the shape profiles this spec details:

- **clock, no identity** → a **partition-addressed table** (derived `grain: partition`), one row per `(partition_column, …)`: a complete table whose *default* cell addressing is region rewrite (DELETE+INSERT). §"Partition-grain declaration" and §"The partition grain".
- **identity, no clock** → **bare keyed state** (derived `grain: key`), one row per `unique_key`, read in full by consumers, kept current by folding deltas into the stored state (keyed `merge_into`). One profile covers the running-aggregate, latest-value, and milestone patterns; what distinguishes them is the **column family** of each projection, derived from the SQL, never declared. §"Key-grain declaration" and §"The key grain".
- **clock + identity, `partition_column` ∉ key** → **keyed state with a home slice** (derived `grain: key`, time-partitioned): one row per key, each key's partition value a fixed per-key constant; admitted iff key temporal locality is established (§"Key temporal locality").
- **clock + identity, `partition_column` ∈ key** → the **trajectory** (derived `grain: key_per_partition`): one row per `(key, partition)`; the natural key recurs across partitions.
- the **identity-no-clock** corner **+ `versioning: interval`** → keyed state **plus history**: every version of a key kept, each stamped with a non-overlapping validity interval (SCD2). Requires identity and **no model clock** — the close-out escapes every time window, so a `timeseries:` block on the model is a hard error. Deliberately not a shape of its own: row addressing is still by key; the interval is structure within the key. §"Interval-versioned declaration" and §"Interval versioning".

The `refresh:` axis itself (including `full`, `materialized_view`) and the declaration law are owned by `models.md` §"Refresh axis"; this spec covers the `incremental` shapes above. The declarations name **shape-defining facts only** — which technique realizes which part of the output, and how it **physically addresses rows**, are properties of `(column-group × trigger × changed-input)` cells (§"The plan (derived, reported)", §"Per-cell write addressing"), never of the model as a whole, and the machinery validates the declared facts rather than choosing them (§"Validator, not chooser").

#### Grain is a derived label (+ optional check-only assertion)

`grain` is **not declared as a driver**. It is a derived classification computed from `(clock?, identity?, partition_column ∈ key?)` — the corners above — reported by `smelt explain`, and computed for sources too (a source also has an effective grain: clocked-fact, keyed-dimension, …). A modeller who wants the friendly name in frontmatter may write it only as a **check-only assertion** (like `maintenance.scan_bounds`): it errors on mismatch with the derived facts (`models.md` §"Constraint violations") and *drives nothing*. This keeps a shared, human-readable shape name that can never disagree with the facts: the declared *facts* stay one per node, and only the *derived* label and addressing vary, so two declarations of one node can never conflict. The single fact `partition_column ∈ unique_key` distinguishes a trajectory (`key_per_partition`) from a keyed lookup whose key has a fixed home slice — which is why key temporal locality's **route 1** ("`partition_column` is a `unique_key` column") *is* the partition-∈-key case and **route 2** ("partition is a per-key constant, functionally dependent on the key") *is* the partition-∉-key case (§"Key temporal locality").

#### The two axes are orthogonal — "partitioned or keyed" is a category error

The two shape-defining facts are **orthogonal**: whether the output declares an **identity**
(`unique_key:` — making it key-addressable) and whether it declares a **clock** (`timeseries:` — a
time axis consumers can window over) vary independently. Physical write *addressing* (region
rewrite vs keyed merge) is a separate, per-cell derived fact (§"Per-cell write addressing"), not a
declared property of the shape. The inhabited combinations (with their derived `grain` label):

| | declares a clock (time axis) | no clock |
|---|---|---|
| **no identity** | derived `grain: partition` — a complete clocked table | — (a keyless, clockless table has no maintainable shape) |
| **declares identity** | derived `grain: key` (time-partitioned, locality-admitted, `partition_column` ∉ key) or `grain: key_per_partition` (`partition_column` ∈ key) | derived `grain: key` bare — a lookup, read in full by consumers |

A model with **both** a key and a partition axis is a first-class shape, not a corner case, and
several capabilities exist **only** in that composed form
(§"What the composed shape uniquely enables"). Both axes are also orthogonal to
**input consumption**: a bare keyed model over a
clocked source still consumes window-forward; a composed model's *output* clock is a property of
its own stored shape, not of its sources.

Any text — in this spec, a sibling spec, research, or a plan — that frames "partitioned" and
"keyed" as mutually exclusive alternatives, or reasons about "the partitioned models" versus
"the keyed models" as disjoint populations, is wrong and must be corrected against this section.
The pre-consolidation file split ("batched" vs "keyed" specs) manufactured exactly this error;
this section exists so it cannot creep back.

### The composition contract

This section is **system surface**: its callers are the shape profiles — the shape sections of this spec (§"The partition grain (`grain: partition`)", §"The key grain (`grain: key`)", §"Interval versioning (`versioning: interval`)") and `materialized_view.md` — and the planner/analysis layer, not the modeller directly.

A maintained model is a **composition** of three kinds of thing:

- **Properties** — what a model's SQL can be proven (or declared) to be: the monotonicity trace, the algebraic discriminants, partition alignment, and the rest (`model_properties.md`).
- **Transforms** — the physical mechanisms a property licenses: keyed `merge_into`, source-filter pushdown, partition DELETE+INSERT, and the rest (`model_transforms.md`).
- **Output shape** — declared via `grain:` (`models.md` §"Refresh axis"): partition-addressed (a complete table with a `partition_column`) or key-addressed (one row per key; optionally time-partitioned under key temporal locality — §"Key temporal locality (the time-partitioned output)").
- **Scope maps** — the per-input dispatch: for each input of a model, the derived mapping from that input's delta to the affected output addresses and the transform that runs for it. The driving source's delta engages the windowed fold; a mutable dimension's delta engages the delta-driven probe + dimension-driven horizon-bounded MERGE; a self-edge engages ordered execution; a model-definition diff engages the targeted column backfill (all `model_transforms.md`). Which map applies follows from input-delta discovery (`model_properties.md`) and the input's declared world-facts (`sources.md`); a run is the union of its inputs' scope maps — "what runs when *this* input changes" is a first-class, per-input answer, surfaced by `smelt explain`.

Every shape profile — the shape sections of this spec and `materialized_view.md` — must present a **composition table** stating, for that profile: the properties it requires, the world-facts it consumes, the transforms it drives — differentiated per input class where they differ (the profile's scope maps) — and its output shape. A profile's normative content is exactly (a) that composition table, referencing shared capabilities **by name**, plus (b) the profile's own **local** machinery, defined in full. It must not re-specify a capability that a capability spec (or a shared section of this spec) owns.

### The plan (derived, reported)

Every non-`full` model has a **maintenance plan**: a set of **cells**, each keyed by
`(output-column-group × trigger × changed-input)` and carrying:

- the **corner** of the read-scope × write-scope 2×2 the cell occupies (§Semantics), where
  write-scope is the cell's **physical write addressing** — `{targeted addresses, region-overwrite}`;
- the **technique** that realizes it (`DELETE`+`INSERT` region recompute, keyed fold `MERGE`,
  column-scoped `MERGE`, in-place `UPDATE`, …) drawn from the open write-pattern registry
  (§"Per-cell write addressing");
- the **write mechanism** admitted for the cell — derived by the available-addressings rule, or a
  validated user `write:` pin (§"Per-cell write addressing");
- the **derived scan clamps** — per read source, the `(partition_col, before, after)` window the
  cell reads, anchored to the output region;
- the **partition-locality verdict** per source (§Semantics);
- the cell's **obligations** and any **traded guarantees** (per-column, two-dimensional:
  equivalence contract × settle bound).

The plan is **derived, never declared**. What stays declared is the model's **shape-defining facts**
— its clock and identity (`models.md`) — validated against the plan, an error on mismatch, never a
silent flip; the `grain` label is derived from those facts. `smelt explain` prints the plan: every
cell, its addressing, clamps and locality verdicts, the per-column guarantee ledger, and — at the
graph level — the model's inbound edges.

### Triggers

Four trigger classes index the plan's columns:

- **creation** — new rows arrived in the driving source;
- **mutation** — a post-creation delta in a source some column group is mutation-sensitive to;
- **definition change** — the model gained output fields while sources stood still;
- **backfill** — an explicit region recompute from replayable input.

Each trigger is paired with the **changed-input** it fires for — the specific source (or self-edge,
or definition diff) whose delta drives the cell; this is the third axis of the plan's cell key. The
same column group under the same trigger class can derive *different* physical write addressing for
*different* changed inputs (§"Per-cell write addressing"): a creation delta on the driving fact
rewrites/folds a region, while a mutation delta on a dimension merges by key. The scope maps
(§"The composition contract") are the per-changed-input dispatch this axis names.

### Upstream model edges

A maintained model's ref to **another maintained model in the same project** is a plan edge of
the same standing as a `sources.*` ref. The upstream model's own `timeseries:` declaration —
already validated by that model's plan — supplies the event-time clock the downstream
creation-trigger cell is clamped by, and scan bounds compose through the chain exactly as the
propagation graph composes them (§"The graph layer"). Deriving the cell requires nothing the
plan does not already hold: the upstream's clock column, granularity, and the downstream's scan
reach over that ref. An upstream-model ref whose clock cannot be derived (the upstream declares
no `timeseries:` and none is inferable) is a **recorded refusal** on that cell
(`MaintenanceReachNotDerivable`, naming the edge) — never a silent drop. A ref to a `full`-mode
or view upstream derives no creation cell (there is no incremental delta to receive); it
participates in mutation/backfill triggers only.

For forward propagation, `--source <address>` accepts either a declared source or an upstream
maintained model; a model's landed delta is the output window a completed run wrote for it.

### Frontmatter

```yaml
maintenance:
  defaults:
    prefer: recompute | fold | auto        # per-model soft default (auto = cost model)
  cells:
    - columns: [<col>, ...]                # names any member of a derived column group
      on: <source-address> | backfill      # the trigger + changed-input this cell handles
      prefer: fold | recompute             # soft per-cell bias (cost model still refines)
      technique: fold | recompute | rederive_columns   # hard per-cell technique pin (bypasses cost model)
      write: <pattern>                     # hard per-cell addressing pin (optional); OPEN name resolved
                                           #   against the write-pattern registry (e.g. region | keyed |
                                           #   column | update); unknown or backend-unavailable → refused
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
- `cells[].write` is a **hard per-cell addressing pin**: an **open name** resolved against the
  write-pattern registry, not a sealed keyword set (§"Per-cell write addressing" → "The
  write-pattern set is open"). Every pin is **validated against the equivalence invariant** for its
  cell — an addressing that cannot uphold equivalence is refused with a diagnostic, never silently
  honoured — and an unrecognised name, or one naming a pattern the target backend cannot execute, is
  refused fail-loud (never silently downgraded). This keeps the whole feature inside *validator, not
  chooser*.
- `cells[].columns` naming columns that span two derived groups is an error (it would silently
  re-partition the plan).
- `scan_bounds` is **check-only**: it never modifies a clamp; it only refuses (or warns) when the
  derived plan exceeds the stated expectation. A project-level default in `smelt.yml` sets the
  baseline; per-model blocks refine it.
- A sibling **top-level** frontmatter key, `horizon_ceiling: '<interval>'` (partition grain
  only), declares a ceiling on the derived horizon — a compile-time warning threshold, never a
  clamp modification (§"Windowed maintenance and the horizon").

### Partition-grain declaration (`grain: partition`)

The shape profile for `refresh: incremental` + `grain: partition`: a partition-addressed table,
one row per `(partition_column, …)`, kept current by the derived per-cell maintenance plan rather
than a declared strategy. The shape's default plan corner is recompute-a-region per touched
partition, driven by DELETE+INSERT — not a mode the modeller selects: which technique realizes
which part of the output is a property of `(column-group × trigger)` cells, never of the model as
a whole. (Historical name: "batched".)

#### Partition-grain composition

Per the composition contract (§"The composition contract"), the partition-grain profile is composed as:

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: partition` — a complete table with a monotone `partition_column`, addressed by partition, not by key | `models.md` §"Refresh axis" |
| **Properties (required)** | event-time monotonicity trace; column nullability gate; unified bound/reach derivation; frame-reach taxonomy; injection-point / pushdown-depth; partition alignment (scoped); driving-fact / anchor resolution; determinism (run vs row) + nondeterminism predicate + taint; body-structure classifier; set-operation distribution; static-seed detection; window-independence / ordered-execution | `model_properties.md` |
| **World-facts (consumed)** | the timeseries clock (`event_time_column`/`partition_column`/`granularity`); source mutation profile and lateness margin (`sources.md`); the column-scoped equivalence contract (`columns.<c>.contract`) | `timeseries.md`, `sources.md`, `models.md` §"`columns:` — column metadata" |
| **Default plan (recompute corner)** | source-filter pushdown; partition DELETE+INSERT; output-window derivation (partition-column skew inversion); outer output-clamp; two-layer widened-scan + exact output clamp; compile-time pinning | `model_transforms.md` |
| **Admission** | every check below is one instance of §"Per-cell admission" evaluated for the recompute-a-region corner over a partition-grain output (§"Safety checks (per-cell admission for the partition grain's recompute corner)") | this spec, §"Per-cell admission" |
| **Invariant upheld** | per-partition equivalence (the partition-grain specialisation of the framework's processed-input equivalence invariant, and of the plan's `S`-vector refinement) | §"The equivalence invariant", §"Per-cell admission" |

The normative content of this spec is that table plus the profile's **local** machinery defined below: the batch-safety roll-up, column-locality of the equivalence, event-time outer-visibility, backfill chunking, run/partition granularity alignment, and the partition-grain surface (`grain: partition`, `timeseries:` requirement, `safety_overrides`, per-source-clamp observability).

#### Partition-grain frontmatter (in `.sql` files)

```sql
---
refresh: incremental
grain: partition              # optional CHECK-ONLY assertion; derived from timeseries + no unique_key
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
safety_overrides:             # optional; bypass specific safety checks
  allow_window_functions: false
  allow_having: false
  allow_subqueries: false
columns:
  inserted_at:
    contract: plausible       # optional; exempts this output column from the determinism requirement
---

SELECT order_date, customer_id, SUM(amount) AS total
FROM smelt.orders
GROUP BY order_date, customer_id
```

`refresh: incremental` plus a `timeseries:` clock and **no declared identity** is the opt-in for the partition shape; the stored `table` is implied. A written `grain: partition` is the **check-only assertion** of the shape the facts already fix (§"Grain is a derived label"). `safety_overrides` is a top-level frontmatter key (`models.md` §"YAML frontmatter keys") admitted only on a partition-shaped output. Declaring a `unique_key` here does **not** add a "dedup aid" to the partition shape — it declares identity, which reshapes the output to the composed clock-and-identity keyed corner (`models.md` §"Refresh axis"), where keyed dimension-change addressing lives (§"Per-cell write addressing", §"What the composed shape uniquely enables"); `safety_overrides` is then a hard error. A model that wants only whole-partition rewrites declares no identity.

The partition shape is fixed by the `timeseries:` block (`timeseries.md`); missing the block on a model asserting `grain: partition` is a hard error (`models.md` §"Constraint violations"). The declared `partition_column` must be **monotone** — validated by the event-time monotonicity trace (`model_properties.md` §"Event-time monotonicity trace"). Monotone admits either a timestamp *or* an ever-increasing integer (a sequence id / offset / watermark column): the trace recognises a constant shift over such a column (`batch_id + 5`, `batch_id - 5`) on the same footing as a constant `INTERVAL` shift over a timestamp column, while a non-monotone integer transform (`batch_id % n`, `batch_id * n`) is rejected fail-closed, naming the construct.

An output column's equivalence contract is the per-column `columns.<c>.contract` declaration (`models.md` §"`columns:` — column metadata", semantics owned by this spec): `contract: plausible` exempts that column from the determinism requirement (audit stamps and surrogates the modeller accepts may vary) exactly where the pre-cut `nondeterministic_columns` list did. Listing `event_time_column`, `partition_column`, or a `unique_key` column as `plausible` is a configuration error (a skeleton position must be deterministic — `models.md` §"Constraint violations").

#### Partition-grain `smelt.yml` overrides

Frontmatter wins over `smelt.yml` when both set the same field.

```yaml
models:
  daily_revenue:
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: order_date
      partition_column: order_date
      granularity: day
```

#### Granularity values

See `timeseries.md` §"Granularity values" for the closed enum. The profile consumes the granularity declared in the model's `timeseries:` block.

#### Strategy enum (backend-internal)

Strategy is **not** declared on the model — it is derived per cell (§"The plan matrix"). For the recompute corner the partition grain's default plan drives, backends pick a physical strategy from the model's config and their capabilities:

```rust
enum IncrementalStrategy {
    DeleteInsert,    // DELETE matching partitions + INSERT
    Append,          // insert-only; no dedup
    InsertOverwrite, // replace entire partitions atomically
}
```

DuckDB currently always uses `DeleteInsert`. A partition-shaped output's *creation/backfill* cells are region-addressed (DELETE+INSERT); UPSERT (`MERGE`) is not the addressing of those cells and a pure partition grain (no declared identity) has no keyed addressing at all. Keyed `MERGE` is the addressing a *dimension-change* cell derives on a **composed clock-and-identity** output (one that declares a `unique_key` — derived `grain: key`, time-partitioned; §"Per-cell write addressing") — the keyed `merge_into` transform (`model_transforms.md`), scoped to the touched partitions — so `MERGE` is per-cell, driven by what changed, not tied to one grain.

### Key-grain declaration (`grain: key`)

The shape profile for `refresh: incremental` + `grain: key`: the stored table is keyed state —
one row per `unique_key` — kept current by the derived per-cell maintenance plan rather than a
declared strategy. One profile covers the running-aggregate, latest-value, and
milestone/retroactive-enrichment patterns; what distinguishes those patterns is the **column
family** of each projection, derived from the SQL, never declared. (Historical names: "keyed",
"cumulative".)

#### Key-grain composition

Per the composition contract (§"The composition contract"), the key-grain profile is composed as:

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: key` — the end-state per key, addressed by `unique_key`, not by partition | `models.md` §"Refresh axis" |
| **Properties (required)** | algebraic discriminants (is-monoid / needs-inverse / decomposable / value-vs-order-monotone) — they define the column families below; driving-fact / anchor resolution (the single clocked source under window-forward); event-time monotonicity trace (the driving source's clock); once-write provenance (the `COALESCE` family's licence); join-contribution monotonicity (enrichment joins); input-delta discovery; **key temporal locality** for a time-partitioned output (key-grain-local, §Semantics) | `model_properties.md` |
| **World-facts (consumed)** | the **timeseries clock** of a clocked driving source (`timeseries.md`); the **source mutation profile** (`sources.md`); a declared **key-recurrence bound** (`sources.md`) where the recurrence-bounded locality route is declared rather than derived (§Semantics) | `timeseries.md`, `sources.md` |
| **Default plan (fold-a-delta corner)** | keyed **`merge_into`** (target-as-replica) sequenced by the **windowed-keyed-maintenance driver**, with **source-filter pushdown** on the driving source; the **transactional merge ledger**; for enrichment shapes, the **dimension-driven horizon-bounded MERGE**; the **slice-pruned merge target** under established key temporal locality (§Semantics) | `model_transforms.md` |
| **Admission** | every check below is one instance of §"Per-cell admission" evaluated for the fold-a-delta corner over a key-grain output (§"Admission matrix (column family × source shape)") | this spec, §"Per-cell admission" |
| **Invariant upheld** | end-state equivalence — the end-state specialisation of the processed-input equivalence invariant, and of the plan's `S`-vector refinement; the oracle is the model's **own SQL** (§Semantics) | §"The equivalence invariant", §"Per-cell admission" |

The normative content of this spec is that table plus the profile's **local** machinery defined below: the column-family catalogue, the derived execution postures, the transactional merge ledger, the two run shapes, the key-temporal-locality routes for the time-partitioned output, and the key-grain surface (`grain: key`, `timeseries:` admission, the classifier).

#### Key-grain frontmatter (in `.sql` files)

```sql
---
refresh: incremental
unique_key: [order_id]
grain: key                    # optional CHECK-ONLY assertion; derived from unique_key + no clock
---

SELECT
    order_id,
    MIN(event_ts)                 AS placed_at,       -- extremal fold
    MAX_BY(status, event_ts)      AS current_status,  -- order-monotone overwrite
    SUM(item_count)               AS total_items,     -- additive fold
    MAX(shipped_ts)               AS shipped_at       -- extremal fold (milestone)
FROM smelt.order_events
GROUP BY order_id
```

`refresh: incremental` plus a declared `unique_key` (with no clock, or a clock admitted under key temporal locality) is the opt-in for the key shape; the stored `table` is implied (`models.md` §Design — the modeller does not restate `materialization: table`). A written `grain: key` is the **check-only assertion** of the shape the identity fact already fixes (§"Grain is a derived label"). `unique_key` is the identity that makes the output key-addressed, and must restate the `GROUP BY` column list — the classifier checks the two agree (§"The column-family catalogue"). No rule-specific config block is read or required, and `safety_overrides` — admitted only on a partition-shaped output (`models.md` §"Constraint violations") — is a hard error once identity makes the output key-addressed.

By default the output carries no partition column (§"Key-grain output shape"). A model **may** declare a `timeseries:` block to time-partition its keyed output — admitted **iff key temporal locality is established** (§Semantics "Key temporal locality"), refused otherwise (`KeyedForbidsTimeseries`, naming the missing route). Output partitioning is independent of event-time-aware *consumption*: a key-grain model over a source that carries a `timeseries:` declaration consumes that source window-forward whether or not its own output declares a clock (§Semantics). `grain: key_per_partition` (`models.md` §"Refresh axis") is a **different grain**, not a sub-declaration of this one — it stores the per-partition trajectory, not the end-state this profile maintains.

The time-partitioned form, on the flagship shape it exists for (event-grain dedupe over a bounded redelivery window; the driving source declares `key_recurrence` — `sources.md`):

```sql
---
refresh: incremental
unique_key: [event_id]
grain: key                    # optional CHECK-ONLY assertion; derived from unique_key + locality-admitted clock
timeseries:
  event_time_column: first_seen_at
  partition_column: first_seen_date
  granularity: day
---

SELECT
    event_id,
    MIN(event_ts)              AS first_seen_at,    -- extremal fold (the output clock)
    MIN(event_date)            AS first_seen_date,  -- extremal fold (the partition column)
    MAX_BY(payload, event_ts)  AS payload           -- order-monotone overwrite (latest copy wins)
FROM smelt.raw_events
GROUP BY event_id
```

The body **must** be an aggregated `GROUP BY` query: `unique_key` is the `GROUP BY` column list, and every non-key projection must classify into exactly one column family (below). A bare, un-aggregated projection is not a key-grain model — the SQL must itself express the per-key semantics, so that a full refresh of the SQL is the profile's correctness oracle (§Design "The SQL is the oracle").

#### Key-grain `smelt.yml` overrides

```yaml
models:
  order_lifecycle:
    refresh: incremental
    grain: key
    unique_key: [order_id]
```

Frontmatter wins over `smelt.yml` when both set the same field. The same `timeseries:`-admission constraint applies.

#### The column-family catalogue

The classifier assigns each non-key projection to exactly one **column family**. The family fixes the cross-window combiner (a lookup off the aggregator — authors never declare combiners) and every derived property:

| Family | Per-key aggregators | Cross-window combiner | Idempotent (re-run safe) | Order-independent | Invertible | Run shapes admitted | Extra licence |
|---|---|---|---|---|---|---|---|
| **additive fold** | `COUNT(...)`, `SUM(...)`, `BIT_XOR(...)` | `+` / `xor` | no | yes | yes | window-forward only | ledger-enforced re-run refusal (§Semantics) |
| **extremal / lattice fold** | `MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR` | `LEAST`/`GREATEST`/`AND`/`OR`/`&`/`\|` | yes | yes | no | window-forward only | — |
| **order-monotone overwrite** | `MAX_BY(value, ordering)`, `MIN_BY(value, ordering)` | max/min-by-ordering (§"Ordering ties") | yes | up to ordering-key ties | no | window-forward only | — |
| **once-write** | `COALESCE`-first-non-null over the group | `COALESCE(target, delta)` | yes | yes (given the proof) | no | window-forward only | once-write provenance proof (`model_properties.md`): key-derived, or a declared functional dependency |
| **plain overwrite** | `ANY_VALUE(...)` | incoming row wins | yes | n/a — one row per key per scan | no | **snapshot-reconcile only** | — |

Any other aggregate, any non-aggregate non-key expression, and any composite expression over aggregates (`SUM(x) + 1`) is rejected (`KeyedUnknownCombiner`). Add columns for the underlying aggregates and derive downstream.

The pattern functions `smelt.latest(value, ordering)` (→ `MAX_BY`), `smelt.once(value)` (→ the once-write canonical spelling), and `smelt.current(value)` (→ `ANY_VALUE`) are the intent-naming sugar for the overwrite, once-write, and plain-overwrite families; they are ordinary transparent functions (`functions.md`) whose expansions are admitted on exactly the same terms as hand-written calls.

### Interval-versioned declaration (`versioning: interval`)

The shape profile for `refresh: incremental` + a declared identity (`unique_key:`) + `versioning: interval`, with no model clock (derived `grain: key`, SCD2): the
stored table is keyed state **plus history** — every version of a key is kept, each stamped with
a non-overlapping validity interval. It is deliberately not a third grain: row addressing is
still by key; the interval is structure within the key. (Historical name: "versioned".)

#### Interval-versioning composition

Per the composition contract (§"The composition contract"), the SCD2 profile is composed as:

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | declared identity (`unique_key:`) + `versioning: interval`, no model clock (derived `grain: key`) — the sub-declaration that keeps every version with a validity interval instead of only the current row | `models.md` §"Refresh axis" |
| **Properties (required)** | algebraic monotonicity / ordering discriminants (value-monotone vs order-monotone, for tracked-attribute change detection); **event-time monotonicity trace** (validity is stamped from source event-time, never the run clock); **driving-fact / anchor resolution** (the single clocked source under window-forward); **window-independence / ordered-execution** (the combiner reads versions in event order) | `model_properties.md` |
| **World-facts (consumed)** | the **timeseries clock** of an update-events / CDC feed (`timeseries.md`), *or* a mutable snapshot's **source mutation profile** (`sources.md`) — one of the two, derived from the source's shape, never declared on the model | `timeseries.md`, `sources.md` |
| **Default plan (fold-a-delta corner)** | keyed **`merge_into`** sequenced by the **windowed-keyed-maintenance driver**, with **source-filter pushdown** on the driving source, folding through the **close-old / open-new interval maintenance** combiner (profile-local, below) | `model_transforms.md` |
| **Admission** | every check below is one instance of §"Per-cell admission" evaluated for the fold-a-delta corner over a key-grain-plus-interval output (§"Interval-versioning admission") | this spec, §"Per-cell admission" |
| **Invariant upheld** | end-state equivalence in its **interval-keyed specialisation** — the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same processed snapshots, independent of merge order (§"End-state equivalence (interval-keyed)") | §"The equivalence invariant", §"Per-cell admission" |

The normative content of this spec is that table plus the profile's **local** machinery defined below: the close-old / open-new combiner, the smelt-managed validity columns, tracked-attribute selection, event-time stamping, and deletion handling.

#### Interval-versioning frontmatter (in `.sql` files)

```sql
---
refresh: incremental
unique_key: [customer_id]
versioning: interval
grain: key                    # optional CHECK-ONLY assertion; derived from unique_key + no clock
---

SELECT
    customer_id,          -- the natural key
    tier,
    region
FROM smelt.customers_snapshot
```

`versioning: interval` is admitted only where the output declares **identity** (a `unique_key:`, the key-shaped corners — `models.md` §"Constraint violations") and is a hard error together with a `timeseries:` block on the model itself — the SCD2 close-out escapes every time window (`models.md` §"Constraint violations"). This forbids output partitioning, not event-time-aware *consumption*: like the plain key grain, a `versioning: interval` model over a source that carries a `timeseries:` declaration (an update-events / CDC feed) consumes that source window-forward (see §"Input consumption").

The model's SELECT projects the **natural key** and the tracked attribute columns as they are *now*. smelt maintains the version history: each `smelt build` compares incoming rows against the stored current version per key and, where a tracked attribute changed, closes the prior version and opens a new one.

#### Interval-versioned output shape

Keyed **plus** a validity interval. The stored table carries the projected columns and the smelt-managed validity columns — a `valid_from` / `valid_to` interval and an `is_current` flag (exact names/types are an Open Question). A key with three successive states yields three rows: two closed intervals and one open (`valid_to` NULL / sentinel, `is_current = true`).

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

#### Partition-grain run flags

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

- Both flags are required for any direct (`--event-time`-driven) partition-grain run; a forward-propagation run (`--since-upstream`) instead derives its regions from the supplied `--landed` intervals (see the shared CLI list above). Format: ISO-8601 (`2026-03-20`, `2026-03-20T00:00:00Z`).
- The end bound is exclusive: `--event-time-end 2026-03-25` does not include `2026-03-25`.
- The supplied `[start, end)` range is the **run window**. It must be a positive integer multiple of `timeseries.granularity` aligned to granularity boundaries (`timeseries.md` §"Granularity arithmetic"). Run-window size may exceed partition granularity (Semantics §"Run window vs partition granularity").
- `backbuild` uses the model's classified batch safety (Semantics §"Batch safety classification") to expand or split the requested range.

#### Key-grain run flags

Which flags apply is determined by the model's derived **run shape** (§Semantics):

```
smelt run       --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # window-forward
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # window-forward
smelt run       [selectors]                                                             # snapshot-reconcile
```

- **Window-forward** (the model's driving source is clocked): both flags are required; they apply to the **driving source's** `partition_column` / `granularity` — not to any column on the keyed output, including an admitted output `timeseries:` block (run flags always address the source's clock). Format and alignment rules follow §"Partition-grain run flags".
- **Snapshot-reconcile** (no clocked source): the flags are a **hard error** — *"model has no clocked driving source; run without event-time flags"*. Each run is a whole reconciliation.

### Diagnostics

All codes are catalogued in `diagnostics.md`; this spec owns their semantics. Partition-grain
rejections surface as `TimeseriesRequiredForBatched` (missing `timeseries:` block — the rule is
`models.md` §"Constraint violations"), `BatchedNotSafe` (the batch-safety classifier,
§"Batch safety classification"), and `EventTimeColumnNotVisibleAtOuterSelect`
(§"Event-time outer-visibility (partition-grain-local)"); the first two code names retain the
retired mode spellings (§Known Divergences → "The partition grain").

#### The `Maintenance*` family

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
- `MaintenanceWriteAddressingRefused` — a `maintenance.cells[].write` pin names an addressing that
  cannot uphold the cell's equivalence invariant (e.g. keyed on an output with no identity, or a
  region write on a cell whose footprint escapes any partition set). Names the cell and the refused
  pattern (§"Per-cell write addressing").
- `MaintenanceWritePatternUnavailable` — a `write:` pin names an unrecognised pattern, or one the
  target backend's capability registry does not provide. Names the pattern and the backend; never a
  silent downgrade (§"Per-cell write addressing" → "The write-pattern set is open").

#### Key-grain diagnostic codes

| Code | Severity | Trigger |
|---|---|---|
| `KeyedRequiresGroupBy` | Error | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `KeyedForbidsTimeseries` | Error | The model declares a `timeseries:` block but key temporal locality cannot be established — no route applies (§Semantics "Key temporal locality"; the routes require the window-forward run shape). The message names the three routes and the nearest missing fact. |
| `KeyedUnknownCombiner` | Error | A non-key projection is not a direct call to a catalogued aggregator. Names the offending expression; when the projection is a bare column or `ANY_VALUE` under window-forward, the message names `MAX_BY` + an ordering column as the fix. |
| `KeyedGroupByContainsPartitionColumn` | Error | The `GROUP BY` contains the driving source's `partition_column` and the model declares **no** `timeseries:` block — ambiguous between the partition-grain shape and the key-embedded time-partitioned key-grain shape. The diagnostic suggests both fixes: `grain: partition` + `timeseries:`, or declaring `timeseries:` on the model to stay `grain: key`. |
| `KeyedForbidsWindowFunctions` | Error | The outer SELECT body uses `OVER (...)`. The keyed state *is* the window. |
| `KeyedForbidsNondeterministic` | Error | The SQL uses `NOW()`, `RANDOM()`, or other non-deterministic functions. Cross-window merge requires deterministic per-window output. |
| `KeyedSqlNotParseable` | Error | The model body cannot be parsed into the shape the classifier reads. |
| `KeyedMultipleDrivingSources` | Error | More than one timeseries-tagged source appears in the FROM clause. Lists the candidates. |
| `KeyedOnceWriteUnproven` | Error | A once-write (`COALESCE`) column has no once-write provenance proof — the value is not provably a per-key constant. Names the column; suggests the key-derived form, a declared functional dependency, or remodelling. |
| `KeyedRetractableContribution` | Error | An enrichment join's per-key contribution is retractable — it feeds a decrementing aggregate or a value that must be un-seen. Steers to `refresh: materialized_view` or DAG composition. Does **not** fire on the join spelling alone (§Semantics). |
| `KeyedSnapshotSourceUnsupportedColumn` | Error | A column family inadmissible under snapshot-reconcile (§"Admission matrix") appears in a model with no clocked driving source. Names the column, the family, and why the current-snapshot oracle cannot hold for it. |
| `KeyedReprocessedWindow` | Error | A run window covers a ledgered window of a non-re-run-tolerant model, or `--auto` detects changed input under an already-merged window (§"Reprocessing"). Points at `--full-refresh`. |
| `KeyedRecurrenceBoundViolated` | Error | Runtime, window-forward, declared-recurrence route only: a merged delta row matched (or would duplicate) a stored key outside the run's derived slice — the driving source's declared `key_recurrence` is violated. The run's transaction rolls back; the message reports the violation count and sample keys. Derived locality routes cannot fire it. |

`safety_overrides:` is a partition-grain-only key (`models.md` §"Constraint violations") and is a hard error on `grain: key`. Every rejection above guards the equivalence invariant itself, not a partial-correctness optimisation — there is nothing safe to waive (§Design).

## Semantics

### The equivalence invariant

This is the parent contract of the whole family. Every maintained (non-`full`) model upholds **one** invariant, stated over an abstract **processed-input set**: **an incremental run produces the result a full refresh would, restricted to the inputs it has processed so far.** Formally, for the processed input set `S`, `incremental_state(S) == full_refresh(source | input ∈ S)`. `S` is a set of *source rows/partitions the run has scanned*, not necessarily a clock-addressed partition set — the **partition-set form** (`source | partition_col ∈ S`, the form used throughout the rest of this spec and the profile specs) is the **clocked specialisation** of this invariant, available whenever the driving source carries a `timeseries:` clock; an unclocked (snapshot) source has no partition set to slice by, and its specialisation is stated per shape profile (e.g. §"End-state equivalence: the SQL is the oracle" states it over "keys present in the current snapshot").

**Order/set-determinacy is a corollary, and it holds for every shape profile — the partition grain included.** The right-hand side depends only on the *set* `S`, never the order it was processed, so any conforming profile is order-independent. This is not special to the key-addressed shapes: a partition-grain model's partitions are disjoint, so its combiner is disjoint union (a commutative monoid) and the property is trivial — but it is present.

The shape profiles differ not in *which* equivalence they satisfy but in **how their writes address rows** — the axis that actually drives the physical transform and the identity requirement. Addressing is a per-cell fact (§"Per-cell write addressing"); each profile below names the addressing of its *dominant* (creation/default) cell, and a model may derive the other addressing for a different `(trigger × changed-input)` cell (a composed clock-and-identity output's dimension-change cell is keyed while its fact-creation cell region-rewrites the touched partitions):

- **Partition-addressed** (identity-free — the partition shape's default cell): output is addressed by `partition_column`; a source partition maps to an output partition rewritten wholesale (DELETE+INSERT), no row identity needed. Here equivalence is additionally checkable slice-by-slice — *per-partition equivalence* — a **strengthening** of the one invariant, available because each output slice depends only on its own source partition (partition-local).
- **Key-addressed** (identity-requiring — the key shape's default cell, `versioning: interval`, `refresh: materialized_view`): output is addressed by a key; each processed input contributes a delta merged into the keyed state (`merge_into`). The write reaches stored rows **by key, wherever they live** — it is *not* bounded by the incoming data's time window. The interval-versioned profile (SCD2, `versioning: interval`) is the sharp case: admitting a new value for a key requires closing the previously-open version, a row whose timestamp lies arbitrarily far outside the current input window — which is exactly why a key-addressed *write* cannot be maintained as a per-partition rewrite. Equivalence is checked on the end-state.

Key-addressing admits a **derived refinement**: a key-addressed output that also carries a `timeseries:` partition column, admitted when **key temporal locality** is established — every stored row a run's deltas can touch provably lies within a derived slice of the output's time axis (§"Key temporal locality"). The write is still a keyed `merge_into`; locality licenses pruning the merge's *target scan* to the slice, and makes **per-slice equivalence** — the keyed analogue of per-partition equivalence — available as the same kind of strengthening. SCD2's close-out is why this is a per-model *established fact*, not a key-grain default: some key-addressed writes intrinsically escape every time window.

So per-partition equivalence is not a peer of some separate "end-state equivalence" — it is a strengthening of the single invariant that partition-addressed, partition-local output enjoys. The key-addressed shapes discharge the *same* invariant on their end-state because their writes are keyed rather than partition-local. Every property is proven in service of this invariant; every transform is licensed **because it preserves it**. For the smelt-driven shapes the invariant is discharged by the generative equivalence oracle (§References), the family's regression net; for `materialized_view` it is discharged by the **engine's** native IVM, not the smelt oracle (smelt runs no combiner for that shape — §"Validator, not chooser").

**The replayability split.** Full equivalence — an executable `full_refresh` oracle a test can actually run — holds only for **replayable inputs**: a set `S` the model can re-evaluate its own SQL over (a clocked source's processed partitions; a snapshot's keys currently present). v1 admits **only** combinations whose oracle is executable this way (this is exactly what §"Admission matrix" enforces per column). The designed-but-unshipped **third column** for the combinations that are not admitted — a non-replayable input under a partitioned output, or a fold family that would need to have observed history it cannot replay — is an **observer / prefix-consistency contract**: a different, weaker equivalence (a property of the *observation sequence*, not a re-runnable full refresh) that a future opt-in could state and admit explicitly, rather than being smuggled in under the executable-oracle invariant this spec states. It is not specified here; each shape profile's Known Divergences records where it would apply.

**Two named carve-outs.** Every admitted keyed model's executable oracle carries exactly two carve-outs, both **named consequences of the executable-oracle requirement, not gaps in it**:

- **Retained departed keys** under an unclocked (snapshot-reconcile) posture: a key present in the stored state but absent from the current snapshot is retained, not deleted, so the stored table is *the oracle's rows plus retained departed keys* — a documented divergence from a hypothetical delete-on-absence oracle (§"The two run shapes (derived, never declared)", §"End-state equivalence: the SQL is the oracle").
- **Ordering-key ties** on an order-monotone overwrite column (`MAX_BY`/`MIN_BY`): equivalence holds up to ties on the ordering expression, because the classifier cannot statically prove ordering-key uniqueness (§"Ordering ties").

The interval-versioned profile's oracle is its end-state equivalence in the **interval-keyed specialisation** — the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same processed snapshots, independent of merge order (§"End-state equivalence (interval-keyed)").

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

**The key grain has no write-eligibility clamp.** Unlike the partition grain, a `grain: key` run merges **every** delta row it scans, into whatever key it names, however old that key is — there is no bound on which keys a run may touch (§"No write-eligibility clamp"). A **derived forward reach** is still computed and reported (via `smelt explain`) for observability, but it never gates admission and never bounds a write. This is a deliberate difference from the partition-grain horizon above, not an oversight: the keyed write is proportional to delta size regardless of how far back the touched keys live, so a write clamp buys nothing for correctness and would silently drop scanned inputs — the one thing the equivalence invariant forbids. What a keyed clamp would buy (settled-key GC, a bounded working set) is deferred optimisation that, if ever introduced, must ship together with late-fact accounting (`docs/research/20260705-keyed-collapse-application.md` D6). The narrow principle beneath both stances: **only proofs prune; a declared bound is admitted only checked (fail-loud on violation); no unproven bound ever refuses a write.** Target-scan slice pruning under established key temporal locality (§"Key temporal locality") conforms — the derived routes prune by proof, the declared key-recurrence route prunes only under a transactional runtime check, and every scanned delta row still merges.

**Three pruning categories, one principle.** The only-proofs-prune rule admits exactly three
categories of narrowing, with sharp boundaries:

1. **Target-scan slice pruning** (read-side) — rows the write provably cannot touch are removed
   from the merge's *read* of stored state; licensed by the key-temporal-locality proofs or the
   transactionally-checked recurrence declaration (§"Key temporal locality").
2. **No-op write elimination** (write-side) — a maintenance write may be skipped **iff** the
   row's applied effect is proven to be the identity, proven per row *by evaluation*: an exact
   `IS DISTINCT FROM` comparison over every column that can differ under the cell's trigger (the
   mutation-sensitive group — comparing only it is sound *because* the other groups are proven
   insensitive). Suppression may never skip **evaluating** a scanned input — restricting what is
   *computed* is a separate concern with its own static licence (§Future Extensions,
   "Conditional maintenance without a change feed"). A compared column must be a pure function
   of the processed inputs; a column that legitimately varies run to run (`contract: plausible`,
   run-pinned `NOW()`) is incomparable, and a cell containing one refuses the conditional
   technique (fail-closed). At a fixed processed-input set `S` the suppressed and unconditional
   variants produce identical state — interchangeable in the strongest sense of §"Per-cell
   admission", so choosing between them is squarely a cost-model/`prefer`/`technique` matter.
   `model_transforms.md` catalogues the two physical realisations this category licenses:
   change-suppressed MERGE (a matched-arm `IS DISTINCT FROM` predicate on the keyed `merge_into`
   or column-scoped merge, dialect-split on the unmatched-by-source side) and the staged-candidate
   conditional DELETE+INSERT (the merge-less realisation, for a backend without `MERGE`) — both
   licensed by region row identity plus per-column change comparability on the compared group.
3. **Write-eligibility clamps** — forbidden on the key grain, derived-only on the partition
   grain (the horizon above).

Categories 1–2 preserve the equivalence invariant bit-for-bit at fixed `S`; category 3 is
different in kind — it bounds which inputs are *in* `S` at all. Naming the middle category
explicitly keeps the boundary sharp: a suppressed write is the write-side dual of slice pruning
(the proof is the per-row equality just evaluated), not a clamp, and must never be argued into
one. Two `model_transforms.md`-catalogued transforms read a **derived** (never declared) forward reach without being write clamps: the dimension-driven horizon-bounded MERGE (a *scan/recompute* bound on the enrichment recompute, not the write) and the horizon settled-delay/tail-rewrite mechanism, which remains partition-grain forward-reach machinery.

### Validator, not chooser

The machinery **validates** the declared shape — the `refresh:` value plus the shape-defining facts (clock `timeseries:`, identity `unique_key:`), and any check-only `grain:`/`write:` assertion — against the derived properties, and rejects (fail-loud) when the SQL cannot uphold the shape's contract. It **never chooses or silently switches** the shape or the addressing. A full refresh is the honest fallback surfaced as a diagnostic, never an automatic downgrade. (Per-cell technique choice among proven-interchangeable techniques — §"Per-cell admission" — operates strictly inside this rule: it may change freshness, never observable bits at a fixed processed-input set.)

### The plan matrix

The plan factors the output columns into **column groups** by shared mutation-sensitivity
(`model_properties.md` §"Per-column mutation-sensitivity / column provenance" — the proof and its
degenerate-collapse rule are defined there; this spec consumes the resulting groups as the plan's
column axis). Creation is shared by every column (all columns of a new row are computed
together); mutation is what partitions them.

Each `(group × trigger × changed-input)` cell picks a corner of the 2×2 of **read scope**
(delta+state vs the region's full upstream input) × **write scope** — the cell's **physical write
addressing** (targeted addresses vs region overwrite):

|              | write: targeted (keyed addressing) | write: region-overwrite (partition addressing) |
|---|---|---|
| **read: delta+state** | fold-a-delta | read-modify-write region |
| **read: full-input** | column-scoped re-derivation | recompute-a-region |

The write-scope column *is* the addressing corner, and which concrete write pattern realizes it —
keyed `MERGE`, column-scoped `MERGE`, in-place `UPDATE`, region `DELETE`+`INSERT`, or a
backend-provided variant — is drawn from the open write-pattern registry by the available-addressings
rule (§"Per-cell write addressing"). Recompute-a-region is contract-agnostic and unconditionally
valid over replayable input; the
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

### Per-cell write addressing

Every `(column-group × trigger × changed-input)` cell derives its **physical write** — how the cell
locates the stored rows it updates — from the **currently known** write-pattern set, an **open
registry**, not a closed enum:

```
{ region DELETE+INSERT, keyed MERGE, column-scoped MERGE, in-place UPDATE, full rebuild, … }
```

**The available-addressings rule.** A write mechanism is admitted for a cell iff:

> `available = (which contract facts the output declares) × (what the trigger/changed-input needs) × (the equivalence invariant) × (backend capability)`

The first three factors are structural; the fourth is the target engine's capability registry
(`architecture.md`). What each declared fact gates:

- keyed `MERGE` / column-scoped `MERGE` / in-place `UPDATE` require a declared `unique_key` (row
  identity);
- region `DELETE`+`INSERT` requires a declared partition axis (`timeseries:`) to delete by;
- a **bare lookup** (identity, no clock) has no region → only keyed merge or full rebuild;
- a **bare partition table** (clock, no identity) has no identity → only region rewrite or full
  rebuild. To gain keyed dimension-change addressing the output must **declare a `unique_key`**,
  which makes it the composed clock-and-identity keyed shape (derived `grain: key`, time-partitioned;
  §"What the composed shape uniquely enables") — so declaring identity is **load-bearing** (it admits
  keyed writes), never a dedup footnote;
- SCD2's close-out cell (§"Interval versioning") has **only** keyed `MERGE` available, because its
  write provably escapes any time window — derived per-cell, fail-loud if the facts can't support
  it, no bespoke shape needed.

A cell with no admissible write mechanism is `MaintenanceNoAdmissibleTechnique`, naming the cell.

**Addressing mechanism vs execution span — a keyed write on a clocked model is still
partition-scoped.** Choosing keyed `MERGE` (or column-scoped `MERGE` / in-place `UPDATE`) for a cell
picks how a row is *found* — by identity — not that the statement runs unbounded over the whole
table. When the output also declares a `timeseries:` axis, the write stays **bounded to (and, where
the backend benefits, iterated over) the affected partitions**: the changed-input delta is first
resolved to the set of touched partitions, and the keyed `MERGE` is emitted per-partition (or with a
partition predicate) against just those. So "dimension change → keyed merge" on a partition-clocked
model is a keyed merge *scoped to the partitions the correction lands in*, not a table-wide scan.
A genuinely window-free keyed write — one whole-table `MERGE` — is the exception, reached only when
the cell **provably cannot** be bounded to a partition set (the SCD2 close-out, whose affected rows
escape any time window); that unboundedness is itself a derived per-cell fact, fail-loud
(`MaintenanceUnboundedFootprint` / `MaintenanceScanUnbounded`), never a default. Partition-scoping
is orthogonal to the addressing corner: region and keyed writes alike ride the same
partition-pruning the plan already computes (§"Partition-local maintenance").

**User pins.** The override ladder (§Surface "Frontmatter") names the write mechanism per cell via
`maintenance.cells[].write`. A pin is **validated against the equivalence invariant** for its cell
(§"Per-cell admission", "Interchangeability and choice") — an addressing that cannot uphold
equivalence is **refused with a diagnostic** (`MaintenanceWriteAddressingRefused`), never silently
honoured — and a name the target backend cannot execute is refused too
(`MaintenanceWritePatternUnavailable`). The pin selects among *admissible* mechanisms; it never
widens the admissible set.

**The two scenarios resolved.**

- **Mixed addressing by which input changed.** The output declares **both** `timeseries:` and
  `unique_key:`. The creation-trigger cell (main fact delta) derives a region rewrite (or fold, per the plan matrix); the
  dimension-change cell derives a keyed column-scoped `MERGE` — available *because* `unique_key` is
  declared — still **scoped to the partitions the correction touches** (per the note above), not a
  whole-table merge. Pin either if the cost model picks wrong.
- **Mixed addressing by trigger.** The output declares `timeseries:` (± `unique_key`). Creation /
  mutation cells derive keyed merge / fold; the `backfill` cell is pinned
  `on: backfill, technique: recompute, write: region` → `DELETE`+`INSERT` (a clean region reset).
  Licensed by the fixed-`S` interchangeability rule (a recompute supersedes and resets what folds
  wrote — §"Per-cell admission").

#### The write-pattern set is open (and partly backend-provided)

The patterns above are the ones understood *today*. The set will grow — partition/atomic swap
(Delta/Iceberg `REPLACE PARTITION`), copy-on-write vs merge-on-read variants, `MERGE … WHEN NOT
MATCHED BY SOURCE` prune, staged-upsert, incremental MV refresh, and backend-specific primitives not
yet met. The design's durable contract is therefore deliberately **not** the enumeration; the
enumeration is data. Three ramifications, each a reason the reframe is *more* robust to growth:

- **The invariant is the admission function, not the enum.** The stable, load-bearing thing is the
  available-addressings rule and the *validate, never choose blind* doctrine. A new pattern is
  admitted purely by declaring **which contract facts it requires** (identity? a partition axis?
  ordered arrival?) and **discharging the equivalence proof obligation**
  (`incremental_state(S) == full_refresh(inputs ∈ S)` for the cells it serves — §"The equivalence
  invariant"). Nothing else moves: grain stays derived, the contract stays the vocabulary, the cost
  model ranks whatever candidates the rule admits. Extensibility is *safe by construction* because
  the equivalence gate is the price of entry — a new mechanism can never be less correct than the
  ones it joins.
- **The pattern set is backend-relative — the fourth admission factor.** Engines differ sharply on
  atomic partition swap, true `UPDATE`, and merge-on-read, so admission carries **backend
  capability** as a fourth factor: the write layer queries the backend's **capability registry**
  (`architecture.md`), and a pattern the target cannot execute is simply not a candidate. This makes
  the registry the natural home for backend-specific optimisations to be *contributed* rather than
  special-cased in the planner, and keeps a portable project from silently depending on a primitive
  only one engine has.
- **The `write:` pin is an open, fail-loud vocabulary.** Because pins name patterns and patterns are
  extensible, `write:` is **not** a sealed `region|keyed|column|update` enum — it is an **open name
  resolved against the registry**. An unrecognised pin, or one naming a pattern the target backend
  cannot provide, is **refused with a diagnostic** (fail-loud discipline), never silently downgraded
  to a default. The surface admits new pattern names the moment a backend registers them, and rejects
  everything it cannot honour.

Net: the enum is a **snapshot of a registry**; the admission rule + equivalence gate + capability
factor are the **contract**. A new write pattern plugs in without reopening the shape/contract
framing and carries its own correctness proof — so growth costs a registry entry, not a redesign.
Backends **execute** registered patterns; they never **author** maintenance-statement text
(§"Statement emission (single owner)"; `architecture.md` §"Constraints & Invariants" maintenance-plan
purity).

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

Three deliberate exclusions: the reconciliation ledger's DDL/DML (§"The reconciliation ledger") is
state bookkeeping owned per dialect by `smelt-state` — it is *interleaved* transactionally with
an emitted fold statement but is not itself a maintenance statement; the observed-output-delta
record (§"The graph layer" — "Observed deltas on model edges") and the fingerprint sidecar's own
storage (table DDL, digest-refresh upsert, GC delete — `sources.md` §"The fingerprint sidecar")
sit in the same excluded class, warehouse-resident and owned per dialect by `smelt-state` alongside
the reconciliation ledger, each interleaved transactionally with the write whose changed-row
set or digest it captures but never itself a maintenance statement — the fingerprint sidecar's diff
query is the one exception inside that same feature: unlike its own table's storage DDL/DML, the
diff is a derived maintenance-relevant comparison (which source keys count as "changed"), so it IS
emitter-authored (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`), not part of
this exclusion; and non-maintenance SQL (introspection, seed loading,
schema-evolution DDL) is outside this rule.

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

**Observed deltas on model edges.** A model edge's propagated delta follows the same landed-delta
refinement as a source edge (`sources.md` §"Landed-delta (derived, recorded)"): where a run
recorded an **observed output delta** — the changed-row set a conditional write (§"Windowed
maintenance and the horizon", category 2) actually touched, restricted to comparable columns
(the per-column change-comparability proof, `model_properties.md`) — that changed-row set,
projected onto the model's own partition axis, is the edge's delta; absent a recorded delta the
edge falls back to the run's written window, the coarser and always-correct form
(**widen-never-narrow**, same rule as the source hierarchy). The record is warehouse-resident,
alongside the reconciliation ledger (§"The reconciliation ledger"), and is written in the **same
backend transaction as the write it records** — a delta visible without its write, or a write
without its delta, breaks propagation soundness (a downstream consumer would schedule against a
delta that does not correspond to any committed state). **Trust boundary:** an observed delta is
trusted because the state is smelt-owned, written only by smelt's own conditional-write execution
path — bookkeeping alongside the write, per §"Statement emission (single owner)"'s third
exclusion, not an emitter-authored maintenance statement — mirroring the trust rule sources.md
applies to declared world-facts; there is no out-of-band-edit
tripwire in v1 — an external mutation to the target table between runs is not detected. This is an
explicit Open Question (§Known Divergences), not a silently-assumed absence. Empty and absent are
distinct: an empty recorded delta means the run executed and changed nothing (a real,
propagatable fact); an absent record means no delta was ever recorded for that write, and a
consumer must not conflate the two. This composes with the derived settle bound exactly as named
in §"What the composed shape uniquely enables" ("Settle-bound × observed-delta composition"): once
both legs are built, a stable upstream chain degenerates to empty-delta no-op propagation with a
provable horizon behind it.

**Refusals.** The graph refuses fail-loud (`MaintenanceGraphUnsupportedNode`) on: a cyclic edge
set; a **self-referential** model (a table-graph cycle that is a DAG only when time-unrolled —
admissible in principle iff its self-clamp is strictly time-backward, with forward dirt running
to the frontier and backward resolution reaching the model's basis/checkpoint); a **keyed-grain
node without an admitted time axis** (no partition axis for interval dirt — silently treating it
as day-axis would be wrong-and-quiet). A **locality-admitted time-partitioned keyed output is
not refused**: it is a clocked node whose edges use its declared granularity, and whose outbound
dirt is the key→partition projection of what its runs changed — exact under locality routes 1–2,
widened backward by `r` plus margins under route 3
(§"What the composed shape uniquely enables").

### The partition grain (`grain: partition`)

The partition-addressed shape: a complete table with a monotone `partition_column`, kept current by the recompute-a-region corner (partition DELETE+INSERT). Its declared surface is §"Partition-grain declaration (`grain: partition`)"; the machinery below is partition-grain-**local**.

#### Execution model (DuckDB, current)

For a partition-grain run with run window `[start, end)`, the recompute corner drives four transforms from `model_transforms.md`:

1. **Partition DELETE** from the output table where `partition_column` falls in the **derived output window** — the run window pushed through the model's declared partition-column relation (`model_transforms.md` §"The output window is derived, never assumed"): identity when the `partition_column` tracks event time, so `output window = run window`; skew-inverted when the `partition_column` is *derived* and skews away from the driving date column, declared by a Form B relation. For such a **write-rebasing model** (e.g. a session keyed by `session_start_date` gaining events the next day, `before = after = 1 day`) the output window for run `[D, D+1)` is `[D−1, D+2)`, so the DELETE covers **every** partition the INSERT will write — including the prior-day partition the new data reaches. Deleting only the run window would strand the skew-reached partition stale forever: no later run's window contains it.
2. **Outer output-clamp** — inject `WHERE partition_column >= out_start AND partition_column < out_end` at the outermost SELECT, constraining the model's *output* to the same derived output window the DELETE covers. This step is **dropped for the transparent slice** (exactly one timeseries source, zero-margin bound `Bounded(_, 0, 0)`, no partition-column skew): the per-source pushdown filter already *is* the output clamp, so a second textually identical outer `WHERE` is redundant (Injection-point / pushdown-depth property; `model_transforms.md` §"Source-filter pushdown + the two clamps"). A genuine lookback margin, a partition-column skew, or more than one timeseries source keeps the outer clamp: scan window and output window are then distinct, load-bearing windows. Each written partition's **scan** is sized from the derived output window's reach, never the run window's — rewriting a skew-reached neighbour partition from a scan sized for the run window would under-read that partition's own reach.
3. **Source-filter pushdown** — inject a per-source `partition_column` filter on each `smelt.<path>` reference, derived from the model's SQL. Sources without a `timeseries:` declaration are lookups: no bound, read in full.
4. **INSERT** the resulting query's output into the output table.

DELETE range and output clamp are derived from **one** window so the contract stays idempotent for any write-window width. Re-running the same `[start, end)` under fixed input converges to the same final state (Constraint: idempotence). Per-partition equivalence holds (Semantics §"Per-partition equivalence").

The derived output window is a range to be **covered**, not a mandate for one statement. Backfill chunking (§"First-run and backfill") splits it into sequential DELETE+INSERT pairs the same way it splits a wide run window — the production pattern of running a multi-day update as several bounded sequential queries rather than one large one, each chunk's scan sized from that chunk's own reach (`model_transforms.md` §Design "Derived output window composes with chunking").

#### Run window vs partition granularity

The CLI `[--event-time-start, --event-time-end)` declares a **run window**, not a per-partition invocation. It must be a positive integer multiple of `timeseries.granularity` aligned to granularity boundaries (`timeseries.md` §"Granularity arithmetic"); within that, run-window size and partition-granularity unit are independent. A daily-partitioned model run with a 30-day window is **one** engine query (sources filtered to the union of the run window and each source's pushdown bound; output clamped to the run window) and **one** partition-aligned DELETE over the 30 partitions followed by one INSERT. Backfilling 60 days is one `smelt run --event-time-start D --event-time-end D+60d`, not 60 daily invocations. Per-partition equivalence holds regardless of run-window size.

The declared `timeseries.granularity` (`g_run`, since it governs run-window alignment) must be at least as coarse as the granularity actually implied by the `partition_column` projection's truncation/grid transform (`g_part`) — derived independently from the model's SQL, not merely trusted from the declaration. A model whose `partition_column` is `DATE_TRUNC('day', event_time)` has `g_part = day`; declaring `granularity: hour` on that model is rejected, because an hourly run window does not correspond to the model's real (daily) partitions and would misalign the DELETE+INSERT contract. `g_run >= g_part` is checked under the closed enum's increasing-coarseness ordering (`hour < day < week < month < quarter < year`, `timeseries.md` §"Granularity values"); `g_run == g_part` or `g_run` coarser than `g_part` both pass. When `g_part` cannot be derived (an opaque projection the classifier has no rule for), this comparison is skipped — undecided, not a positive disproof — and only the declared-granularity alignment check applies. This is enforced with hard validation: a sub-`g_part` run window is rejected with a message naming the minimum window, never silently widened or coarsened to fit (Known Divergences).

#### Batch safety classification

The optimizer rolls the per-source bound map (Property: *unified bound/reach derivation*, `BoundResult` per source) into a single **partition-grain-local** class per model. This roll-up is meaningful only inside the recompute-a-region execution shape and is owned here:

| Class               | Meaning                                                                 | Execution                                                |
|---------------------|-------------------------------------------------------------------------|----------------------------------------------------------|
| `FullyBatchSafe`    | All timeseries sources `Bounded(_, 0, 0)`; no temporal dependencies     | Single query for any run window                          |
| `BoundedSafe(n)`    | All timeseries sources `Bounded`, with `n = max(before + after)` > 0    | Auto-sized chunks (3× context, clamped 7–90 partitions)  |
| `PerPartitionOnly`  | One or more timeseries sources `Unbounded` (cumulative-across-history)  | One partition at a time, sequential                      |

`n` for `BoundedSafe` is rendered in the source's partition-column unit and is the same value the source-filter pushdown transform reads.

A model with **any** `NotDerivable` source is **refused at planning time**, not assigned a class — the optimizer cannot prove the partition-DELETE+INSERT contract is safe (`MaintenanceReachNotDerivable`, §"Per-cell admission" obligation 4). The diagnostic names the offending construct and the source-map points at the original SQL. The author rewrites into a derivable form or removes the dependency. There is **no silent downgrade to full-refresh** (§"Validator, not chooser").

**Wide single-batch builds.** When `FullyBatchSafe` causes a single-batch build spanning more than 30 partition periods, smelt warns and recommends `--per-partition` or `--batch-size <n>`. The warning is informational; both `--per-partition` and `--batch-size` suppress it (the user has opted into a safe batching shape).

#### First-run and backfill

A first run (no output table) and a backfill (re-run of a written range) follow the same DELETE+INSERT contract — the DELETE is a no-op when the partition is absent. The planner picks a **backfill-chunking** shape (a partition-grain-local transform, `model_transforms.md` §"Transforms that stay in a mode spec") from the batch-safety class:

**First-run bootstrap for a self-referential model.** A non-self-referential model's first run creates its target directly with `CREATE TABLE ... AS SELECT ...` over the first batch. A **self-referential** model (`window_independence`'s `Ordered` self-edge, §"Window independence and self-referential models") cannot take that path: the first batch's own SELECT reads the target table via `smelt.<self>`, and no engine can resolve a table to itself mid-creation. Instead, when the target does not yet exist, the runtime first materialises an **empty** target table carrying the model's inferred output schema (column names and types, derived the same way any downstream consumer's schema is resolved), then executes every batch — including the first — as the ordinary partition DELETE+INSERT. The self-read over the empty table correctly yields no prior state for the model's first partition, so the trajectory it builds from there is identical to seeding the table by hand before the run. This bootstrap is a one-time, structural step keyed only on "does the target exist yet", not a property of the batch-safety class below.

| Class                | Chunking                                                                                   |
|----------------------|--------------------------------------------------------------------------------------------|
| `FullyBatchSafe`     | A single DELETE+INSERT pair covers any `[start, end)`. No chunking.                        |
| `BoundedSafe(n)`     | Auto-sized sub-ranges (3× context, clamped 7–90 partitions). Each sub-range is one DELETE+INSERT pair, executed sequentially in temporal order. |
| `PerPartitionOnly`   | One partition per iteration, sequential, temporal order. Each partition is one DELETE+INSERT pair. |

**Per-partition batching is calendar-aligned for Month/Quarter/Year.** When per-partition execution is forced (or `smelt backbuild --per-partition` is requested), batches for `Month`/`Quarter`/`Year` advance by true calendar units, so every batch lands on a month/quarter/year boundary regardless of month length. `Day` and `Week` use fixed 1-day / 7-day steps.

**Output grain may be finer than partition grain.** A model whose `partition_column` holds monthly boundaries may emit daily/hourly rows within them; batch-splitting operates on the *partition* grain and writes/reads finer rows in their entirety within each partition batch.

**Per-chunk transaction boundary.** Each chunk's DELETE+INSERT is one backend transaction. INSERT failure rolls back the chunk's DELETE; earlier committed chunks do **not** roll back — partial progress is intentional since each chunk is idempotent.

**Failure mode.** A run halts at the first failed chunk and exits non-zero. Re-running the same `[start, end)` resumes correctly because every committed chunk is idempotent.

**Late-arriving data (interim guidance).** smelt does **not** auto-re-run partitions when data arrives late. Two interim mitigations: (1) trail `--event-time-end` behind real-time by the source's known latency; (2) run overlapping ranges (e.g. always re-process the last 7 days). A planned automated mechanism is per-column `data_latency:` (Known Divergences). The contract-level statement of this behaviour is the derived horizon (§"Windowed maintenance and the horizon"): a late arrival past the derived clamp is silently excluded from the maintenance run, and surfacing it is a model-author + data-quality concern; the mitigations above only widen the window a late row can still land in.

#### Per-partition equivalence

For every partition `p` in the run window `[run_start, run_end)`:

```
partition_grain_run(model, [run_start, run_end)).where(partition_column = p)
  == full_refresh(model).where(partition_column = p)
```

This is the partition-grain specialisation of the framework's processed-input equivalence invariant (§"The equivalence invariant"), and of the plan's `S`-vector refinement (§"Per-cell admission"). It is independent of run-window size.

**Column-locality (partition-grain-local).** The equality holds for **local** columns — those whose value depends only on source rows visible within the model's source-filter ranges. A column depending on history outside those ranges (a cumulative aggregation such as connected-components or backward-fill) is **not equivalent**: its per-partition value reflects state at run time, not the final cumulative state. Such a column forces its source to `Unbounded` and the model to `PerPartitionOnly`; the run is correct as-of-the-run, just not equal to a full refresh that re-runs every partition with the final input.

**Equivalence is up to full-refresh non-determinism.** The equality is bit-identical on **deterministic** columns. A column with `contract: plausible` need only be a *plausible full-refresh value*. This never extends to a column that governs *which* rows exist, *where* they are partitioned, or *how* they are deduplicated (Semantics §"Safety checks").

#### Safety checks (per-cell admission for the partition grain's recompute corner)

The optimizer rejects a partition-grain model whose SQL uses constructs that break the partition-DELETE-then-INSERT contract. Each check applies a shared `model_properties.md` proof to discharge one of §"Per-cell admission"'s obligations for the recompute-a-region corner over this output shape; the table below names, for each check, the obligation it instantiates. Each check is individually disabled via `safety_overrides.allow_<check>: true` (opt-in, recorded).

| Check | Admitted when | Obligation instantiated |
|---|---|---|
| **Window functions** | `OVER (PARTITION BY <keys>)` where `<keys>` is a **superset** of `partition_column` (Property: *partition alignment*, scoped over window `OVER`) — every window then evaluates within a single partition, so DELETE+INSERT of whole partitions cannot change its result. Also admitted when `PARTITION BY` omits `partition_column` but the `OVER` clause carries a bounded `RANGE BETWEEN INTERVAL '…' PRECEDING` frame with no `UNBOUNDED` bound (Property: *frame-reach taxonomy* — a derivable reach the source read widens to cover). `UNBOUNDED PRECEDING`, or an `OVER (...)` with no `PARTITION BY`, is never admitted this way. Escape hatch: `safety_overrides.allow_window_functions: true`. | Obligation 4, *bounded reach* |
| **`HAVING`** | the enclosing scope's own `GROUP BY` key is a **superset** of `partition_column` (Property: *partition alignment*, scoped over `GROUP BY`) — every group is then scoped to a single partition value, so group composition matches a full refresh restricted to that partition. | Obligation 4, *bounded reach* |
| **`DISTINCT`** | `partition_column` is projected in the same scope (Property: *partition alignment*, scoped over the select list) — two rows can only collide on a `partition_column`-bearing row when they agree on the partition. | Obligation 4, *bounded reach* |
| **`LIMIT`** | never — a row-count cap never commutes with the partition filter: which rows survive depends on which other rows are present, and that set differs between a partition-grain run and a full refresh even when the cap value is unchanged. | fails obligation 4 unconditionally |
| **Subqueries** (`SELECT ... FROM (SELECT ...)`) | rejected unless overridden. A `WITH`-clause CTE is *not* gated by this structural check; only a subquery nested in FROM/JOIN is — CTE bodies flow through bound derivation via the *body-structure classifier* property. | Obligation 4, *bounded reach* |
| **Non-deterministic functions** | confined to a payload column with `contract: plausible` (below). | Obligation 6, *well-defined groups* (the deterministic/plausible split must not blur skeleton vs payload) |

All partition-alignment checks are evaluated **per scope**: a `UNION` branch's own `HAVING`/`DISTINCT`/window is judged against that branch's own key set, never inheriting alignment from a sibling or the outer query (Property: *partition alignment* is a per-scope containment fact; *set-operation distribution* governs how the framework distributes over branches).

**Non-determinism and the payload rule.** The partition grain consumes the *determinism (run vs row) + nondeterminism predicate + taint* property (`model_properties.md`). A non-deterministic value is admitted only when it flows **exclusively** into a column declared `columns.<c>.contract: plausible` — a payload written once per window and never read back to place, filter, group, or dedup a row. The taint check enforces three **hard exclusions**, rejecting regardless of the opt-in and naming the offending position: the `event_time_column`/`partition_column` expression; any `unique_key` column; any row-set-membership or grouping position (`WHERE`, `HAVING`, `JOIN … ON`, `DISTINCT`, `GROUP BY`, or a window's `PARTITION BY`/`ORDER BY`/frame). The run-nondeterministic class (`NOW()`/`CURRENT_*`) is additionally admitted as a **direct** SELECT-list projection even into a column without `contract: plausible`, because compile-time pinning (`model_transforms.md`) freezes it once per run — every row of a run sees one value, so a direct projection carries no cross-run variance. The row-nondeterministic class (`RANDOM()`/`UUID()`) still requires the target column to be declared `plausible`. Declaring an excluded column `contract: plausible` is a configuration error. The blunt `safety_overrides.allow_nondeterministic` drops the guardrail wholesale and is discouraged.

#### Event-time outer-visibility (partition-grain-local)

The outer output-clamp injects a `WHERE event_time_column >= start AND event_time_column < end` at the outermost SELECT. For that to bind correctly, `event_time_column` must be **accessible** there. A plain `UNION`/`INTERSECT`/`EXCEPT`, or a `UNION ALL` whose branches cannot be proven traceable, would bind the clamp to only the first branch and produce wrong results; a subquery FROM that does not project `event_time_column` references an inaccessible column. Either case is rejected with `EventTimeColumnNotVisibleAtOuterSelect` (Error) before execution.

A `UNION ALL` is **exempt** when every branch's projection of `event_time_column` traces `Traceable` (Property: *event-time monotonicity trace*; distributed by *set-operation distribution*) back to a real source's own partition column: per-source pushdown then narrows each branch's scan independently and the outer clamp's placement is immaterial. A `StaticSeed` branch is named and rejected; a `NotTraceable` branch conservatively keeps the whole-model outer clamp.

#### Observing the per-source clamp (partition-grain-local surface)

Because lookback is *derived from the model's SQL rather than declared* (Design), the author has no declaration to read back; the derived clamp — the window `partition_col ∈ [run_start − before, run_end + after)` each `smelt.<path>` reference is read under — is surfaced instead, so the author can confirm the analyzer read their SQL as intended. Two surfaces expose it, both using the ISO-8601 duration rendering of the bound:

- **`smelt explain` (`--json`).** The per-cell `source_bounds` map reports, per source, its `source_partition_col` and derived `(before, after)` offsets. With a concrete run window it additionally resolves the scan window `[run_start − before, run_end + after)`.
- **Editor hover (LSP).** Hovering a `smelt.<path>` reference in a partition-grain model shows that reference's clamp alongside the existing schema/column readout.

The bound outcomes render distinctly so the readout communicates *why* a source is read the way it is:

| Outcome | Readout |
|---|---|
| `Bounded(c, 0, 0)` | read partition-by-partition; no lookback or lookforward |
| `Bounded(c, before, after)` | the window `c ∈ [run_start − before, run_end + after)`, with `before`/`after` shown |
| `Unbounded` | read across all history (cumulative); forces `PerPartitionOnly` |
| lookup (no `timeseries:`) | read in full; not a pushdown candidate |

A `NotDerivable` source is refused at planning time (§"Batch safety classification"), so it surfaces the refusal diagnostic instead of a per-source window.

#### Functions inside partition-grain model bodies

A partition-grain model body may call transparent functions (`smelt.define`-resolved) and opaque calls (`smelt.extern`, canonical built-ins, source references). Function expansion (`expansion.md`) runs **before** every analysis stage here — bound derivation, source-filter pushdown, and most batch-safety sub-checks all see the expanded CST, so a `LAG()` inside a `smelt.define` body and one inlined at the call site are indistinguishable. The outer output-clamp is injected at the outermost expanded query and so sees columns produced inside expanded bodies; source-filter pushdown reaches `smelt.<path>` references that originated inside a `smelt.define` body via expansion. **Exception:** the `OVER`-clause admissibility sub-check scans the outer model SQL before expansion (Known Divergences).

**Opaque calls remain black boxes.** Bound derivation cannot read through `smelt.extern`/built-ins. A partition-grain model whose time-dependence is hidden behind an opaque call is `NotDerivable` and refused, unless a bound is provable from the surrounding SQL (a WHERE clause, an explicit RANGE-windowed projection). Cross-link: `planner_integration.md` §"Optimization boundary: transparent vs black-box".

#### Window independence and self-referential models

Whether windows may be built **in parallel** or must be built **sequentially in temporal order** is the *window-independence / ordered-execution* property (`model_properties.md`), derived from the model's dependency graph, never declared. The partition grain's application:

- **Window-independent (the default).** Every window is a pure function of source rows in its own scan range (widened by the derived lookback). The entire safe slice the recompute corner admits is window-independent — the lookback reaches into *sources*, never the model's own earlier partitions — so a backfill of `[t₀, tₙ)` may split into sub-ranges built in any order, including in parallel.
- **Window-dependent → ordered.** A **self-referential** partition-grain model — one reading its own prior partitions via `smelt.<self>` (a running balance, a partition-by-partition state machine) — is **in scope** and still executes as partition DELETE+INSERT (it stays a partition-addressed table; it does **not** become key-grain), but the runtime must build its windows **sequentially in strict temporal order**, and its backfill may not be parallelised or reordered. A self-edge the planner cannot prove converges partition-by-partition (a self-reference reading *forward* or across all history) is refused at planning time, not silently mis-parallelised.

This is the same stateless/stateful spine that separates the partition grain from the key grain: a self-referential partition-grain model is *stateful-ordered* in execution yet keeps the partition-grain *output shape* (partitioned, per-partition-equivalent within each window's input).

**Ordered execution composes with the derived output window.** An `Ordered` self-referential model's write window is rebased by the same derived-output-window rule a window-independent model gets (§"Execution model (DuckDB, current)" above, `model_transforms.md` §"The output window is derived, never assumed"): when the model's `partition_column` is itself derived and a genuine Form B relation — anchored on a *non-self* source — declares that it skews away from the driving date column, a run requesting `[D, D+1)` also rewrites the skew-reached neighbouring partitions, exactly as it would for a window-independent model. Ordering then applies over the *rebased* partitions, not just the originally requested ones: every partition in the rebased range still builds strictly sequentially, in temporal order, one partition per batch. The self-edge itself is never a skew anchor — its own bounding relation (the backward-bounded read that proves the `Ordered` verdict) is a distinct, already-proven convergence mechanism, not a partition-column skew declaration, even when the self-referenced table's column happens to share the model's own `partition_column` name.

#### State ownership

smelt does not track watermarks, offsets, or run history for partition-grain models — the backend owns computational state (DuckDB: table state + transactions; future Delta/Spark: transaction log + MERGE; future Flink: checkpoints). Optional run-state tracking with gap detection is opt-in via the `state.mode: intervals` posture (`virtual_environments.md`); the on-disk layout is owned by `run_state.md`.

#### `partition_column` validation

Partition-column projection is owned by `timeseries.md` §"Constraints & Invariants" rule 1: `partition_column` must appear in the model's output `SELECT` (and in the `GROUP BY` when grouping is present), else `MalformedTimeseries`. The partition-grain rule consumes that guarantee rather than re-checking.

### The key grain (`grain: key`)

The key-addressed shape: keyed state, one row per `unique_key`, kept current by the fold-a-delta corner (keyed `merge_into`). Its declared surface is §"Key-grain declaration (`grain: key`)"; the machinery below is key-grain-**local**.

#### The two run shapes (derived, never declared)

The run shape is the keyed application of the input-consumption axis (`models.md` §"Input-consumption axis"), derived from the driving source:

- **Window-forward** — the FROM clause contains exactly one source whose resolved target declares `timeseries:` (the **driving source**, resolved by the shared driving-fact / anchor proof; zero clocked sources means snapshot-reconcile below; two or more is `KeyedMultipleDrivingSources`). The run steps over the source partitions covered by `[run_start, run_end)` **in temporal order**; for each partition, source-filter pushdown injects the partition's window onto the driving source's reference, the per-partition delta SELECT executes, and a `merge_into` folds the delta into the target with the per-column combiner map. Non-timeseries sources (lookups / dimensions) are read in full each step. If the output table does not exist at the first step, it is created from that step's delta (`CREATE TABLE AS SELECT`).
- **Snapshot-reconcile** — no clocked source. The run re-scans the source whole, computes the per-key aggregation, and `merge_into`s the result: matched keys are overwritten, unmatched inserted. A key present in the store but **absent from the incoming scan is retained** unchanged; deletion requires an explicit mechanism (out of scope, §Known Divergences).

Out-of-order, parallel, or sliced-backfill window application is admitted **iff the model is order-independent** (below); otherwise windows must be applied sequentially in temporal order.

#### Derived execution postures

Three model-level properties are folded from the column families; each is derived, surfaced by `smelt explain`, and never declared:

1. **Re-run tolerance** — may an already-merged window be blindly re-merged over *unchanged* input? Holds iff every column is idempotent, i.e. **no additive-fold column**. For re-run-tolerant models a repeated window converges (`GREATEST(x, GREATEST(x, y)) = GREATEST(x, y)`); for additive models it double-counts and must be refused (the ledger, below).
2. **Order-independence** — may windows be applied out of order or in parallel? Holds iff every column's combiner is order-independent: the extremal/lattice and proven once-write families qualify; the **order-monotone overwrite family does not** (its order-independence holds only up to ordering-key ties, which are not statically excludable — §"Ordering ties"), so any model with an overwrite column executes windows sequentially in temporal order.
3. **Reprocessing refusal** — a window whose *input changed* since it was merged must not be re-merged for **any** family: an irreversible fold cannot un-see a removed contribution, and an overwrite cannot retract a superseded-by-nothing value. Detection and mitigation below.

#### The transactional merge ledger

Every **window-forward** keyed model maintains a per-model **ledger** — a small backend table recording each merged window — written **in the same backend transaction** as that window's `merge_into`. Its role by posture:

- **Additive-fold models** (not re-run tolerant): a run whose window is already ledgered is **refused** (`KeyedReprocessedWindow`) — exactly, not best-effort. Crash resume merges only unledgered windows; a run interrupted at window *k* of *n* resumes correctly by re-running the same range.
- **Re-run-tolerant models**: a ledgered window may be re-merged (a no-op on unchanged input); the ledger serves reprocessing detection and `--auto` bookkeeping, not refusal.

Snapshot-reconcile models keep no ledger — each run is a self-contained reconciliation and re-running is always safe. The ledger is backend-resident and transactional with the write it describes; it is a **correctness structure**, distinct from the opt-in run-state observability surface (`run_state.md`).

#### Admission matrix (column family × source shape)

Which families a model may use depends on its run shape. This is the key-grain instance of §"Per-cell admission": each cell in the matrix below is that framework's obligations 2 ("faithful fold") and 3 ("combiner algebra class") discharged for one `(column family × run shape)` pair — fold families consume **events** (each row contributes exactly once, satisfying the faithful-fold obligation only under a replayable, retraction-free feed); overwrite families consume **observations** (each row supersedes, so they discharge the obligation only under the snapshot's current-state semantics, never a fold). The matrix is checked per column:

| Column family | window-forward (clocked source) | snapshot-reconcile (mutable snapshot) |
|---|---|---|
| additive fold | ✓ (obligation 2, ledger-enforced) | ✗ — re-folding state double-counts (fails obligation 2) |
| extremal / lattice fold | ✓ (obligation 2) | ✗ — observer semantics (below); fails obligation 2 |
| order-monotone overwrite | ✓ (obligation 2) | ✗ — observer semantics (below); fails obligation 2 |
| once-write | ✓ (obligation 2, provenance proof) | ✗ — observer semantics (below); fails obligation 2 |
| plain overwrite | ✗ — order-dependent over events (fails obligation 3; `KeyedUnknownCombiner` names the `MAX_BY` fix) | ✓ (obligation 3, current-snapshot semantics) |

The three snapshot ✗ cells marked *observer semantics* are not double-count hazards — those families re-merge safely — they are **equivalence failures**: `MIN(price)` folded over successive snapshots computes *min ever observed* while a full refresh over the current snapshot computes the *current* min; `MAX_BY(attr, updated_at)` retains a stale incumbent forever if a mutation regresses the ordering value; `COALESCE`-once-write captures *first observed*, unrecoverable from the current snapshot. Each is a different contract (a history *observation*, not a recomputation) and is refused (`KeyedSnapshotSourceUnsupportedColumn`) rather than admitted silently — obligation 2 fails closed, never approximated.

#### End-state equivalence: the SQL is the oracle

The key grain upholds the **end-state specialisation** of the processed-input equivalence invariant (§"The equivalence invariant"), and because the body is required to be the aggregation itself (§Surface), the oracle is executable for every admitted model — it is the model's **own SQL**:

- **Window-forward:** for any set `S` of processed driving-source partitions and any admitted ordering over `S`, the stored state equals the model SQL evaluated over `source.where(partition ∈ S)`. Order-independence beyond sequential-temporal application holds per posture 2 above; for overwrite columns it holds **up to ordering-key ties** (§"Ordering ties").
- **Snapshot-reconcile:** the stored row for every key **present in the current snapshot** equals the model SQL evaluated over that snapshot. Keys absent from the snapshot are retained (a named divergence from the oracle relation — the stored table is the oracle's rows plus retained departed keys).

#### No write-eligibility clamp

There is **no write-eligibility clamp**: a run merges **every** delta row it scans, into whatever key it names, however old that key is. A derivable forward reach is computed and reported (`smelt explain`) but never gates admission and never bounds which keys a run may touch — so no scanned input is ever silently dropped. (The contract-level statement and its rationale — why this differs deliberately from the partition-grain horizon — is §"Windowed maintenance and the horizon".)

#### Key temporal locality (the time-partitioned output)

A keyed model may time-partition its output with a `timeseries:` block (grammar and structural rules: `timeseries.md`; the named columns must be projections of the model, and `event_time_column` may name the partition column itself). Admission requires **key temporal locality** — a guarantee that every stored row a run's deltas can touch lies within a computable **slice** of the output's time axis. Locality is what lets the `merge_into` target scan be pruned to the slice, and what lets downstream consumers window over the output.

Structural preconditions, checked before the routes:

- the run shape is **window-forward** — the partition values derive from the driving source's clock; snapshot-reconcile establishes no locality;
- `partition_column` names either a `unique_key` column or a non-key projection in the extremal-fold, order-monotone-overwrite, or once-write family, provably NOT NULL from a key's first stored row (`timeseries.md` validation rules);
- the block's `granularity` equals the driving source's granularity.

Any one of three **routes** establishes locality:

1. **Key-embedded** — `partition_column` is a `unique_key` column. A stored row's partition value is its key's own; a delta touches exactly its own partition values. Slice: the run's scan window, widened by the derived lateness/skew margins.
2. **Key-determined** — the partition projection is a per-key constant under the once-write provenance proof (`model_properties.md`): a key-derived expression, or a declared functional dependency over a column present non-null on every input row. Every delta row carries its key's fixed partition value, so the slice is the delta's own partition values — exact **regardless of key age** (a years-old key prunes as tightly as a fresh one).
3. **Recurrence-bounded** — a **key-recurrence bound** `r` holds: every pair of input rows sharing a key lies within `r` of each other on the event-time axis. `r` is derived from the model's SQL where statically decidable; otherwise it is declared on the driving source (`sources.md` §"Source YAML shape", `key_recurrence`). Slice: the scan window widened backward by `r`, plus the derived margins. A **declared** `r` is admitted only **checked**: the run verifies at merge time that no delta row matched (or would duplicate) a stored key outside the slice, and any violation fails the run transactionally (`KeyedRecurrenceBoundViolated`). A declaration can bound work; it can never silently drop data.

**Pruning is not a write clamp.** Slice pruning is no-op elimination on the merge's **target scan**: rows outside the slice provably cannot match a delta key (routes 1–2) or are checked not to (route 3). Every scanned delta row still merges — the no-write-eligibility-clamp rule above is unchanged. The general principle is stated in §"Windowed maintenance and the horizon": only proofs prune; a declared bound is admitted only checked; no unproven bound ever refuses a write.

**Row movement.** Under routes 1–2 a key's partition value never changes. Under route 3 it may move (an extremal or overwrite partition projection superseded by a late row); the merge updates the stored row in place, partition value included, and both the old and new values lie within the slice by the bound. Movement does not change the derived postures — an overwrite column forces sequential temporal order exactly as before.

**Per-slice equivalence.** With locality established, the invariant is additionally checkable slice-by-slice: for any output slice, the stored rows equal the model SQL evaluated over the source rows within the slice's derived reach — the keyed analogue of the partition grain's per-partition strengthening (§"The equivalence invariant").

**The output as a clocked source.** An admitted block makes the output a clocked, time-partitioned table: downstream partition-grain models receive source-filter pushdown against it, and a downstream keyed model may take it as its clocked driving source — the clock propagates through the DAG instead of stopping at the keyed stage. The output's **settle bound** — how long a written slice may still change — is derived and surfaced by `smelt explain`: under route 1 a slice settles with the source's lateness margin; under route 3 after `r` plus the margins; under route 2 it never settles (a late delta may touch an arbitrarily old slice). A re-written slice is *changed input* to downstream consumers, handled by the ordinary staleness machinery (§"Interaction with `--auto` / staleness").

#### What the composed shape uniquely enables

The composed shape — key-addressed **and** time-partitioned — is not "keyed with an
optimisation"; it is the only form in which the following hold, which is why treating the axes
as exclusive (§Surface "The two axes are orthogonal") forecloses real capabilities:

- **Propagation admissibility.** A bare keyed node refuses in the graph layer — it has no
  partition axis to carry interval dirt. A locality-admitted keyed output *has* one: it
  participates in forward propagation and backward resolution as a clocked node, its edges at
  the declared `timeseries.granularity` like any other node (§"The graph layer"). The composed
  shape is the only way a keyed stage can sit *inside* a propagation chain rather than
  terminating it.
- **Exact key→partition dirt projection.** Under locality routes 1–2 a stored row's partition
  value is a per-key constant, so a key-level change set projects to **exact** partition
  intervals — the keys' own partitions, no widening. Under route 3 the projection widens
  backward by `r` plus the derived margins (widen-never-narrow). This is what lets a composed
  node hand precise interval dirt downstream without any key-level dirt representation in the
  graph itself.
- **Slice-bounded no-op write elimination.** The conditional write
  (§"Windowed maintenance and the horizon", category 2) must read stored rows to compare
  against candidates. On a bare
  keyed output that read is the whole key space; on a composed output it is bounded by the
  pruned target slice — the compare cost is proportional to the slice, which is what makes
  suppression affordable at volume.
- **Settle-bound × observed-delta composition.** The settle bound (derived, static: when a
  written slice can no longer change) composes with the observed output delta (dynamic: which
  rows a run actually changed — §Future Extensions): consumers skip settled slices
  unconditionally and skip unsettled slices whose observed delta is empty. Together a stable
  upstream chain degenerates to empty-delta no-ops with a provable horizon behind it.

The first two bullets bind at the graph layer, the third at statement emission, the fourth
across both; their implementation status is recorded in §Known Divergences.

#### The maintenance boundary

On the algebraic ladder (§"The algebraic maintenance ladder") the keyed families sit on the **direct-monoid rung**: every catalogued combiner folds `(state, delta)` with no inverse and no history re-read. The additive family is additionally a **group** (invertible), which is what a future subtract-then-add reprocessing path would exploit; the idempotent families are monoids but not groups (a folded contribution cannot be un-seen), which is why reprocessing is refused for them. Rungs 2–4 (decomposed state + presentation view for `AVG`-class aggregates; group-rung retraction; the opt-in bounded-domain multiset for exact holistic aggregates) grow this shape without changing its contract; the transforms are catalogued in `model_transforms.md` and the `bounded_domain:` budget declaration in `model_properties.md`. Beyond the ladder — general-operator retraction over joins, unbounded non-additive state — is delegated to `refresh: materialized_view`.

#### Reprocessing

If a merged window's source data changes, re-running it does not produce correct state for any family (posture 3). The rule refuses at planning time when it can detect it — the ledger says the window was merged; `--auto` staleness says the input changed — with `KeyedReprocessedWindow` pointing at the two mitigations: `--full-refresh` (truncate-and-rebuild), or a manual cascade rebuild. Subtract-then-add for all-invertible models is a future path (§Known Divergences).

#### Ordering ties (order-monotone overwrite)

The pairwise combiner for `MAX_BY(value, ordering)` is: the delta wins iff `delta.ordering > target.ordering` (strict); **on equality the incumbent (target) wins**. This is deterministic given the processing history but **not order-independent when ties occur across windows** — which is why overwrite columns force sequential execution (posture 2). The recommended modelling practice is a composite, provably-tie-free ordering expression (e.g. `(updated_at, source_seq)`); the classifier cannot verify uniqueness and does not claim to.

#### Enrichment joins

A fact-to-dimension join that brings an enriching event in as a separately-arriving relation is admitted when its per-key contribution is **provably monotone** — the join-contribution monotonicity proof (`model_properties.md`): the contribution feeds only extremal, order-monotone, or once-write columns and does not fan into a decrementing aggregate. The maintainability line is monotone-vs-retractable **semantics, not join-vs-union spelling** — the join form is normalised to the same keyed-monoid merge as the union form. Only a genuinely retractable contribution is refused (`KeyedRetractableContribution`). A **re-scanned existence flag** additionally requires the dimension source to be declared `append_only` (`sources.md`); extremal milestones are safe regardless. Where a dimension batch's forward reach `H` is **derivable from the model's SQL**, the dimension-driven horizon-bounded MERGE (`model_transforms.md`) may clamp the enrichment *recompute* to `[event_ts, event_ts + H]` — a scan-side bound that cannot under-cover because it is derived; where `H` is not derivable, the transform is not licensed and the enrichment evaluates through the ordinary widened scan. No declared value ever truncates a recompute or a write.

#### Key-grain output shape

One row per `unique_key`; column names are the projection's `AS` aliases (or source column names). By default there is no `partition_column`, no `event_time_column`, and no `timeseries:` on the model, and downstream consumers see the output as a lookup table read in full each run, identical to any non-timeseries source. With an admitted `timeseries:` block (§"Key temporal locality") the output is instead a clocked, time-partitioned keyed table — still one row per key — that downstream consumers window over like any clocked source.

#### Functions inside keyed bodies

Function expansion (`expansion.md`) runs **before** the classifier. Projection reading, GROUP-BY inspection, FROM-clause walking, family classification, and pushdown operate on the expanded CST. A `smelt.define`-resolved call is admitted iff its expanded body produces a catalogued aggregator at the outermost expression position — the pattern functions (§Surface) are admitted exactly this way, with no privileged treatment. Opaque calls (`smelt.extern`, non-inlinable built-ins) in the projection list are rejected via `KeyedUnknownCombiner`.

#### Interaction with `--auto` / staleness

- **Window-forward:** stale driving-source windows are re-processed subject to posture — re-run-tolerant models re-step exactly the stale windows (safe by idempotence); additive models refuse re-processing of ledgered windows (`KeyedReprocessedWindow`) and steer to `--full-refresh`.
- **Snapshot-reconcile:** the model is treated as always-stale; every `--auto` run reconciles.

### Interval versioning (`versioning: interval`)

The key grain's history-keeping sub-declaration (SCD2): keyed state plus a validity interval per version. Its declared surface is §"Interval-versioned declaration (`versioning: interval`)"; the machinery below is local to `versioning: interval`.

#### End-state equivalence (interval-keyed)

The profile upholds the **end-state equivalence invariant** in its interval-keyed specialisation (§"The equivalence invariant"): the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same set of processed snapshots, independent of the order in which non-overlapping snapshots were merged. smelt owns freshness (pull) — the history is correct as of the last `smelt build`.

Order-independence holds because validity is anchored to the source's event time, not the run clock (see §"Validity stamped from source event-time"): the close-old / open-new combiner reads versions in event order via the **driving-fact / anchor resolution** and **ordered-execution** proofs (`model_properties.md`), so replays and out-of-order windows converge to the same history rather than shifting interval boundaries.

#### Input consumption is derived from the source

How new input is discovered is never declared on the model; it is the input-consumption axis (`models.md` §"Input-consumption axis"), derived from the source's shape:

- **Window-forward** — a source carrying a `timeseries:` declaration (an update-events / CDC feed) is consumed in `--event-time` run windows applied to the *source's* `partition_column`, exactly as the plain key grain consumes its driving source (§CLI). Only the new tail is read (source-filter pushdown, `model_transforms.md`). Because the close-old / open-new combiner consumes versions in event order, windows are applied in temporal order (ordered execution, `model_properties.md`).
- **Snapshot-diff** — a mutable snapshot source (no monotone clock) is re-scanned each run and compared against the stored current versions; the end-state contract is identical, only the scan cost differs.

The choice between the two is the mutation-profile world-fact (`sources.md`) feeding the input-delta-discovery proof (`model_properties.md`); moving along this axis never changes the equivalence contract, only what is scanned.

#### Interval-versioning admission

Every admission check for this profile is one instance of §"Per-cell admission" evaluated for the fold-a-delta corner over a key-grain-plus-interval output:

- **Replayable input / faithful fold** (obligations 1–2) — the close-old / open-new combiner consumes an update-events / CDC feed (replayable, append-only) or a mutable snapshot (re-scanned whole each run); either discharges the obligation for its own consumption route, never a hybrid of the two on one model.
- **Combiner algebra class** (obligation 3) — the combiner is the profile's own local machinery (below), not a catalogued key-grain column family; it is admitted once per model, not per column, because every tracked attribute is folded through the same close-old / open-new step.
- **Bounded reach / bounded footprint** (obligations 4–5) — window-forward: the reach is the run's event-time window on the driving source, exactly as the plain key grain (§"Admission matrix"); the footprint is the set of keys touched by that window's rows. Snapshot-diff: reach and footprint are the whole snapshot and the whole key space — an intentional escape hatch for a source with no monotone clock, not a derivation gap.
- **Well-defined groups** (obligation 6) — all tracked attributes plus the validity columns form one column group; a version change is a single indivisible event across every tracked column, so there is no sub-model factoring to compute.

The following is owned in full by this spec — it is the machinery meaningful only inside `versioning: interval`.

#### Close-old / open-new interval maintenance (the combiner)

The combiner the windowed-keyed-maintenance driver folds through. For each incoming row, keyed by natural key:

1. Look up the key's current (open) version in the stored table.
2. If no current version exists, **open** a new version: insert the row with `valid_from` = the incoming event time, `valid_to` = open, `is_current = true`.
3. If a current version exists and a **tracked attribute** differs, **close** the old version (set its `valid_to` = the incoming event time, `is_current = false`) and **open** a new one at that boundary.
4. If a current version exists and no tracked attribute differs, do nothing — no spurious version.

The close and the open share the same boundary timestamp, so intervals abut without gaps or overlaps. The mechanism is emitted as a keyed `merge_into` (`model_transforms.md`) — matched keys close-and-reopen, unmatched keys open — so history is never re-read wholesale.

#### Validity columns (smelt-managed)

`valid_from`, `valid_to`, and `is_current` are **managed by smelt**, not projected by the user's SELECT. The user projects only the natural key and the tracked attributes; smelt appends and maintains the interval columns. The open interval's `valid_to` is either NULL or a far-future sentinel (undecided — see §Known Divergences); `is_current` is a convenience flag equivalent to "`valid_to` is open" that indexes the current-version lookup the combiner performs every run.

#### Tracked-attribute selection

A new version is opened for a key only when a **tracked attribute** changes between the stored current version and the incoming row. By default every projected non-key column is tracked. Whether a modeller can mark a column *untracked* (a slowly-drifting field that should not open a new version), and whether that is derived from the SQL or declared, is an Open Question (§Known Divergences); the posture is to derive the key and tracked set from the SQL where unambiguous rather than restate them in a strategy block (§"Key-grain design").

#### Validity stamped from source event-time (not run clock)

`valid_from` / `valid_to` boundaries are stamped from the **source's event time** — the update-events feed's event-time column, or the snapshot's as-of timestamp — **never the run clock**. This is what makes the history replay-safe: re-running a window, or backfilling windows out of order, reproduces byte-identical interval boundaries, so end-state equivalence survives replays. A run-clock stamp would make the same version boundary depend on *when* `smelt build` happened to run, breaking order-independence.

#### Deletion handling

A key present in the store but absent from the incoming set is a **retraction**, and how it is handled is settled here as a soft-close: the key's current version is closed (`valid_to` set, `is_current = false`) with no new version opened, marking "no longer present as of this event time." The event time used is the run's window boundary for a window-forward feed, or the snapshot's as-of time for snapshot-diff. A hard delete (physically removing the key's rows) is **not** the default — the whole point of `versioning: interval` is to retain history — but the exact surface for opting into a hard delete, and for *late corrections* to an already-closed interval, remain Open Questions (they are the retraction question the key grain shares; §"Reprocessing"). A CDC feed that carries explicit delete events resolves this directly: the delete event is the close signal.

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

**One invariant, not two; addressing is the real axis — and it is per-cell.** An earlier cut split
the contract into "per-partition equivalence" (partition grain) and "end-state equivalence" (key
grain), one per output shape. That split was miscast: order/set-determinacy falls out of the single
invariant for *every* shape (the partition grain included), and per-partition equivalence is a
*strengthening* of that one invariant, not a peer of it. What actually drives the physical transform
is how a write **addresses rows** — partition-addressed (identity-free, whole-partition rewrite)
versus key-addressed (identity-requiring `merge_into`, reaching stored rows by key outside the input
window). The decisive point is that addressing is a property of *a write*, not
of *a model*: the declared shape facts (clock, identity) fix which addressings are **available**, but
each `(column-group × trigger × changed-input)` cell derives its own addressing from the
available-addressings rule (§"Per-cell write addressing"). SCD2 is the proof that addressing is
intrinsic to the *write*, not the source clock: its close-out write escapes the input time-window,
so that cell is keyed regardless of whether its source is clocked — while the same model's creation
cell is region-addressed. Deriving `grain: partition`'s old "addressed by whole-partition rewrite"
half per-cell (keeping only "a stored row is one row of a complete clocked table" declared) is what
puts each half on its correct side of the litmus line (`models.md` §Design). Full derivation:
`docs/research/20260716-relation-contract-and-per-cell-addressing.md`.

**The two mechanisms stay binary per cell; locality is a refinement, not a third pole.** Within a
cell, region-overwrite vs keyed-merge remains the binary write-scope corner (identity-free rewrite
vs identity-requiring merge); which concrete pattern realizes the corner is drawn from an **open
registry** (§"The write-pattern set is open"), so the *mechanism set* grows without the corner
distinction changing. Key temporal locality does not change how a keyed write is addressed — it is
still a keyed `merge_into` — it adds a proof about *where* addressed rows can live, licensing target
pruning, a time-partitioned keyed output, and per-slice equivalence. Promoting it to a third
addressing pole was rejected: it would suggest a different write primitive and identity requirement
where there is none, and it would misplace a per-model derived/declared fact as a shape property
(`docs/research/20260705-keyed-time-superset.md`).

**The axes compose; exclusivity is the recurring error.** Because the pre-consolidation specs
were *named* "batched" and "keyed", text in specs, research, and plans repeatedly slid into
treating partition and key as rival modes — and each recurrence quietly re-derived designs that
forgot the composed shape: DAGs whose clock dies at a keyed stage, keyed nodes excluded from
propagation categorically rather than conditionally, conditional-write costs sized to whole key
spaces, dedup shapes falling "between the modes". The composed shape is deliberately first-class
(§Surface "The two axes are orthogonal — \"partitioned or keyed\" is a category error";
§"What the composed shape uniquely enables"), and it is where several capabilities pay best —
propagation through keyed stages, exact dirt projection, slice-bounded write suppression.
Reviewers should treat one-or-the-other phrasing anywhere in this corpus as a defect against
those sections, not a stylistic nit.

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
own section of this spec (or `materialized_view.md`). (So pushdown-depth is a SQL property and
lives in `model_properties.md`; backfill chunking, meaningless outside partition-grain execution,
stays in the partition grain's own section.) This gives every
capability exactly one home — what lets `smelt:validate` catch drift — without a mechanical
≥N-consumer rule; because these capabilities are broadly useful, building one before a second
consumer exists is fine. The invariant and ladder live in the shared sections because every shape
profile cites them as its contract; keeping them inside one profile's section would force the
others to reach into a sibling for their own contract. The key grain (§"The key grain
(`grain: key`)") remains the reference implementation of the key-addressed maintenance path
(retraction, reprocessing, presentation-purity) with its column-family catalogue — see its
declaration and semantics sections for a worked composition-contract example.

**Rejected alternatives**, briefly: a `strategy:` sub-knob (dbt's invisible-contract footgun); a
new `smelt-maintenance` crate (the derivation needs the tightest coupling to the sibling
classifiers; module boundary kept extraction-mechanical instead — `08-code-placement.md` §2.1);
qualifying the output clamp to a resolved inner alias (answers a question the output clamp must
never ask — `03-design-forks.md` F1); a third addressing pole for locality (it changes no write
primitive — §"The two mechanisms stay binary per cell" above); per-edge grain declarations (two
declarations can disagree — resolved by the derived label + check-only assertion, §"Grain is a
derived label"); a *declared model-wide* addressing token (the per-cell plan already knows better —
§"Addressing is per-cell", `docs/research/20260716-relation-contract-and-per-cell-addressing.md`);
a **closed** write-pattern enum baked into the surface (bakes today's engines in — §"The
write-pattern set is open"). Deeper rationale:
`docs/research/20260705-refresh-as-maintenance-plan/` (parts 01–10, with ratification records in
09 §1 and 10 §11).

### Partition-grain design

This section captures the partition-grain-**specific** rationale; the rationale for each shared property/transform lives in its owning spec, and the rationale for deriving strategy while declaring grain lives in `models.md` §Design and §Design.

**Logical SQL is pure; the framework injects the time filter.** A model body never contains `is_incremental()` or any conditional branching on full-vs-incremental. The same SQL is both descriptions; the framework injects the outer clamp and drives pushdown. *Jinja-style `is_incremental()` branching* (dbt) was rejected because it splits one model into two implicit ones that drift. The trade-off — partition-grain models must accept the framework's per-model filter shape — is policed by the batch-safety analysis.

**DELETE+INSERT over partition columns, not MERGE, for v1.** DuckDB's strategy is `DeleteInsert`. *MERGE* was rejected as the v1 default because it requires a `unique_key` (not every model has one) and carries cross-engine subtleties; it stays in the `IncrementalStrategy` enum for backends that opt in. DELETE+INSERT is idempotent under fixed input and aligns with the partition-column safety analysis.

**Three-class batch-safety taxonomy.** The `FullyBatchSafe` / `BoundedSafe(n)` / `PerPartitionOnly` roll-up (Semantics §"Batch safety classification") is partition-grain-local because it is meaningful only for this execution shape. *A binary safe/unsafe flag* was rejected — too many real workloads are bounded-safe and need auto-chunking. *A continuous safety score* was rejected — the user-facing decision is qualitative and maps directly to three backend-execution shapes.

**Derive lookback from the model's SQL, not from frontmatter.** The per-source bound is computed by the shared bound/reach derivation over the model's SQL (including inlined `smelt.define` bodies), not a `lookback_days:` YAML annotation, which would let declaration and logic drift (`feedback_derive_dont_declare`). The trade-off — a model with implicit time logic refuses partition-grain eligibility and must be rewritten into a derivable form — is arguably the right outcome. Deriving from SQL removes the artifact the author would read to confirm behaviour, so the derived clamp is made **observable** (Semantics §"Observing the per-source clamp") as the deliberate counterpart. Deeper rationale: `docs/research/20260521-incremental-as-planner-rule.md`.

**smelt does not own state — scoped to the partition grain.** Watermarks, run history, and offsets live in the backend; *owning a watermark store* was rejected as a v1 requirement because it duplicates engine state and opens a sync-correctness window. Optional run-state tracking is an opt-in extension. This doctrine is **specific to the partition grain**: `grain: key` maintains one deliberate exception, the transactional merge ledger (§"The transactional merge ledger") — a small backend-resident table written in the *same transaction* as the window's merge, so it cannot drift from the state it records and does not reopen the sync-correctness window this doctrine guards against. A consequence of the ledger's correctness role: a backend may only select a physical strategy that preserves the declared shape's invariants, which is why the partition-grain `Append` strategy below is unreachable until it is gated on ledger-verified unwritten windows (`docs/research/20260705-keyed-collapse-application.md` D7) — an unguarded append-only write could not detect a re-run without the ledger's bookkeeping.

**Non-determinism is opted in per column, and confined by proof.** Whether a column is acceptable-to-vary is a value judgement only the author holds, so it is **declared** (`columns.<c>.contract: plausible`) — the one place the derive-don't-declare default correctly yields. *A whole-model `allow_nondeterministic` boolean* was rejected as the primary mechanism because it drops the guardrail keeping non-determinism out of the skeleton roles. The per-column opt-in keeps the guardrail and still proves, by the shared taint flow, that the tolerance did not leak into the deterministic skeleton. Derivation: `docs/research/20260703-model-updates.md` §9.2.

### Key-grain design

**One mode; the column family is the pattern.** The running-aggregate, latest-value, and milestone patterns share the output shape (keyed), the invariant (end-state equivalence), the transform (`merge_into` via the one windowed driver), and the key derivation — they differ only in per-column combiner algebra, and every consequence of that difference (re-run tolerance, ordering, ledger, reprocessing) is derivable from the SQL. By the litmus rule (`models.md` §Design), facts that change only execution posture under an unchanged contract are **derived, never declared** — so they must not multiply the refresh enum. Splitting them into peer modes was rejected for a second, decisive reason: combiner intent is **per column, not per model** — the §Surface example mixes an additive fold, an overwrite, and two extremal milestones in one table, a shape no per-pattern mode can express without materialising the same keyed state several times. Full derivation: `docs/research/20260705-unified-keyed-refresh.md`; decision record: `docs/research/20260705-keyed-collapse-application.md`.

**The SQL is the oracle.** The body must be the aggregation itself so that `full_refresh(model SQL)` is an executable correctness oracle for every admitted model. A bare-projection surface with mode-imposed dedup was rejected: its full refresh is not one row per key, so the equivalence invariant would have no executable oracle and the mode would add semantics the SQL does not carry (`docs/research/20260705-model-refresh-review.md` §1.1). The plain-overwrite family (`ANY_VALUE`) exists to give the snapshot posture an honest aggregated spelling under this rule.

**Derive `unique_key` and combiners from the SQL, not frontmatter.** The `GROUP BY` names the key; each projection names its aggregator; the combiner is a fixed lookup. A config block restating them re-introduces metadata-vs-SQL drift (`docs/research/20260521-incremental-as-planner-rule.md`). If it is in the SQL, it is not also in YAML.

**No write-eligibility clamp.** A horizon-clamped merge (only keys newer than `run_start − H` are eligible) was rejected: it silently drops *scanned* inputs — the one silent-data-loss point in the maintained family — and it is not needed for correctness, since merge work is proportional to delta size. What a clamp would buy (settled-key GC, a work bound) is deferred optimisation and must arrive as a package with late-fact accounting (`docs/research/20260705-keyed-collapse-application.md` D6). Slice pruning under key temporal locality (§Semantics) is not such a clamp: it removes provably-unmatchable rows from the merge's *read* side — or, on the declared route, checks the bound transactionally — while every scanned delta row still merges. The narrow principle: only proofs prune; a declared bound is admitted only checked; no unproven bound ever refuses a write (§"Windowed maintenance and the horizon").

**Time-partitioned keyed output is locality-gated, not a new mode.** The (key, time)-addressed output cell absorbs the shapes that previously fell between the modes — event-grain dedupe over a bounded redelivery window (which the partition-local partition grain cannot dedup across partitions, and which an unpruned keyed merge cannot afford at volume), per-(key, period) aggregates, and the clock-sink problem where a keyed stage strips the timeseries property from the DAG so every downstream consumer degrades to full scans. A peer mode was rejected: the cell shares the key grain's invariant, oracle, driver, ledger, and column families, differing by one derived/declared world-fact — by the litmus rule (`models.md` §Design) that earns a gate, not a peer. The gate exists because without locality the merge target is the whole key space and an output clock would promise a partition structure the writes do not respect; the declared route is runtime-checked because an over-optimistic recurrence bound would otherwise re-import exactly the silent truncation the no-clamp rule exists to prevent (`docs/research/20260705-model-refresh-review.md` §3.2). Full derivation, including why the partition grain remains the honest peer for keyless/multiset bodies: `docs/research/20260705-keyed-time-superset.md`.

**The ledger is the deliberate exception to "smelt does not own state".** The partition-grain doctrine (backend owns watermarks/run history; §"State ownership") rejected a watermark *store* because it duplicates engine state and opens a sync-correctness window. The keyed ledger has neither defect: it is backend-resident and written in the same transaction as the merge it describes, so it cannot drift from the state it records. Without it, additive-fold models cannot detect a double-counting re-run and any mid-run crash forces a full rebuild — an unacceptable operational cliff for the family's most common combiners (`SUM`/`COUNT`).

**Observer semantics are refused, not smuggled.** Folding state observations (a mutable snapshot) into `MIN`/`MAX`/once-write columns yields min-ever / first-observed values no full refresh can reproduce — a genuinely different contract (a history observer). Admitting it silently would put two contracts behind one mode, the exact dbt-`strategy:` failure the refresh peers exist to avoid. The refused cells name the observer contract as the future opt-in path.

**Ties: honest boundary, not fake proof.** Incumbent-wins plus mandatory sequential execution makes overwrite columns deterministic-given-history without claiming an order-independence no static analysis can prove. A last-processed combiner (no ordering column, order-dependent for *all* rows) was rejected outright; the snapshot posture's plain-overwrite family serves that need where it is well-defined (one row per key per scan).

**No `safety_overrides:`.** The partition grain offers per-check overrides because some of its rejections guard partial-correctness properties a modeller may knowingly waive. Every keyed rejection guards the equivalence invariant itself — a bypass would produce silently order-dependent or double-counted state that is impossible to debug. The escape from a rejection is to remodel, or to move to `refresh: materialized_view`.

**One windowed executor, shared.** The window-forward step loop is the windowed-keyed-maintenance driver (`model_transforms.md`), parameterised by `(classifier, merge-SQL builder)`. A per-pattern copy of the loop was rejected as four-way drift risk; a consequence is that the mode inherits the driver's granularity support (§Known Divergences).

### Interval-versioning design

**A sub-declaration of the key grain, not a third grain.** `versioning: interval` composes onto `grain: key` rather than introducing a peer grain: row addressing is still by key, and the interval is structure *within* the key, not a different addressing scheme (`models.md` §"Refresh axis"). This is the shape-profile demotion's consequence for the former `refresh: versioned` peer: what changed was never the freshness owner (still smelt-per-run) or the addressing (still by key) — only the local combiner and the extra validity columns, which the litmus rule (`models.md` §Design) says are derived machinery, not grounds for a new enum value.

**A smelt-owned pattern, distinct from engine-owned SCD.** This profile is one of the patterns smelt maintains itself — it owns the combiner (close-old / open-new) and validates the profile against the derived properties rather than choosing it (§"Validator, not chooser"). An *engine-maintained* SCD2 is not a variant of this profile — it is hand-written SCD2 SQL declared `refresh: materialized_view`, where the engine's IVM runtime does the maintenance (`materialized_view.md` §Design "No named pattern"). The two are not this profile plus a maintainer flag; they are different modes with different freshness owners (`docs/research/20260703-model-updates.md` §17.8).

**The combiner stays local; the driver and `merge_into` are referenced.** Close-old / open-new is meaningful only inside this profile, so it lives here in full (`model_transforms.md` §"Transforms that stay in a mode spec"). The mechanisms it is emitted *through* — keyed `merge_into`, the windowed-keyed-maintenance driver, source-filter pushdown — are general capabilities referenced by name, not re-specified.

**Derive from SQL where possible.** Following the key-grain posture, the natural key and tracked attributes should be derived from the SQL and the model's declared key rather than restated in a strategy block wherever that is unambiguous (§"Key-grain design"). The precise derive-vs-declare line for change-tracking columns is an Open Question.

## Constraints & Invariants

### The contract, plan, and graph layer

- The **equivalence invariant** holds for every non-`full` model and on every ladder rung; a
  transform that cannot preserve it for a given model is refused, never applied approximately.
  Order/set-determinacy is a corollary of it for **every** shape (the partition grain included);
  per-partition equivalence is a *strengthening* of it, not a separate contract.
- **Write addressing** is the load-bearing axis, and it is **per-cell, not per-model**:
  region-addressed writes (identity-free) rewrite whole partitions; key-addressed writes
  (identity-requiring) `merge_into` by key and may write outside the input time-window. Which a
  cell uses is derived by the **available-addressings rule** — `available = declared contract facts
  × trigger/changed-input needs × equivalence invariant × backend capability` (§"Per-cell write
  addressing") — over the **open write-pattern registry**. The declared shape facts (clock,
  identity) fix which addressings are *available*; a model may derive region addressing for its
  creation cell and keyed addressing for a dimension-change cell. Some writes are intrinsically
  keyed regardless of source clock (SCD2's retroactive close-out). A keyed write on a clocked output
  is still **partition-scoped** to the touched partitions unless it provably cannot be
  (§"Per-cell write addressing"). Key temporal locality, where established, refines keyed
  addressing with a derived slice bound — target-scan pruning and per-slice equivalence — without
  changing the addressing corner (§"Key temporal locality").
- **The write-pattern set is an open registry, not a closed enum.** New patterns are admitted by
  declaring their required contract facts and discharging the equivalence proof obligation; the
  `write:` pin is an open, fail-loud name resolved against the registry; a pattern the target
  backend cannot execute is not a candidate (`architecture.md` capability registry). The stable
  contract is the admission function + equivalence gate, never the enumeration
  (§"The write-pattern set is open").
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
- The declared **`refresh:` value plus the shape-defining facts (clock `timeseries:`, identity
  `unique_key:`) are the only shape surface**; the `grain` label is a derived check-only assertion,
  physical write addressing is derived per cell (steerable only via the validated `write:` pin), and
  input-consumption is derived from the source — none declared per model as a driver. No `strategy:`
  sub-knob. The machinery **validates, never chooses** the shape or the addressing; a fallback to
  full refresh is a surfaced diagnostic, never an automatic switch.
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

### Partition-grain constraints

1. **Logical model is pure SQL.** No `is_incremental()`, no macros, no conditional branches. The framework injects the time filter.
2. **`timeseries:` is required for `grain: partition`.** A model with `grain: partition` and no `timeseries:` block is a hard error at workspace load (`models.md` §"Constraint violations").
3. **Strategy is not on the model.** Frontmatter declares `unique_key`; the backend chooses `DeleteInsert`/`Merge`/etc. for the recompute corner's execution.
4. **smelt does not manage computational state — a partition-grain-scoped doctrine.** Watermarks, offsets, and run-history live in the backend. The one deliberate exception across the refresh axis is `grain: key`'s transactional merge ledger (§"The transactional merge ledger"), which is backend-resident and transactional-with-the-merge rather than a separate synced store, so it does not reintroduce the sync-correctness window this constraint guards against. A backend may select only a physical strategy that preserves the declared shape's invariants; the partition grain's `Append` strategy (below) is unreachable until it is gated on ledger-verified unwritten windows.
5. **Output-filter injection is per-model; source-filter pushdown is per-reference.** The outer clamp is applied once at the outermost SELECT; pushdown filters are applied per `smelt.<path>` reference in the expanded body.
6. **Per-partition equivalence with full refresh, up to full-refresh non-determinism.** For every partition `p` in the run window, the partition-grain output `where(partition_column = p)` equals the full-refresh output for `p` on all local, deterministic columns; a `columns.<c>.contract: plausible` column need only be a plausible full-refresh value; globally-dependent columns are not equivalent (Semantics §"Per-partition equivalence").
7. **Idempotence under fixed input.** Re-running the same run window on unchanged sources converges to the same output table state.
8. **Granularity is closed under partition arithmetic.** A run window must align to whole granularity units; partial-unit ranges are rejected. The declared granularity must also be at least as coarse as the granularity independently derived from the `partition_column` projection's own truncation transform (`g_run >= g_part`); a declared granularity finer than the derived partition grid is rejected (Semantics §"Run window vs partition granularity").
9. **Safety-check overrides are explicit.** A `safety_overrides` entry names the specific check it bypasses; there is no global disable.
10. **No silent downgrade to full-refresh.** A model the safety classifier rejects, or whose bound derivation is `NotDerivable`, is refused at planning time with a diagnostic, never a silent fall back to full-table execution (§"Validator, not chooser").
11. **`event_time_column` must be accessible at the outermost SELECT, unless every UNION ALL branch traces `Traceable`.** Otherwise `EventTimeColumnNotVisibleAtOuterSelect` (Error) fires at the diagnostic gate (Semantics §"Event-time outer-visibility").
12. **Non-determinism stays in the payload.** Non-deterministic SQL is admitted only when its value flows exclusively into a `columns.<c>.contract: plausible` column (except the run-nondeterministic class as a direct projection); it must never reach `event_time_column`, `partition_column`, a `unique_key` column, or any membership/grouping position. Declaring an excluded column `plausible` is a configuration error.

### Key-grain constraints

1. **Opt-in is `refresh: incremental` + `grain: key`** (storage implied `table`); `unique_key` is required and must restate the `GROUP BY`. No config block; `safety_overrides:` is a hard error (partition-grain only).
2. **A `timeseries:` block is admitted iff key temporal locality is established** (§Semantics "Key temporal locality"); otherwise it is refused (`KeyedForbidsTimeseries`).
3. **The body is an aggregated `GROUP BY` query; `unique_key` is derived from `GROUP BY`; every non-key projection classifies into exactly one column family.** The combiner is a fixed lookup; authors never declare combiners.
4. **The catalogue is closed and the classifier is fail-closed.** Unrecognised aggregators, composite expressions, unproven once-write columns, and retractable contributions are refused — never approximated, never silently downgraded (§"Validator, not chooser").
5. **End-state equivalence holds with the model's own SQL as the oracle**, with exactly two named carve-outs: retained departed keys under snapshot-reconcile, and ordering-key ties on overwrite columns.
6. **No write-eligibility clamp.** A run merges every delta row it scans; no scanned input is silently dropped. Target-scan slice pruning under established key temporal locality is no-op elimination (or a transactionally-checked declared bound), never a write clamp. Any future clamp or settled-key GC must ship together with late-fact accounting.
7. **The run shape is derived from the driving source** (clocked ⇒ window-forward; unclocked ⇒ snapshot-reconcile) and surfaced by `smelt explain`; it is never declared.
8. **The admission matrix is enforced per column.** Fold and once-write families require a clocked (replayable) driving source; the plain-overwrite family requires the snapshot posture.
9. **Window-forward models maintain the transactional merge ledger**, written atomically with each window's merge. Additive-fold models must refuse a ledgered window's re-run; re-run-tolerant models may re-merge. Snapshot-reconcile models keep no ledger.
10. **Ordering and parallelism follow the derived postures.** Out-of-order/parallel/sliced backfill only for order-independent models; overwrite columns force sequential temporal order.
11. **Reprocessing changed input is refused for every family** when detected; the mitigation is `--full-refresh` (or a manual cascade rebuild).
12. **Exactly one clocked driving source under window-forward.** Zero clocked sources selects snapshot-reconcile; two or more is refused.
13. **Without an admitted `timeseries:` block the output has no `partition_column`** and downstream consumers treat the keyed table as a lookup. With one, the output is a clocked, time-partitioned keyed table (§Semantics "Key temporal locality").
14. **The windowed step loop is the shared driver**, not a per-pattern copy (`model_transforms.md`).
15. **Key temporal locality is established only by the three named routes** (key-embedded, key-determined, recurrence-bounded). Derived routes prune by proof; the declared route prunes only under the transactional runtime check (`KeyedRecurrenceBoundViolated`). A violated declaration fails the run; it never silently drops.

### Interval-versioning constraints

1. **`versioning: interval` is admitted only on `grain: key`.** No `materialized_view` restatement; the opt-in implies `table` storage (inherited from the key grain).
2. **No `timeseries:` block on the model itself, together with `versioning: interval`.** Keyed + interval output; not a partitioned build. Window-forward consumption of a `timeseries:` *source* is derived and in-bounds (§"Input consumption").
3. **Validity intervals are non-overlapping per key.** At most one open (`is_current`) version per key at any time; closed intervals abut at shared boundaries with no gaps.
4. **Validity is stamped from source event-time, never the run clock.** This is what makes the profile order-independent and replay-safe.
5. **End-state equivalent and order-independent** (§"The equivalence invariant"). Merging non-overlapping snapshots in any order converges to the same version history.

## Known Divergences / Open Questions

### The contract, plan, and graph layer

- **The grain-demotion has landed for the top-level surface (one narrow gap remaining).**
  This spec makes the shape-defining facts (`timeseries:` / `unique_key:`) the declared surface and
  `grain:` a derived check-only assertion (§"The declared shape axis"). Top-level `unique_key:` now
  parses (`.sql` frontmatter and `smelt.yml` model overrides, frontmatter wins); `refresh: incremental`
  is admitted on the facts alone (no `grain:` required), and a written `grain:` is validated against
  `derive_grain(clock?, identity?, partition_column ∈ key?)` whenever a top-level `unique_key:` is
  declared, erroring on mismatch and naming both labels. The narrow gap: a `grain: key` model with no
  top-level `unique_key:` (identity derived from the SQL body's own `GROUP BY` instead) is checked
  against that derived key only at plan derivation (`smelt-db::queries::maintenance`), not at the
  earlier frontmatter-validation step — and only when a top-level `unique_key:` is also declared to
  check it against; a bare `grain: key` model with neither declaration is unchanged (`models.md`
  §Known Divergences).
- **The open write-pattern registry, the `maintenance.cells[].write` pin, and both write-addressing
  diagnostics are built; the equivalence-invariant factor the registry consults is still the
  structural contract-facts check only.** The registry (`smelt_logical::maintenance::
  WRITE_PATTERN_REGISTRY`) declares each pattern's required contract facts and backend-capability key;
  `resolve_write_pin` implements the available-addressings rule's first, second, and fourth factors
  (declared facts × the pattern's requirements × backend capability, sourced from
  `BackendCapabilities` via the project's declared target backends) and refuses fail-loud —
  `MaintenanceWritePatternUnavailable` for an unrecognised name or a capability gap, never
  `MaintenanceWriteAddressingRefused` for one — never a silent downgrade to a substituted technique.
  `maintenance.cells[].write` parses as an open string (`smelt_core::config::MaintenanceCellConfig`),
  not a sealed enum. What is not yet consulted: the third factor (a per-cell equivalence proof beyond
  the pattern's declared required facts — e.g. threading `P3` column-comparability or a
  suppression-specific proof into the pin's own equivalence check) is a caller-supplied hook
  (`resolve_write_pin`'s `cell_can_uphold_equivalence` closure) that today always accepts; deepening it
  is later work, tracked alongside this entry. `supports_column_scoped_merge` migrated from the
  `Backend` trait into `BackendCapabilities` (`multi_backend.md` §Known Divergences narrows the
  remaining two flags in that section). Design derivation:
  `docs/research/20260716-relation-contract-and-per-cell-addressing.md`; the Relation Contract that
  reframes the declared facts is `models.md` §"The Relation Contract".
- **Observed-delta recording is built for the change-suppressed column-scoped MERGE family
  (§"The graph layer" — "Observed deltas on model edges"); its key→partition projection into
  forward propagation is built for a composed model edge, and both are surfaced by `smelt
  explain`; backward resolution and the keyed-fold/staged-candidate write families' own recording
  are not.** A change-suppressed column-scoped MERGE records its changed-row set — the same `IS DISTINCT FROM`
  comparison predicate that guards the write's matched arm, restricted to comparable columns only —
  into a warehouse-resident table alongside the reconciliation ledger, in the same backend
  transaction as the write itself: a failed write leaves no delta row, and a fully-suppressed run
  records a present-but-empty delta, distinct from no record at all. Recording is scoped to DuckDB
  today, matching the reconciliation ledger's own DuckDB-only posture. A composed model edge's
  recorded delta projects to exact partition-day intervals via the model's own established key
  temporal locality route (§"Key temporal locality", "Row movement") — routes 1–2 (key-embedded,
  key-determined) project the touched partitions exactly, since under those routes a key's
  partition value never changes; route 3 (recurrence-bounded) widens each touched partition
  backward/forward by the bound `r` plus lateness/skew margins the route's own scan clamp already
  carries, for **both** its sub-routes — the statically-derived sub-route and the declared
  sub-route alike — since under route 3 a key's partition value may move (an extremal or overwrite
  partition projection superseded by a later row), regardless of whether the recurrence bound `r`
  licensing that movement was proven or declared. An empty recorded delta projects to nothing (the graph half of the no-op
  cascade — a fully-suppressed run's downstream has nothing to do); an absent record (no
  conditional write has ever run for that window) still falls back to the full written window, the
  always-correct widen-never-narrow default. `smelt explain <model>` surfaces both halves as
  static facts about the derived plan: a `ColumnScopedMerge` cell's block prints an
  `observed-delta recording:` line reading `yes` only when that cell's own row identity is
  proven (`region key: Key(...)`, never `WholeRow`) and every compared column is proven
  comparable across runs — the same two fail-closed proofs write suppression itself requires
  (`model_properties.md` §"Change comparability"; a `WholeRow`-identity cell or an incomparable compared column always
  falls back to an unconditional matched-arm rewrite and has nothing to record, so its line reads
  `no` instead) — and a composed model's `Key temporal
  locality:` block prints an `observed-delta projection:` line — `exact (key-embedded)` /
  `exact (key-determined)` for routes 1–2, widened by `r` plus margins for route 3 — alongside its
  route and settle bound (a bare keyed model, with no established locality, prints no projection
  line). This is a plan-level report only: `explain` never opens a backend connection, so it
  reports what a cell's technique *would* record and how its route *would* project, never a
  specific past run's actual recorded delta (that is `smelt run --since-upstream`'s own read
  path, not `explain`'s) — so the settle-bound × observed-delta composition named in §"What the
  composed shape uniquely enables" is `smelt explain`-visible on its static shape (route, settle
  bound, projection form) but has no live "is this slice's recorded delta actually empty right
  now" leg to compose with yet. What remains unbuilt: this projection is exercised by directly supplying
  an already-fetched observed-delta lookup to the propagation assembly — reading the real
  warehouse-resident table live during `smelt run --since-upstream` itself is not yet wired into
  the CLI path; backward resolution does not yet consume a recorded delta at all (every ancestor
  requirement is still the full clamp-derived slice); the keyed-fold and staged-candidate write
  families do not yet record, so their cells print no recording line at all. Tracked by
  `docs/plans/20260715-composed-axes-conditional-maintenance.md` (wiring `--since-upstream`'s live
  read path; and, for external sources, the M3-input fingerprint-sidecar variant in §Future
  Extensions, whose lifecycle — naming, storage, transactionality, GC, invalidation — is now
  normative in `sources.md` §"The fingerprint sidecar").
- **No execution technique keys off a maintained-model creation cell.** §"Upstream model edges"
  is otherwise live: the per-model derivation `smelt explain` reports and the forward-propagation
  graph (`crates/smelt-runtime/src/propagation.rs::build_forward_graph`) both resolve a
  maintained-model upstream through the SAME edge-aware derivation
  (`derive_model_maintenance_plan_with_edges`), so the propagation clamp for a model edge equals
  the creation cell's clamp and an underivable upstream clock is a `MaintenanceReachNotDerivable`
  refusal (contributing no walkable edge) rather than a silently permissive whole-table synthesis;
  and `--source <address>` accepts either a declared source or an upstream maintained model as the
  delta origin (the origin model itself is never re-run — its landed delta is the window a
  completed run already wrote for it). What remains is the execution side: `execute.rs`'s technique
  resolution excludes model refs entirely, so a maintained-model creation cell drives forward
  propagation and `smelt explain`, but no per-cell *execution* technique keys off it yet (the
  propagated region is materialized by the ordinary incremental run loop over the reflected
  window). Tracked in `docs/plans/20260710-web-analytics-maintenance-demo.md`.
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
  the eight `Maintenance*` diagnostics — `MaintenanceNoAdmissibleTechnique` and
  `MaintenanceScanUnbounded` — into `file_diagnostics()` (see `diagnostics.md` §Known
  divergences for the other six, including the two write-addressing codes that guard the unbuilt
  `write:` pin). `smelt explain <model>` reads the same derivation (via
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
- **Statement emission is single-owner for the region-recompute, keyed-fold, and
  column-scoped-MERGE families, and both the conformance gate and `--show-sql` are wired to
  prove/print it.** The region `DELETE`+`INSERT` pair (`IncrementalStrategy::DeleteInsert`) is
  produced by `emit_delete_insert` in `crates/smelt-logical/src/maintenance/emit.rs` and
  executed, never authored, by the backends (`smelt-backend`'s `execute_statement_group`,
  overridden by `smelt-backend-duckdb` for a real transaction and by `smelt-backend-spark` for
  its catalog-qualified table name). The keyed fold `MERGE` (combiner-aware `UPDATE SET`,
  `INSERT *`) and the windowed-keyed-maintenance driver's first-run `CREATE TABLE … AS` are
  likewise produced by `emit_keyed_fold`/`emit_create_table_as`; the caller
  (`smelt-runtime::cumulative`) renders each aggregator column's `CrossPartitionCombiner` to a
  plain expression string before calling the emitter, keeping `smelt-logical` free of any
  dependency on `smelt-planner`. The column-scoped `MERGE` (`Technique::ColumnScopedMerge`) is
  produced by `emit_column_scoped_merge` — `UPDATE SET *`/`INSERT *`, dialect-keyed but
  currently dialect-invariant text (DuckDB and Spark's pre-unification `merge_into` text were
  byte-identical) — with `Backend::merge_into`'s default implementation building the
  `StatementGroup` and routing it through `execute_statement_group`; no backend overrides
  `merge_into` any more. Every family's `crates/smelt-runtime/tests/statement_parity.rs` leg
  diffs a real `execute_project` run's executed statements against a direct emitter call over the
  same inputs (the column-scoped-MERGE leg re-runs `examples/timeseries/daily_events_enriched`'s
  fact+dimension fixture through a recording backend). The ledger-graded (`Grade::Additive`) fold
  path still interleaves the emitted action statement with the reconciliation ledger's own
  DDL/DML via `Backend::fold_ledger_delta`, unchanged — that interleaving is spec-excluded
  bookkeeping, not itself a maintenance statement. `crates/smelt-runtime/tests/statement_parity.rs`
  additionally carries a structural gate
  (`no_maintenance_statement_authoring_outside_the_emitter`) that scans every production `.rs`
  file for the forbidden `DELETE FROM`/`MERGE INTO`/`UPDATE … SET` shapes outside the emitter
  module, allowlisting only the DuckDB/Spark `DELETE` strings the dead
  `delete_partitions`/`insert_overwrite` paths still hand-format (tracked below); every other file
  that matches is a hard test failure. The conformance suite's technique-equivalence legs
  (`crates/smelt-logical/tests/maintenance_plan_conformance.rs`) prove the *emitters* equivalent to
  full refresh (the HOLDS legs) and each such leg's doc string additionally names the
  `statement_parity.rs` case that grounds the same family's production-execution byte+result
  parity, so the two suites together close the loop from "the emitter is correct" to "the emitter
  is what actually ran". `smelt explain <model> --show-sql` (`cli.md` §"`smelt explain <model>`
  maintenance-plan report") prints each cell's statements by calling the same emitters `smelt run`
  calls. What genuinely remains open: `emit_in_place_update`
  (`crates/smelt-logical/src/maintenance/emit.rs`) has no production consumer — no live plan cell
  lowers to it, so its own leg exists only in `crates/smelt-logical/tests/maintenance_tracer.rs`
  and `crates/smelt-runtime/tests/tracer_maintenance.rs` (the schema-evolution column backfill's
  `UPDATE … FROM` in `smelt-runtime::backfill` is a separate, untouched surface); the
  `Grade::Additive` keyed fold's MERGE-inside-the-ledger-transaction interior
  (`Backend::fold_ledger_delta`) is not observable at `execute_statement_group`, so its
  `statement_parity.rs` leg proves parity against a self-contained idempotent keyed fixture rather
  than a real Additive-graded model (e.g. `examples/web_analytics`'s `device_user_edges`); and
  `Backend::delete_partitions`/`insert_overwrite` (DuckDB and Spark) still author a
  hand-formatted `DELETE`+`INSERT`/`INSERT OVERWRITE` shape for `IncrementalStrategy::InsertOverwrite`
  even though that strategy is unreachable in production — `resolve_incremental_strategy` and every
  backend's `resolve_strategy` only ever yield `DeleteInsert`, so `insert_overwrite`/
  `delete_partitions` are dead code, allowlisted in the structural gate with justification
  comments rather than emitter-backed. Deleting the dead `InsertOverwrite` strategy (or routing it
  through an emitter if it is ever revived) is follow-up work outside
  `docs/plans/20260710-emit-unification.md`.
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
  `PRIMARY KEY` and refuses the run (`KeyedReprocessedWindow`, §"Reprocessing") before the
  action ever runs a second time. An idempotent-only cell never
  creates this table — only an additive-graded cell needs never-fold-twice enforcement. The
  DuckDB-dialect DDL/DML is the only ledger substrate implemented today; an additive-graded cell
  on a non-DuckDB backend fails loudly (`UnsupportedFeature`) rather than being handed
  DuckDB-flavored SQL it cannot run — a Spark-dialect ledger builder is unbuilt.
- **A bare keyed-grain hop still refuses in the graph; a locality-admitted composed node no
  longer does.** A `grain: key` node with no admitted key-temporal-locality verdict (no
  `timeseries:` declared, or one declared but not admitted — §"Key temporal locality") still
  refuses fail-loud (`MaintenanceGraphUnsupportedNode`, P7/P8), with a message naming the
  missing time axis and the composed-shape fix. A `grain: key` node whose locality gate *did*
  admit is classified by its declared `timeseries.granularity` instead — a clocked node that
  contributes edges like any other node (§"The graph layer": "A locality-admitted
  time-partitioned keyed output is not refused"), rather than `PartitionGrain::Keyed`
  (`crates/smelt-runtime/src/propagation.rs::build_forward_graph`,
  `crates/smelt-logical/src/maintenance/propagate.rs::refuse_keyed_nodes`). Time-unrolled
  self-edges are still designed and unbuilt (`10-dependency-propagation.md` §6). The composed
  node's own key→partition dirt projection is now derived, route-aware, for the inbound edge
  (from its own driving source): exact under locality routes 1–2 (a per-key-constant partition
  value projects to its own partitions, no widening —
  `smelt_logical::maintenance::propagate::locality_margin_days`), widened backward by `r` plus
  the derived lateness/skew margins under route 3, reading the SAME admitted `KeyLocality`
  verdict the plan and `smelt explain` already carry (`crates/smelt-runtime/src/propagation.rs::
  build_forward_graph`). The composed node's outbound edge (to its consumers) was never a
  placeholder — it derives through the ordinary per-cell `ScanClamp` path any downstream reader
  contributes, unchanged by this narrowing. No key-level dirt representation exists anywhere;
  intervals stay the graph's only currency (`10-dependency-propagation.md` §6, S12 — a
  key-addressed dirt-set representation remains designed but unbuilt for any node this interval
  projection cannot cover). The CLI loop is now closed end to end: `--source <address>` accepts
  a locality-admitted composed model as the delta origin exactly like any other maintained-model
  origin (the origin is never re-run; its landed delta reflects through the composed node's own
  outbound edge), and `smelt build --include-upstreams` walks *through* a composed node as an
  ordinary ancestor in its build order. Reaching this required threading a declared
  `key_recurrence` bound into the graph-layer's own per-model derivation
  (`crates/smelt-runtime/src/propagation.rs::build_forward_graph` now builds the same
  `(bare source name, key_recurrence)` list `smelt-db`'s `derive_model_maintenance_plan_with_edges`
  call site does, via `smelt_db::queries::maintenance::build_key_recurrences`) — previously this
  call site passed an empty list unconditionally, so a route-3 declared-sub-route composed node
  (the flagship `examples/web_analytics/models/silver/events_deduped.sql`) never established
  locality in the graph layer at all and its bare `PartitionGrain::Keyed` classification made
  `refuse_keyed_nodes` fail-loud refuse any graph containing it, origin or not. A **bare** keyed
  model named directly as a `--source` origin now also refuses fail-loud with the same "without
  an admitted time axis" message even when it has no edge in the assembled graph (an isolated or
  edge-less bare keyed origin was previously a silent no-op instead of a refusal, since the
  edge-only `refuse_keyed_nodes` check never visits a node with no edge touching it). What
  remains open: the real `examples/web_analytics` workspace is not yet fully
  `--since-upstream`-compatible end to end — `silver.sessions_chained` (a self-referential
  recursive-accumulation model) and `silver.device_user_edges` (a bare keyed model with real
  downstream readers) each independently refuse the whole-workspace graph today, since
  `--since-upstream`/`--include-upstreams` build the graph over every discovered model
  unconditionally with no `--select` scoping — both are the same pre-existing, explicitly
  deferred limitations named above (time-unrolled self-edges; bare keyed dirt-sets), not new.
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
- **Key temporal locality: all three routes and their slice-pruned merge (route 3's checked)
  are built, the admitted slice and derived settle bound are folded into `smelt-db`'s own
  plan-derivation surface, and `smelt explain` prints the route/slice/settle bound; the
  broader per-input scope-map explain surface (§"Scope maps") is specified but unbuilt.** The
  locality gate (§"Key temporal locality (the time-partitioned output)") checks the structural
  preconditions (window-forward run shape, a provably NOT NULL partition column, matching
  granularity) and then admits via either route: route 1 when `partition_column` is itself a
  `unique_key` column (the derived slice is the run step's own partition value, widened by the
  driving source's derived read margin); route 2 when `partition_column` is proven a per-key
  constant by a declared `functional_dependencies:` entry naming it — in which case the derived
  slice is the run step's own delta relation's partition-column values, with no margin widening.
  Route 2 deliberately does **not** auto-admit from a bare query-derived functional dependency
  (the model's own `GROUP BY` key subsuming the declared `unique_key`, with no declaration): that
  proof only establishes the column is a deterministic function of the key *within one fixed
  computation*, which does not distinguish a genuinely once-write shape from an extremal-fold
  (`MIN`/`MAX`) combiner — a combiner whose folded value a later, out-of-order redelivery can
  still change on re-merge, so it is a different family from once-write provenance (§"The
  algebraic maintenance ladder"; "Row movement" — only route 3, not route 2, may see a partition
  value move). A `MIN`/`MAX`-derived partition column is refused by route 2 even when a
  `functional_dependencies:` entry names it: a declaration can widen only a genuinely undecidable
  origin, never override the walk's own proof that the column is an extremal-fold combiner.
  Either admitted slice is carried as a target-scan predicate on the keyed `merge_into`'s `ON`
  condition (a literal range for route 1, an `IN (SELECT DISTINCT … FROM (delta))` subquery for
  route 2), pruning which stored rows the merge needs to match without changing which delta rows
  merge. A model satisfying none of the three routes refuses with the three-route
  `KeyedForbidsTimeseries` message.

  Route 2's declared-FD sub-route is reachable only when the determined column's non-nullness is
  itself provable: the shared NOT-NULL derivation
  (`smelt_logical::analysis::not_null::partition_column_provably_not_null`) recognises only
  driving-clock-derived shapes — the column is itself a `unique_key` column, a direct `MIN`/`MAX`
  aggregate over the driving source's own clock column, or a direct scalar wrapper
  (`DATE_TRUNC`/`CAST`) around it. A declared functional dependency naming an arbitrary dimension
  column that is not derived from the driving source's clock (e.g. a plain enrichment column with
  no relation to the model's event-time axis) fails the NOT-NULL structural precondition before
  the functional-dependency check ever runs, so the declared-FD sub-route is real for a
  clock-derived determined column but not yet reachable for an arbitrary declared non-null
  dimension column — extending the NOT-NULL derivation (or introducing a dedicated non-null column
  declaration) to cover that case is unbuilt.

  Route 2's real-fixture coverage is unit- and driver-level rather than a full
  `execute_project`-driven DuckDB fixture: the keyed classifier's aggregator allowlist
  (`combiner_for`) admits only the additive-fold and extremal-fold families (moot for route 2
  today in any case, since a `MIN`/`MAX`-derived column is refused by route 2 on family grounds
  regardless — §"Route 2 deliberately does not auto-admit" above). A grouped extremal aggregate
  (`MIN`/`MAX` under a `GROUP BY`) over a provably NOT NULL argument infers NOT NULL
  (`types.md` §11 "Nullability"), so a `MIN`/`MAX`-derived `timeseries.partition_column` satisfies
  the NOT-NULL precondition (`incremental_models.md` §Diagnostics) whenever the folded argument is
  itself NOT NULL. Building a runnable route-2 fixture end-to-end needs the once-write classifier
  family (tracked by `docs/plans/20260705-keyed-collapse.md`) to produce a determined column that
  is both clock-derived-NOT-NULL and genuinely once-write — out of this plan's scope. The derived slice
  and settle bound are folded into `smelt-db`'s own plan-derivation surface (`MaintenancePlan`)
  and printed by `smelt explain`; the broader per-input `smelt explain` scope-map rows
  (§"Scope maps" — the full per-input dispatch table, not just the locality verdict) remain
  unbuilt. `smelt-db`'s plan derivation still admits routes 1–3 only where it can determine the
  driving source's granularity from either a declared source or a referenced upstream model's own
  admitted composed output; the runtime execution path always can.
  Design derivation: `docs/research/20260705-keyed-time-superset.md`.

  Route 2's slice-pruned merge (the `IN (SELECT DISTINCT <partition_column> FROM (<delta_select>))`
  target-scan predicate this section names above) is unexercised against a real backend even in the
  once-write composed pool recipe family (§"Tests" below): every merge step in that family's
  route-2 driver runs with the slice predicate omitted (`slice: None`), because passing the real
  predicate makes DuckDB refuse to bind the `MERGE` at all — `Invalid Input Error: BindMerge -
  expected to find an operator of type LOGICAL_GET but got FILTER` — for *any* `ON` clause that
  combines a `USING (<subquery>)` with an `IN (SELECT …)` predicate, independent of whether the
  delta is a `VALUES` literal or a real table and independent of `DISTINCT`; confirmed directly
  against the `duckdb` CLI (v1.4.4 and v1.5.4). This is a genuine DuckDB `MERGE` binder limitation,
  not a defect in the emitted predicate shape or in the test's own construction of it — the identical
  `ON`-clause subquery form fails to bind even when both sides of the `MERGE` are ordinary tables.
  The pool's route-2 driver still exercises the real merge mechanics it asserts (write-once
  `pdate`, additive `total`) against real DuckDB; only the target-scan pruning optimisation itself
  goes unexercised there (§"Key temporal locality": "pruning is not a write clamp" — every delta
  row still merges with or without it, so omitting the predicate does not change the asserted
  equivalence). Lifting this needs either a DuckDB-side fix/workaround upstream of
  `smelt_logical::maintenance::emit::emit_keyed_fold` (e.g. rewriting the `IN (SELECT …)` predicate
  to a form DuckDB's `MERGE` binder accepts, such as a pre-materialized semi-join) or a demonstrated
  DuckDB version where the binder limitation no longer applies; tracked by
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.

  **Route 3 (recurrence-bounded)** is built: a statically-derivable `r` (the same lookback-margin
  derivation route 1's window slice uses) admits as an ordinary, unchecked window slice; a
  driving-source-declared `key_recurrence` whose `key` exactly matches the model's own `unique_key`
  admits as a **checked** slice — every merge step first runs a read-only out-of-slice match probe
  (a single `COUNT`/sample-keys query joining the target against the step's own delta) before the
  merge itself, and a violation refuses the run with `KeyedRecurrenceBoundViolated` (naming the
  violation count and sample keys) without ever writing to the target; a derived `r` never runs
  the probe. Both the probe and the merge are single-owner-emitted
  (`smelt_logical::maintenance::emit::{emit_recurrence_bound_probe, emit_keyed_fold}`) and covered
  by the `statement_parity` gate. Route 3's routine unit- and driver-level coverage manually
  builds the classification and drives the windowed-keyed-maintenance driver directly rather than
  through the full `execute_project` pipeline (its flagship shape is also an extremal-fold
  (`MIN`/`MAX`) partition column — now NOT NULL under the grouped-extremal rule above). An
  `execute_project`-driven route-3 fixture also exists: the web-analytics tracer's composed
  `events_deduped` model, driven through the real run pipeline with a redelivery-storm re-run
  proving both the doubly-predicated `MERGE` text (the recurrence-bounded slice on the target
  read, the write-suppression arm on the matched clause — see below) and a zero-row write.
  The declared-vs-derived
  precedence order (derived tried first) and the
  order-independent key-set comparison for the declared fallback are implementation choices this
  plan made where the spec text underdetermines them.

  The slice-pruned merge is the
  prerequisite every bullet of §"What the composed shape uniquely enables" builds on. All three
  bullets are realized: the two graph-layer ones (propagation admissibility, key→partition dirt
  projection), and slice-bounded write suppression — a composed (key + time) output's
  suppressed `MERGE` carries the locality slice on the target read and the `IS DISTINCT FROM`
  suppression arm together, keeping compare cost proportional to the slice rather than the full
  key space; a bare keyed model's suppressed `MERGE` carries the suppression arm alone, never an
  invented slice. The settle-bound × observed-delta composition remains
  unbuilt, tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **`grain: key_per_partition` derives no plan yet.** The value parses and passes declaration
  validation, but maintenance-plan derivation has no trajectory/backfill machinery to back the
  per-`(key, partition)` shape, so a `refresh: incremental` model declaring it refuses fail-loud
  at plan derivation (`MaintenanceUnsupportedGrain`, naming the grain and the tracking plan) —
  no cells are derived and no executor runs. Full trajectory support (the locality routes, a
  real emitted plan, and graph-layer admission of the shape) is tracked by
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **Conditional maintenance technique: column-scoped and keyed-fold MERGE, plus a merge-less
  keyed realisation; the region DELETE+INSERT family and the whole-row merge-less realisation
  remain unbuilt.** §"Windowed maintenance and the horizon" category 2 (no-op write elimination)
  is now partly built: both the column-scoped `MERGE` (`Technique::ColumnScopedMerge`) and the
  keyed-fold `MERGE` admit a change-suppressed matched arm (`AND (target.c IS DISTINCT FROM
  source.c OR …)` — the keyed-fold variant compares the stored value against the fold's own
  combine expression rather than a plain source column) that writes zero rows for an
  unchanged-input re-run — admission is fail-closed over the P2 row-identity verdict (a
  `WholeRow` cell never suppresses) and the P3 per-column change-comparability verdict (one
  `Incomparable` column in the group refuses the whole cell's suppression, falling back to the
  pre-existing unconditional matched arm). For a backend that cannot run `MERGE` at all, the
  keyed-identity **staged-candidate conditional DELETE+INSERT**
  (`smelt_logical::maintenance::emit::emit_staged_candidate_conditional`) realises the same
  no-op-write-elimination as one transaction (stage the candidates, conditionally `DELETE`+
  `INSERT` only the rows whose effect is not the identity, `DROP` the staged relation) —
  `maintenance::choice::resolve_keyed_write_mechanism` chooses between the keyed `MERGE` and this
  mechanism purely from a backend-capability flag, never a silent substitution on a
  `MERGE`-capable backend. That choice is wired into the live `refresh: keyed` per-partition
  execution loop (`smelt-runtime::cumulative`): the loop resolves each cell's `WriteSuppression`
  once per run, from the same P2 row-identity and P3 change-comparability facts the column-scoped
  path uses, and dispatches the keyed-fold `MERGE` to its suppressed or unconditional matched arm
  accordingly — composing with a locality-admitted model's target-scan slice unchanged (both
  predicates land on the same `MERGE`, never one displacing the other). `smelt explain <model>
  --show-sql` does not yet reflect any of this: it always renders a `ColumnScopedMerge`/`KeyedFold`
  cell's unconditional matched arm, never the suppressed form the live run actually executes for a
  cell that resolves `Suppressed` — the reporting path hasn't been wired to the same
  `resolve_write_suppression` check the executor already runs. Still unbuilt: the region
  `DELETE`+`INSERT` family has no conditional
  variant yet (every region overwrite still rewrites unchanged rows), the whole-row (keyless,
  `EXCEPT ALL`-both-ways) staged-candidate realisation does not exist, a `write:` pin over the
  keyed `MERGE`/staged-candidate choice does not exist, and no observed output delta is recorded
  anywhere except a maintained-model edge's own conditional write. A maintained-model edge's
  region recompute (creation-trigger cell, `Technique::DeleteInsert`) now **does** restrict its
  own compute to an observed delta where licensed: `maintenance::derive::append_model_edge_cells`
  derives the P1 skeleton-source-closure verdict shared by every model edge of a downstream model
  (proving every OTHER edge's enrichment join preserves the driving edge's row skeleton —
  `model_properties.md` §"Skeleton-source closure"), `maintenance::choice::
  resolve_recompute_restriction` admits the delta-restricted variant only when that verdict is
  `Closed` *and* the driving edge's own observed delta (Group D, T5) is present and non-empty, and
  `maintenance::emit::emit_delete_insert_delta_restricted` emits the semi-joined `DELETE`+`INSERT`
  — byte-identical to the ordinary widened-scan `emit_delete_insert` whenever either factor is
  absent, never a partial restriction. `smelt_runtime::maintenance_driver::
  execute_delete_insert_with_delta_restriction` reads the recorded delta and dispatches between
  the two emitted forms against a real backend, and `execute_project`'s own per-batch execution
  loop (`crates/smelt-runtime/src/execute.rs`) now dispatches every model-edge-sourced,
  `DeleteInsert`-strategy creation cell (over an already-materialized target, on a DuckDB target)
  through this path — both the live executor and the `--dry-run`/`smelt explain` reporting branch
  route through the same `resolve_live_delta_restriction_facts` derivation and `build_delete_
  insert_group_dispatched` decide-and-emit call, so a real `smelt run` actually restricts recompute
  breadth when P1 closes and an exact delta exists, not only a direct call of the executor. This
  restriction is licensed independently of write suppression (Group C) — it narrows recompute
  *breadth*, never what is scanned into `S` — and today only reaches a maintained-model-edge
  driving source: an external `mutable_snapshot` source's own synthesized delta now has an exact
  form available (M3's input-fingerprint sidecar is built for DuckDB — see the sidecar paragraph
  above), but `execute_delete_insert_with_delta_restriction`'s own admission does not yet consume
  it as a driving-source delta; a non-DuckDB target
  keeps the ordinary widened-scan region recompute unchanged (the observed-delta read is
  DuckDB-only, so the live dispatch falls back rather than reaching for a capability that target
  doesn't have). Mechanisms and sequencing:
  `docs/research/20260715-conditional-maintenance-without-cdf.md`;
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **User docs describe the trichotomy + grain surface; the plan's own CLI surface is now partly
  covered.** The `docs-site/` pages consistently describe
  `refresh: full | incremental | materialized_view` and `grain: partition | key |
  key_per_partition`, seeded from the worked example catalogue
  (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`).
  `docs-site/docs/reference/cli.md` now documents `--since-upstream` (forward propagation) and
  `--include-upstreams` (backward resolution) under `smelt run`/`smelt build`, plus
  `smelt explain`'s cell/clamp/ledger report and `--show-sql`; the `maintenance:` frontmatter
  block is documented in `docs-site/docs/reference/smelt-yml.md`. What is still not yet
  covered — because the underlying surface doesn't exist yet — is `smelt bakeoff`.
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

### The partition grain

- **The mode value is cut; the sub-block remains.** `refresh: batched` is a hard error with a fix-it naming `refresh: incremental` + `grain: partition` (`crates/smelt-core/src/config.rs`); the `batched:` sub-block (`batched.unique_key`, `batched.nondeterministic_columns`, `batched.safety_overrides`) is still the live surface for those options and is refused without `refresh: incremental` + `grain: partition` (`crates/smelt-core/src/metadata.rs`). Top-level `unique_key`/`safety_overrides` do not yet parse; `columns.<c>.contract` does (`models.md` §Known Divergences). The `smelt migrate` assist does not exist. Delivered/tracked by `docs/plans/20260707-maintenance-plan-impl.md`.
- **`nondeterministic_columns` predates `columns.<c>.contract`.** The pre-cut `batched.nondeterministic_columns` list and the target `columns.<c>.contract: plausible` declaration are the same mechanism under two surfaces; the column-scoped `contract` key is owned by `models.md` §"`columns:` — column metadata" (semantics: this spec). The `columns.<c>.contract` key parses today; the pre-cut list form remains the live surface inside the `batched:` sub-block (previous divergence).
- **Diagnostic-code and config-type spellings still carry the pre-cut mode names.** The diagnostic codes (`TimeseriesRequiredForBatched`, `BatchedNotSafe`, `KeyedForbidsBatched`) and config types (`BatchedConfig`, `BatchedSafetyOverrides`) retain the retired "batched" spelling, and `crates/smelt-logical/src/rules/incremental.rs` carries the rule module. A pure internal rename is deferred. (An earlier divergence entry also listed a `CumulativeForbidsBatched` code; it no longer exists — the two-peer-mode conflict it guarded is no longer expressible now that `grain` is a single enum value.)
- **One non-hot classification call site still reads the outer SQL body.** The bound-`NotDerivable` refusal gate (`derive_model_source_bounds`, pure planner) classifies on the outer `model.sql`; a lookback living only inside a function body with no outer Form B filter is the sole case that would behave differently, and none exists in the repo. Tracked in `docs/plans/20260530-thread-fn-registry-classification.md`.
- **Window-function batch-safety check runs on unexpanded outer SQL.** `find_inadmissible_over` scans the outer model SQL before function expansion, so an `OVER` clause inside a `smelt.define` body is invisible to it. Tracked in `docs/plans/20260530-thread-fn-registry-classification.md`.
- **Per-source clamp observability partly emitted.** `smelt explain --json` reports `source_partition_col` and `(before, after)` offsets but does not yet resolve the run-relative scan window even when a run window is supplied; the editor-hover readout is not yet implemented (LSP hover is type/column/ref oriented). Both are specified ahead of a plan.
- **Per-column `data_latency` not implemented.** Late-arriving-data automation is deferred; the two interim mitigations (Semantics §"First-run and backfill") are the only options.
- **Non-deterministic row-set-membership or grouping is out of scope.** Always rejected regardless of `columns.<c>.contract`; reconciling frozen-per-window membership against a full refresh needs its own design (research §9.1a).
- **CTE-only `event_time_column` references not yet detected.** Constraint 11 is enforced for direct-subquery FROM clauses and set operations; a CTE alias that does not project `event_time_column` is not yet caught and fails at DuckDB execution. Tracked in `docs/plans/20260616-smelt-feedback-fixes.md`.
- **Three execution paths in `crates/smelt-cli/src/main.rs`.** Legacy, optimizer+batched, and batched-only paths are unified around `BatchedConfig` but the CLI dispatch is still tri-modal; should converge.
- **Schema evolution is unspecified.** A `partition_column` rename or output schema change has no defined handling today.
- **`smelt.metric()` interaction.** The interaction between metric expansion and time-filter injection is not fully spelled out for partition-grain models consuming metrics.
- **Generator-emitted partition-grain models are landed.** A `ModelDef` emitted by a generator (`meta_language.md`) may carry the partition-grain frontmatter and is subject to every rule here on equal terms. Per-`ModelDef` overrides are not part of the closed field set in v1. Tracked in `docs/plans/20260509-meta-language-overall.md`.
- **Diagnostic code ownership.** This spec owns the *semantics* of the diagnostic codes it lists; [`diagnostics.md`](diagnostics.md) is the cross-feature catalogue indexing severity and canonical trigger. The two must agree.
- **`g_run >= g_part` auto-coarsening is not implemented.** Today a sub-`g_part` run window is a hard rejection (Semantics §"Run window vs partition granularity"); the model author must correct `granularity:` or the run-window flags. A future enhancement could instead auto-coarsen the run window (or reject only with a suggested corrected value) rather than requiring a manual retry — deferred; hard-validation was chosen first as the fail-closed default.
- **Monotone-integer `partition_column` is recognised by the trace but not yet driven end to end.** The event-time monotonicity trace and the per-source bound/reach derivation both admit a monotone integer key (a constant `batch_id ± n` shift derives a source's lookback margin the same way a constant `INTERVAL` shift does). The run-window/backfill-chunking machinery and the per-source scan-filter injection are date-typed throughout (`run_start`/`run_end` are ISO dates, not integers), so a partition-grain model built entirely around an integer `partition_column` does not yet get an end-to-end run — the calendar-aligned run-window and DELETE+INSERT execution described in Semantics assumes a temporal partition grid. `smelt explain --json`'s per-source clamp rendering (Semantics §"Observing the per-source clamp") is also temporal-only today; it does not yet render an integer bound's magnitude. Tracked in `docs/plans/20260704-model-updates-l4-batched.md`.

### The key grain

- **The pre-cut surface is removed.** The surface described above (`refresh: incremental` + `grain: key`, top-level `unique_key`) is what parses today; `refresh: keyed` (like `refresh: cumulative`) is a hard error with a fix-it pointing at `refresh: incremental` with the matching `grain:` (`crates/smelt-core/src/config.rs`; `models.md` §Known Divergences). `KeyedForbidsBatched` remains live in one form: a `grain: key` model declaring a `batched:` sub-block is refused (`crates/smelt-core/src/metadata.rs`); the historical grain-conflict form (`refresh: keyed` + `refresh: batched` as peer values) is no longer expressible since `grain` is a single enum value. Delivered by `docs/plans/20260707-maintenance-plan-impl.md`.
- **The classifier covers only the direct-monoid families.** The classifier seed (`crates/smelt-logical/src/rules/cumulative.rs`, emitting the `Keyed*` diagnostic family), the windowed-keyed-maintenance driver (`crates/smelt-runtime/src/maintenance_driver.rs`), and the per-window `merge_into` execution (`crates/smelt-runtime/src/cumulative.rs`) admit only the additive-fold and extremal/lattice-fold families. The classifier union (overwrite, once-write, and plain-overwrite families) and the run-shape/posture derivation that distinguishes window-forward from snapshot-reconcile are unbuilt (decision record: `docs/research/20260705-keyed-collapse-application.md`; tracking plan: `docs/plans/20260705-keyed-collapse.md`).
- **The transactional merge ledger is built on DuckDB only.** Every additive-graded keyed-merge step folds its delta identity into a warehouse-resident per-delta ledger table in the same transaction as the merge (`smelt_backend::Backend::fold_ledger_delta`; DDL/DML in `smelt_state::ddl_duckdb`); a repeat delta violates the table's `PRIMARY KEY` and refuses the run (`KeyedReprocessedWindow`) before the action runs a second time. An idempotent-only cell never creates the table. The DuckDB dialect is the only substrate implemented; an additive-graded cell on another backend fails loudly (`UnsupportedFeature`) rather than being handed SQL it cannot run (§Known Divergences).
- **The snapshot-reconcile executor is unbuilt.** Until it lands, an unclocked keyed model (zero timeseries-tagged sources in the FROM clause) is refused fail-loud with a not-yet-supported diagnostic (`KeyedSnapshotPostureUnsupported`) naming the delivering plan — it is not treated as a model error.
- **The time-partitioned keyed output's admission, downstream pushdown, downstream keyed driving-source selection, and the `smelt explain` settle-bound surface are all wired.** Locality establishment (all three routes) and the `KeyedRecurrenceBoundViolated` runtime check are built (see the fuller bullet above); the admissibility decision lives in the single fail-closed locality gate in plan derivation (not a frontmatter shape check) that decides every keyed model's `timeseries:` block — a model satisfying none of the three routes refuses with `KeyedForbidsTimeseries`, naming all three routes and the nearest missing fact. The settle bound itself is derived by a pure per-route function (route 1/statically-derived route 3: the source lateness margin; declared route 3: the recurrence bound plus margins; route 2: honestly never) and threaded onto the derived `MaintenancePlan` for `smelt explain` to print (route, slice form, settle bound). A locality-admitted composed output is visible to the rest of the DAG exactly like a declared source: a downstream **partition-grain** model's compiled SQL carries the ordinary widened source-filter pushdown against it, and a downstream **keyed** model's driving-source resolution considers a referenced upstream model's own admitted composed output alongside declared `sources:` YAML entries — the clock propagates through the composed stage rather than stopping there either way. Design derivation: `docs/research/20260705-keyed-time-superset.md`.
- **Locality open questions.** Whether a derived recurrence bound can license slice pruning under snapshot-reconcile (v1: window-forward only); relaxing the granularity-equality precondition (e.g. a daily driver with weekly output partitions); slice-scoped deletion (`NOT MATCHED BY SOURCE` over a provably complete slice, e.g. re-dropped duplicates) — interacts with the key-deletion divergence below.
- **The pattern functions (`smelt.latest`, `smelt.once`, `smelt.current`) are unshipped**, as is the decision whether they are built-ins or template files; the canonical once-write spelling is fixed alongside them. Tracked in the keyed-collapse plan.
- **Driver granularity is `day`/`week` only** (`maintenance_driver.rs::driving_steps` refuses others) — a property of the shared driver inherited by every consumer; widening it is driver work, not profile work.
- **`--auto` staleness fidelity** for all-invertible models ("exactly the changed windows") needs the delta-history mechanism of the group rung; the v1 answer is conservative. Carried from the cumulative-era divergence list.
- **Self-referential keyed models** (`state += delta − decay`; the model joining its own target) are rejected — the self-reference is not an admissible input. An explicit input/state-distinction design would be needed to admit them. Carried.
- **Run-pinning alignment**: `NOW()`/`CURRENT_*` are rejected outright rather than compile-time-pinned as batched does; adopting the pinning transform here is a deferred alignment. Carried.
- **Key deletion is unresolved beyond retention.** Snapshot-reconcile retains departed keys; window-forward has no delete signal short of a change feed with delete events. Tombstones, opt-in hard delete, and the observer contract for the refused matrix cells are recorded as deferred in the decision record (§5 there).
- **Rungs 2–4 are specified ahead of this profile's use of them** (`AVG` via decomposed state + presentation view; group-rung retraction; the bounded-domain multiset). The mechanisms live in `model_transforms.md` / `model_properties.md`; wiring them into keyed columns is future composition work.

### Interval versioning

- **Not implemented — `versioning:` does not parse.** `RefreshStrategy` (`crates/smelt-core/src/config.rs`) accepts only `full` / `incremental` / `materialized_view` (the removed mode names hard-error with a fix-it), and `grain: key` parses — but there is no `versioning:` frontmatter key at all today (`models.md` §Known Divergences), so `versioning: interval` fails deserialization. The classifier, the close-old / open-new maintenance (via `merge_into`), and the validity-column management are delivered by `docs/plans/20260707-maintenance-plan-impl.md`.
- **Validity-column surface is unsettled.** Exact names/types of `valid_from` / `valid_to` / `is_current`, whether the open interval uses NULL or a sentinel far-future timestamp, and whether these are configurable are Open Questions to settle when the profile is built.
- **Tracked-attribute selection is unsettled.** All projected non-key columns vs an explicitly declared subset; how a modeller marks a column untracked. Prefer deriving from SQL over a strategy block; the exact line is undecided.
- **Late corrections to a closed interval.** Deletion is settled as a soft-close (§"Deletion handling"), but how a correction to an *already-closed* interval is applied — and any opt-in hard-delete surface — need their own design, the same retraction question the key grain shares (§"Reprocessing"; `docs/research/20260703-model-updates.md` §18.2).
- **Umbrella subsumption.** Whether this profile shares execution machinery with the plain key grain or is a standalone classifier is settled here as **standalone** (its own classifier), consistent with the narrow-composable-rules posture (`docs/research/20260522-cumulative-as-its-own-rule.md`). It composes shared capabilities by name but owns its combiner.

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

- **Conditional maintenance without a change feed.** Three composable mechanisms
  (`docs/research/20260715-conditional-maintenance-without-cdf.md`; tracking plan
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`):
  - **M1 — change-suppressed writes**: the emitted MERGE gains an `IS DISTINCT FROM` matched-arm
    predicate (merge-less backends get a staged-candidate conditional DELETE+INSERT), so an
    unchanged region writes zero rows and redelivery storms become no-ops. Built for the
    column-scoped and keyed-fold write families (`model_transforms.md` §Known Divergences
    "Change-suppressed MERGE").
  - **M2 — delta-restricted enrichment compute**: where the row skeleton is provably owned by
    the driving source alone (payload-only 1:1 enrichment joins), the expensive joins run only
    over rows whose enrichment inputs changed — the classical delta-join algebra, licensed by
    the skeleton-source-closure proof (`model_properties.md` §"Skeleton-source closure", P1) plus
    an exact input delta. The proof, the transform (`model_transforms.md` — delta-restricted
    enrichment join), and the `referential_integrity` world-fact (`sources.md`) the proof's
    row-preservation conjunct consumes for an inner-join enrichment are all built and reach a
    maintained-model edge's own driving-source recompute (see the maintained-model-edge paragraph
    above). The compute-restriction licence now also extends to an `UpstreamMutation` cell driven
    by an external `mutable_snapshot` source: the SAME skeleton-source-closure proof and the SAME
    restriction gate (`smelt_logical::maintenance::choice::resolve_recompute_restriction`) admit
    the cell when its enrichment join closes and the fingerprint sidecar's synthesized
    changed-key set (M3) is non-empty for the touched region — a renamed dimension row's recompute
    is then a point lookup on that row's key, not a scan of every fact row the dimension's
    unclocked, accepted-full-scan reach would otherwise touch. Wiring this into a live run's own
    trigger/technique dispatch (`crates/smelt-runtime/src/execute.rs`'s regular incremental batch
    loop) is separate follow-on work; today the mechanism — the closure derivation, the delta
    threading, and the emitted delta-restricted statement — is proven against a real fixture and a
    real backend directly, the same "build it, then wire live dispatch" split M3's own sidecar
    build/consume halves went through.
  - **M3 — derived change feeds**: snapshot-diff made real on both boundaries — a fingerprint
    sidecar (lifecycle: `sources.md` §"The fingerprint sidecar") synthesizes a change feed for an
    external `mutable_snapshot` source, and the
    conditional write's own changed-row set is recorded as the model's **observed output
    delta**, turning every maintained model into a change-feed-postured upstream for free. On a
    composed (key + time) output the observed delta projects to exact partition dirt
    (§"What the composed shape uniquely enables"), which is what makes M3 propagatable through
    the interval-based graph without keyed dirt-sets. The output-delta half (recording +
    key→partition projection) is built for the change-suppressed column-scoped MERGE family
    (§Known Divergences "Observed-delta recording is built…"); the fingerprint-sidecar half is
    built for DuckDB (table DDL, digest-refresh upsert, and the emitter-authored diff query —
    `sources.md` §"Known Divergences" — "The fingerprint sidecar is built for DuckDB"), as a
    standalone, independently-tested capability — a non-DuckDB target fails loudly. Invalidation is
    live: a stored row's identity stamp (digest-construction version, P4 projection identity, and
    a hash of the consuming model's SQL) is checked against a freshly computed one on every diff,
    and any mismatch — a projection change, a model-definition edit, or a corrupted stamp —
    degrades that partition to the same whole-table delta an absent sidecar produces, logged
    loudly, never silently trusted or silently skipped (`sources.md` §"The fingerprint sidecar" —
    "Invalidation"). Wiring the sidecar's synthesized changed-key set into a live run's own
    trigger/technique selection (so a maintained model actually consumes it instead of the
    whole-table fallback) is separate follow-on work.
  Each mechanism needs its own spec diff before it is surface: P1–P4 proofs in
  `model_properties.md` (P1–P4 landed — P4, fingerprint projection, §"Fingerprint projection"),
  T1–T5 transform variants in
  `model_transforms.md` (T1/T2/T3 landed as catalogue rows; the observed-output-delta recording
  (T5) is specified in this spec's own graph-layer section above rather than as a catalogue row;
  T4 — the fingerprint sidecar build + diff query — is built for DuckDB, matching the M3 status
  above), the referential-integrity world-fact (landed) and landed-delta refinement (landed) in
  `sources.md`, capability flags in `multi_backend.md`, and a persistence-fingerprint stance
  reconciled with `output_fingerprint.md`.

## References

### The contract, plan, and graph layer

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
  `crates/smelt-maintenance-testkit` (dev-only, `publish = false`; the Link-C in-process harness —
  the real-run-pipeline driver, the typed `ModelRecipe` generator (`recipe.rs`), the schema-generic
  schedule generator (`schedule_gen.rs`), the S-tracked equivalence oracle (`s_tracker.rs`,
  `oracle_modes.rs`), and the multiset-equality oracle (`oracle.rs`) — wired as a dev-dependency of
  `smelt-cli`); `cargo test -p smelt-cli --test maintenance_conformance` is the standing generative
  equivalence gate: on every `cargo test`, a deterministic-seeded sample of typed `ModelRecipe`
  values (append-only partition-grain, fact+mutable-dimension, `grain: key`, and generated 2-3 node
  DAGs) is staged, classified through the real maintenance derivation, and driven through
  `execute_project` against a real DuckDB backend, asserting emitted maintenance output equals a
  full-refresh oracle after every run step under adversarial append/lateness/mutation/redelivery/
  definition-change schedules (`SMELT_CONFORMANCE_CASES` scales the sample depth for a deeper local
  or nightly soak run). A composed (`grain: key` + `timeseries:`) recipe family exercises all three
  key-temporal-locality routes — key-embedded (driven through `execute_project`), key-determined,
  and declared-recurrence-bounded with in-bound redeliveries (both driven directly against a real
  DuckDB backend, the workaround `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`
  also uses) — asserting whole-table and per-slice equivalence after every step, gated by its own
  admission-rate floor (`SMELT_CONFORMANCE_COMPOSED_CASES` scales its sample depth). A generated
  model-edge enrichment recipe family (one closure-admissible `LEFT JOIN` shape and its two
  closure-failing siblings — a bare inner join, and a membership predicate over an enrichment
  column) drives the delta-restricted-vs-widened-scan choice both ways over the same fixed
  processed-input set `S` — its own P1 skeleton-source-closure verdict, derived through the real
  per-cell derivation rather than asserted, gates which cases the equivalence check runs, and the
  end states must be bit-identical; a second case drives a fully-suppressed conditional write
  through its own real observed-delta recording and asserts the cascade this composition exists
  to unlock — zero rows written, a present-and-empty recorded delta, zero regions scheduled across
  every downstream consumer, and an end state still equal to a from-scratch full-refresh oracle —
  both gated by their own admission-rate floor. The
  key-determined route's merge mechanics (write-once partition, additive fold) are exercised this
  way against real DuckDB, but its slice-pruned target scan is not — the driver runs every
  key-determined step with the slice predicate omitted because DuckDB's `MERGE` binder refuses the
  real predicate shape (§"Key temporal locality" above, the `BindMerge` divergence). A `pinned`
  module reproduces every construct × posture cell and named hazard
  schedule the gate subsumes as deterministic, always-reproducible cases (never proptest-drawn
  alone), and a `registry` module tracks named divergences with a staleness report (entries that
  never fire over the deterministic sample are reported, never failed — the same governance pattern
  `crates/smelt-db/tests/prop_helpers/known_unknowns.rs` uses). The per-cell probe modules under
  `crates/smelt-cli/tests/property_discovery/` cover constructs the typed recipe generator has no
  vocabulary for yet (self-referential models, `UNION ALL`, `LEFT JOIN`, correlated `EXISTS`,
  stacked window frames, cross-source column-name collision, a mutable source aggregated directly)
  and remain disposable research probes (see `.claude/scripts/property-experimental-gate.sh`).
  `crates/smelt-cli/tests/incremental/` is narrower still: it drives a backend's incremental
  strategy directly given a hand-supplied filter, proving the strategy executes correctly once
  handed one, independent of how that filter is derived. `cargo test -p smelt-logical --test
  maintenance_plan_conformance :: coverage_matrix_is_inhabited` is the standing inventory gate over
  the research example catalogue's coverage matrix (`docs/research/20260705-refresh-as-maintenance-plan/
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
  in `KNOWN_GAPS` (most as "plausibly covered by an existing `maintenance_conformance::pinned`
  hazard case or `G-*`/`SC-*` property-discovery probe, not re-verified against this exact catalogue
  id" — cross-referencing those cases to catalogue ids by name is itself unbuilt; a few, like
  EX-25's LAG/LEAD footprint reflection and EX-29's as-of-run-contract gating, need production
  investigation not yet done). Both lists are per-cell, never per-row, so a future change can lift
  one cell at a time without re-deriving the whole inventory.
- **User docs**: `docs-site/docs/index.md`, `docs-site/docs/guide/{incremental-models,sql-models,materializations}.md`,
  `docs-site/docs/concepts/how-it-works.md`, `docs-site/docs/reference/{timeseries,smelt-yml,cumulative-aggregate,cli}.md`
  describe the trichotomy + grain surface; `docs-site/docs/reference/cli.md` also documents
  `--since-upstream`, `--include-upstreams`, and `smelt explain`'s cell/clamp/ledger report with
  `--show-sql`; `docs-site/docs/reference/smelt-yml.md` documents the `maintenance:` block.
- **Plans (history)**: `docs/plans/20260704-model-updates.md`,
  `docs/plans/20260704-model-updates-fundamentals.md` (the L1+L2 substrate),
  `docs/plans/20260705-property-discovery-loop.md` (the empirical engine).
- **Research**: `docs/research/20260715-conditional-maintenance-without-cdf.md` (change-suppressed
  writes, delta-restricted compute, derived change feeds — the source of the pruning taxonomy's
  no-op write-elimination category and the composed-shape composition points);
  `docs/research/20260716-relation-contract-and-per-cell-addressing.md` (the shared Relation
  Contract, grain-as-derived-label, per-cell write addressing, and the open write-pattern registry
  this spec's §"Per-cell write addressing" and §"The declared shape axis" encode).
- **Related specs** (one list for the whole spec): `model_properties.md` (the derived proofs —
  monotonicity trace, bound/reach, partition alignment, determinism, discriminants, anchor
  resolution, once-write and join-contribution proofs, `bounded_domain:`); `model_transforms.md`
  (the physical mechanisms — pushdown, DELETE+INSERT, the clamps, pinning, `merge_into`, the
  windowed-keyed-maintenance driver, dimension-horizon MERGE); `models.md` (the refresh axis,
  the declared shape facts + derived grain label, the Relation Contract, three-state declaration
  law, input-consumption axis, litmus rule);
  `timeseries.md` (declares `event_time_column`, `partition_column`, `granularity`); `sources.md`
  (host of `timeseries:` and source-lateness/mutation-profile/key-recurrence world-facts);
  `expansion.md` (function expansion; runs before every analysis stage here); `functions.md` (the
  pattern-function surface); `materialized_view.md` (the engine-owned shape profile — where
  beyond-the-ladder shapes and hand-written SCD2 go); `multi_backend.md` (backend capability
  flags a strategy checks); `schema_evolution.md`, `run_state.md`, `virtual_environments.md`,
  `diagnostics.md`, `architecture.md`, `cli.md`.

### The partition grain

- **Code**:
  - `crates/smelt-core/src/config.rs` — `BatchedConfig`, `Granularity`, `Weekday`
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction, `ModelMetadata`
  - `crates/smelt-logical/src/rules/incremental.rs` — partition-grain detection + safety checks (in `smelt-logical`; `smelt-planner` re-exports)
  - `crates/smelt-logical/src/types.rs` — safety-override types
  - `crates/smelt-runtime/src/transformer.rs` — `inject_time_filter`, `inject_source_filters`, `is_transparent_single_source`
  - `crates/smelt-backend/src/lib.rs` — `Backend::delete_partitions`, `Backend::insert_into_from_query`, `Backend::delete_and_insert_transactional` (per-chunk transaction boundary)
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `DeleteInsert` impl
  - `crates/smelt-dialect/src/dialect.rs` — `BackendCapabilities::supports_merge`
- **Tests**: batched safety unit tests in `crates/smelt-logical/src/rules/incremental.rs`; CLI integration tests in `crates/smelt-cli/tests/incremental_*.rs`; the per-partition full-refresh-equivalence harness
- **User docs**: [`docs-site/docs/guide/incremental-models.md`](../../docs-site/docs/guide/incremental-models.md), [`docs-site/docs/guide/materializations.md`](../../docs-site/docs/guide/materializations.md)
- **Plans (history)**:
  - [`docs/plans/20260322-incremental-model-support.md`](../plans/20260322-incremental-model-support.md) — comprehensive plan; many phases still open
  - [`docs/plans/20260325-materialization-types.md`](../plans/20260325-materialization-types.md)
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — the mode-vertical master this spec re-cuts as a composition
  - [`docs/plans/20260707-maintenance-plan-impl.md`](../plans/20260707-maintenance-plan-impl.md) — lands the target frontmatter surface and diagnostics
- **Research**:
  - [`docs/research/20260521-incremental-as-planner-rule.md`](../research/20260521-incremental-as-planner-rule.md) — design direction this spec absorbs
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — batched eligibility audit; §9.2 non-determinism derivation
  - [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — the maintenance-framework design
  - [`docs/research/20260705-refresh-as-maintenance-plan/`](../research/20260705-refresh-as-maintenance-plan/) — the shape-profile demotion and per-cell admission this spec composes
- **Legacy reference**: `docs/DESIGN.md` §"Incremental Table Builds" — superseded for current behavior; useful for design rationale

### The key grain

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy`); `crates/smelt-logical/src/rules/cumulative.rs` (the built classifier seed — combiner lookup, GROUP-BY key derivation, driving-source resolution); `crates/smelt-runtime/src/maintenance_driver.rs` (the windowed-keyed-maintenance driver, `WindowedKeyedRule`); `crates/smelt-runtime/src/cumulative.rs` (per-window merge execution); `crates/smelt-backend/src/lib.rs` (`merge_into`), impls in `crates/smelt-backend-duckdb`/`-spark`.
- **Tests**: the cumulative classifier unit tests (`smelt-logical/src/rules/cumulative.rs`); the keyed end-state-equivalence harness; `smelt-backend-duckdb` `merge_into` tests.
- **User docs**: `docs-site/docs/guide/materializations.md` (to be replaced by a keyed-models guide with per-pattern recipes); `docs-site/docs/guide/incremental-models.md` §"The composed shape (key + time)" documents the composed (key-addressed *and* time-partitioned) form and its three locality routes; `docs-site/docs/examples/web-analytics/deduplication.md` is the worked tutorial — a redelivery-prone feed deduplicated by a keyed extremal fold under a declared recurrence bound, contrasted against the partition-grain `QUALIFY`-window workaround the preceding tutorial page builds.
- **Plans (history)**: `docs/plans/20260523-cumulative-aggregate.md` (the built seed); `docs/plans/20260704-model-updates.md` (the mode-vertical master this spec re-cuts as a composition); `docs/plans/20260705-keyed-collapse.md` (the keyed-collapse sub-plan); `docs/plans/20260707-maintenance-plan-impl.md` (lands the target frontmatter surface and diagnostics).
- **Research**: `docs/research/20260705-keyed-time-superset.md` (key temporal locality, the time-partitioned output, per-input scope maps); `docs/research/20260705-model-refresh-review.md`; `docs/research/20260705-unified-keyed-refresh.md`; `docs/research/20260705-keyed-collapse-application.md` (the decision record this spec encodes); `docs/research/20260704-monotone-join-maintenance.md` (the monotone-vs-retractable boundary); `docs/research/20260703-model-updates.md`; `docs/research/20260705-refresh-as-maintenance-plan/` (the shape-profile demotion and per-cell admission this spec composes).

### Interval versioning

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy` — no `grain`/`versioning` surface yet); on build, the classifier under `crates/smelt-logical/src/rules/` and the maintenance path under `crates/smelt-runtime/`.
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 17 (the user surface; naming); Part 19 (the input-consumption axis)
  - [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — the maintenance framework this profile composes into
  - [`docs/research/20260705-refresh-as-maintenance-plan/`](../research/20260705-refresh-as-maintenance-plan/) — the shape-profile demotion and per-cell admission this profile composes
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — the sibling-rule sketches (`scd2`, `latest_value`, `accumulating_snapshot`)
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
  - [`docs/plans/20260707-maintenance-plan-impl.md`](../plans/20260707-maintenance-plan-impl.md) — lands the target frontmatter surface (`grain`/`versioning`) and diagnostics

