---
feature: incremental_shapes
status: experimental
last_reviewed: 2026-08-16
owners: [andrew]
---

# Incremental Shape Profiles

> **What this is.** The normative spec for the **shape profiles** of maintained models — the
> per-shape implementation chapters for the two declared output addressings: the **partition
> grain** (a clocked table maintained partition by partition) and the **key grain** (a keyed
> table maintained by folding deltas into per-key state), including the composed key-and-time
> form. A profile owns only what is meaningful *inside* its shape; everything shared it composes
> by name. Out of scope, with their own homes: the delta algebra, the equivalence invariant, the
> maintenance plan, per-cell admission and write addressing, the contract lattice, the frontier,
> and the graph layer (`incremental_models.md`); definition-delta migration
> (`definition_deltas.md`); the provable properties of a model's SQL (`model_properties.md`);
> the physical transform mechanisms (`model_transforms.md`); the `refresh:` axis and declaration
> law (`models.md`); the `timeseries:` declaration grammar (`timeseries.md`); source world-facts
> (`sources.md`); engine-maintained views (`materialized_view.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Overview

*This section is a non-normative primer. The normative statements live in §Surface, §Semantics,
and §Constraints & Invariants — on any conflict, those win.*

A maintained model declares at most two shape-defining facts about its output: a **clock**
(`timeseries:` — the output has a time axis) and an **identity** (`unique_key:` — the output is
addressable by key, one row per key). `incremental_models.md` owns that declared surface and
everything derived from it that is shared across shapes. This spec owns what is *not* shared: a
**shape profile** is the implementation chapter for one output addressing — which declarations
it accepts, how its runs execute, which SQL it admits, and the local machinery (ledgers, safety
checks, column families) that exists only inside that shape.

Three shapes are inhabited, each named by its derived **grain** label:

- **partition grain** — a clock and no identity: a complete time-partitioned table, kept current
  by rewriting touched partitions.

  ```sql
  ---
  refresh: incremental
  timeseries: { event_time_column: order_date, partition_column: order_date, granularity: day }
  ---
  SELECT order_date, SUM(amount) AS revenue FROM smelt.orders GROUP BY order_date
  ```

- **key grain** — an identity and no clock: keyed state, one row per key, kept current by
  folding deltas into per-key state.

  ```sql
  ---
  refresh: incremental
  unique_key: [order_id]
  ---
  SELECT order_id, SUM(item_count) AS total_items, MAX(shipped_ts) AS shipped_at
  FROM smelt.order_events GROUP BY order_id
  ```

- **composed key + time** — both facts: a key-addressed *and* time-partitioned table, folding by
  key with the target scan pruned to a time slice (§"Key temporal locality (the time-partitioned
  output)").

  ```sql
  ---
  refresh: incremental
  unique_key: [event_id]
  timeseries: { event_time_column: first_seen_at, partition_column: first_seen_date, granularity: day }
  ---
  SELECT event_id, MIN(event_ts) AS first_seen_at, CAST(MIN(event_ts) AS DATE) AS first_seen_date,
         MAX_BY(payload, event_ts) AS payload
  FROM smelt.raw_events GROUP BY event_id
  ```

Declaring neither fact leaves no maintainable shape. A fourth derived label,
`key_per_partition` (the key recurs across partitions, storing a per-partition trajectory), is
derived-only: declaring it is a hard error, and the derived label currently refuses at plan
derivation (§Known Divergences).

The shapes are not modes to choose between — they are what the declared facts already imply,
and they compose. Declaring `unique_key` on a clocked table does not bolt "dedup" onto it; it
makes the output key-addressable, which unlocks keyed maintenance for dimension corrections
while new-data arrival still rewrites partitions. Several capabilities exist *only* in the
composed shape (§"Key temporal locality (the time-partitioned output)").

The examples throughout draw from the running warehouse of `incremental_models.md` §Overview
("The running example"): sources `orders`, `order_events`, `raw_events` (redelivery-prone,
declared `key_recurrence: '7 days'`), `customers` (mutable dimension snapshot), and
`customer_changes`. `order_date` throughout is the date form of `orders`' clock `order_ts`,
derived in each model that reads it. Models:

| model | declares | shape |
|---|---|---|
| `daily_revenue` | clock | partition grain |
| `order_lifecycle` | identity | key grain (bare) |
| `order_facts` | clock + identity (joins `customers`) | composed — the per-cell-addressing example |
| `event_dedupe` | clock + identity | composed — the locality example |

Reading guide: *what can I declare, and what errors can I get?* → §Surface. *What does a run of
my shape actually do?* → §Semantics, one chapter per shape. *Why is the shape designed this
way?* → §Design. *What must never break?* → §Constraints & Invariants. *Where does the
implementation fall short?* → §Known Divergences.

## Surface

### Partition-grain declaration (`grain: partition`)

Opt-in: `refresh: incremental` plus a `timeseries:` clock and **no declared identity**. The
stored `table` is implied. `daily_revenue` from the running example:

```sql
---
refresh: incremental
grain: partition              # optional CHECK-ONLY assertion of what the facts already fix
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
safety_overrides:             # optional; bypasses specific safety checks (§"Safety checks (per-cell admission for recompute-a-region)")
  allow_window_functions: true  # waives the window-function check for this model
columns:
  inserted_at:
    contract: plausible       # optional; exempts this column from the determinism requirement
---

SELECT CAST(order_ts AS DATE) AS order_date, customer_id, SUM(amount) AS revenue,
       NOW() AS inserted_at
FROM smelt.orders
GROUP BY CAST(order_ts AS DATE), customer_id
```

Rules:

- The `timeseries:` block (grammar: `timeseries.md`) is **required**: a model asserting
  `grain: partition` without one is a hard error, `TimeseriesRequiredForPartitionGrain`
  (`models.md` §"Constraint violations").
- The declared `partition_column` must be **monotone**, validated by the event-time monotonicity
  trace (`model_properties.md`). Monotone admits a timestamp *or* an ever-increasing integer
  (sequence id / offset / watermark): a constant shift over such a column (`batch_id + 5`) is
  recognised on the same footing as a constant `INTERVAL` shift, while a non-monotone transform
  (`batch_id % n`, `batch_id * n`) is rejected fail-closed, naming the construct.
- `safety_overrides` is a top-level frontmatter key admitted **only** on a partition-shaped
  output (`models.md` §"YAML frontmatter keys").
- `columns.<c>.contract: plausible` (key owned by `models.md` §"`columns:` — column metadata";
  semantics owned by this spec) exempts that output column from the determinism requirement —
  audit stamps and surrogates the modeller accepts may vary between a run and a full refresh.
  Listing `event_time_column`, `partition_column`, or a `unique_key` column as `plausible` is a
  configuration error: skeleton positions must be deterministic (§"Safety checks (per-cell
  admission for recompute-a-region)").
- Declaring a `unique_key` here does **not** add a "dedup aid": it declares identity, which
  reshapes the output to the composed clock-and-identity shape (where keyed dimension-change
  addressing lives — `incremental_models.md` §"Per-cell write addressing"), and
  `safety_overrides` then becomes a hard error. A model that wants only whole-partition
  rewrites declares no identity.

The same declaration may live in `smelt.yml` instead; frontmatter wins over `smelt.yml` when
both set the same field:

```yaml
models:
  daily_revenue:
    refresh: incremental
    grain: partition
    timeseries: { event_time_column: order_date, partition_column: order_date, granularity: day }
```

### Key-grain declaration (`grain: key`)

Opt-in: `refresh: incremental` plus a declared `unique_key` — with no clock, or with a clock
admitted under key temporal locality (§"Key temporal locality (the time-partitioned output)").
The stored `table` is implied. `order_lifecycle` from the running example:

```sql
---
refresh: incremental
unique_key: [order_id]
grain: key                    # optional CHECK-ONLY assertion
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

Rules:

- The body **must** be an aggregated `GROUP BY` query (`KeyedRequiresGroupBy` otherwise):
  `unique_key` must restate the `GROUP BY` column list, and every non-key projection must
  classify into exactly one column family (§"The column-family catalogue"). The SQL must itself
  express the per-key semantics, so that a full refresh of the SQL is the profile's executable
  correctness oracle (§"End-state equivalence: the SQL is the oracle").
- One profile covers the running-aggregate, latest-value, and milestone patterns; what
  distinguishes them is the **column family** of each projection, derived from the SQL, never
  declared.
- No shape-specific config block exists, and `safety_overrides` is a hard error once identity
  makes the output key-addressed: every keyed rejection guards the equivalence invariant itself,
  and there is nothing safe to waive (§"Key-grain design").
- By default the output carries no partition column and downstream consumers read it in full,
  like any lookup. A `timeseries:` block on the model is admitted **iff key temporal locality
  is established** (§"Key temporal locality (the time-partitioned output)"), refused otherwise
  with `KeyedForbidsTimeseries` naming the three routes and the nearest missing fact. Output
  partitioning is independent of *consumption*: a keyed model over a clocked source consumes it
  window-forward regardless.
- `key_per_partition` is a **different derived grain**, not a sub-declaration and not writable:
  it stores the per-partition trajectory (`partition_column ∈ unique_key`), not the end-state
  this profile maintains.

The time-partitioned form, on the shape it exists for — event dedupe over a bounded redelivery
window (`event_dedupe` from the running example; the driving source declares `key_recurrence` —
`sources.md`):

```sql
---
refresh: incremental
unique_key: [event_id]
grain: key                    # derived from unique_key + locality-admitted clock
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

And in `smelt.yml` (frontmatter wins on conflict; the same `timeseries:`-admission constraint
applies):

```yaml
models:
  order_lifecycle:
    refresh: incremental
    grain: key
    unique_key: [order_id]
```

#### The column-family catalogue

The classifier assigns each non-key projection to exactly one **column family**. The family
fixes the cross-window combiner — a lookup off the aggregator; authors never declare combiners —
and every derived property:

| Family | Per-key aggregators | Cross-window combiner | Idempotent (re-run safe) | Order-independent | Invertible | Run shapes admitted | Extra licence |
|---|---|---|---|---|---|---|---|
| **additive fold** | `COUNT(...)`, `SUM(...)`, `BIT_XOR(...)` | `+` / `xor` | no | yes | yes | window-forward only | ledger-enforced re-run refusal (§"The transactional frontier write (merge ledger)") |
| **extremal / lattice fold** | `MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR` | `LEAST`/`GREATEST`/`AND`/`OR`/`&`/`\|` | yes | yes | no | window-forward only | — |
| **order-monotone overwrite** | `MAX_BY(value, ordering)`, `MIN_BY(value, ordering)` | max/min-by-ordering over hidden `(v, o)` state (§"Decomposed state (rung 2) in keyed models", §"Ordering ties (order-monotone overwrite)") | yes | up to ordering-key ties | no | window-forward only | — |
| **once-write** | `COALESCE`-first-non-null over the group | `COALESCE(target, delta)`, or the decomposed `(value, written)` state fold for the fallback/multi-candidate spellings (§"Decomposed state (rung 2) in keyed models") | yes | yes (given the proof) | no | window-forward only | once-write provenance proof (`model_properties.md`): key-derived, or a declared functional dependency over a NULL-preserving reduction |
| **decomposed fold** | `AVG(...)`, `STDDEV_*(...)`, `VAR_*(...)` | pairwise state combiner (§"Decomposed state (rung 2) in keyed models") | no | yes | per underlying combiner (additive state, invertible) | window-forward only | ledger-graded as additive (§"The transactional frontier write (merge ledger)") |
| **plain overwrite** | `ANY_VALUE(...)` | incoming row wins | yes | n/a — one row per key per scan | no | **snapshot-reconcile only** | — |

Any other aggregate, any non-aggregate non-key expression, and any composite expression over
aggregates (`SUM(x) + 1`) is rejected (`KeyedUnknownCombiner`). Add columns for the underlying
aggregates and derive downstream.

The order-monotone overwrite family needs no companion projection: the ordering expression's
value is carried as hidden state (§"Decomposed state (rung 2) in keyed models"), so the
cross-window combiner compares the *stored* state's ordering value against the delta's without
the modeller projecting it themselves. A `MAX_BY(x, x)` materialises the same uniform two-column
`(v, o)` state as any other call — value and ordering coincide, so the ordering state column
repeats the value expression rather than introducing a new one; there is no one-column special
case.

The once-write family admits four spellings, and no others:

- `COALESCE(<unique_key column>, …)` — key-derived, no declaration needed. Fallback arguments
  are permitted here: a key column is non-null within its own group by construction, so a
  fallback can never stand in for a value a later window would supply.
- `COALESCE(MAX(<col>))` / `COALESCE(MIN(<col>))` — a single-column reduction with **no further
  argument**, admitted only under a declared functional dependency naming `<col>` (the source
  payload, never the projection's alias) over a key the model's `unique_key` covers.
- `COALESCE(MAX(<col>), <fallback>)` / `COALESCE(MIN(<col>), <fallback>)` — the same reduction
  with a fallback argument, admitted under the same functional dependency, backed by the
  decomposed `(value, written)` state (§"Decomposed state (rung 2) in keyed models"): the raw
  reduction and the fallback are kept apart, so the fallback is applied fresh in `π` on every
  read rather than merged into the stored value.
- `COALESCE(MAX(<a>), MAX(<b>))` (and the `MIN` variants, and longer candidate lists) — a
  multi-candidate reduction, admitted under a declared functional dependency naming *every*
  candidate column, backed by one decomposed `(value, written)` state pair per candidate: `π`
  applies the arguments' declared preference order over the candidates whose state is `written`,
  so the order candidates happened to arrive in across windows never overrides the declared
  preference.

The functional dependency is declared in the model's frontmatter under
`functional_dependencies:` (a declared world-fact owned by `model_properties.md`), naming the
key and the column it determines — here, that an order's `order_date` is a per-key constant:

```yaml
functional_dependencies:
  - key: [order_id]
    determines: order_date
```

The family's **NULL-preservation obligation** follows directly from the equivalence invariant:
the presented value must be NULL exactly when the key has no value yet under a full refresh.
The bare key-derived and no-fallback single-reduction spellings discharge it directly, since
their cross-window combiner *is* `COALESCE(target, delta)` — "the first non-null value any
window produced wins." The fallback-bearing and multi-candidate spellings discharge it through
the decomposed state instead: the state never stores a fallback-applied or preference-collapsed
value, only the raw per-candidate reduction plus its `written` flag, and `π` — a pure function
of one row's state — applies the fallback or preference order on every read. A declared
functional dependency asserts that a candidate's payload is a per-key constant; it never asserts
that the payload is non-null, and this family is literally "first non-null", so intra-key NULLs
are anticipated for every candidate independently. Every spelling refuses `KeyedOnceWriteUnproven`
absent its functional dependency (§"Diagnostics").

A `COALESCE(...)` used as a null-safe composite `GROUP BY` key is a key column, not a
once-write column, and needs no proof.

The pattern functions `smelt.latest(value, ordering)` (→ `MAX_BY`), `smelt.once(value)` (→ the
once-write canonical spelling), and `smelt.current(value)` (→ `ANY_VALUE`) are intent-naming
sugar for the overwrite, once-write, and plain-overwrite families. They ship as a
`smelt.define` template file a project imports — ordinary transparent functions
(`functions.md`) with no parser or registry surface of their own — and their expansions are
admitted on exactly the same terms as hand-written calls. Promotion to built-ins is a possible
later step, not part of this surface (§Future Extensions).

### Diagnostics

All codes are catalogued in `diagnostics.md`; this spec owns the semantics of the shape-local
codes below. The shared plan codes (`Maintenance*`) and contract-lattice codes (`Contract*`)
are owned by `incremental_models.md` §"Diagnostics". Every rejection is fail-loud and
fail-closed: nothing degrades to a silent fallback (`incremental_models.md` §"Validator, not
chooser").

**Partition-grain codes.**

| Code | Fires when |
|---|---|
| `TimeseriesRequiredForPartitionGrain` | `grain: partition` asserted with no `timeseries:` block (rule owned by `models.md` §"Constraint violations"). |
| `PartitionGrainNotSafe` | The batch-safety classifier rejects the model's SQL (§"Safety checks (per-cell admission for recompute-a-region)"). |
| `EventTimeColumnNotVisibleAtOuterSelect` | The outer output-clamp cannot bind: a set operation or subquery hides `event_time_column` at the outermost SELECT (§"Event-time outer-visibility"). |
| `PartitionGrainForbidsMetrics` | A partition-grain model's body consumes `smelt.metric()` — the composition of metric expansion with time-filter injection is deliberately unspecified, so the combination refuses ahead of execution rather than composing unpredictably (§"Functions inside partition-grain bodies"). |

**Key-grain codes.**

| Code | Fires when |
|---|---|
| `KeyedRequiresGroupBy` | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `KeyedForbidsTimeseries` | The model declares `timeseries:` but key temporal locality cannot be established — no route applies; names the three routes and the nearest missing fact (§"Key temporal locality (the time-partitioned output)"). |
| `KeyedUnknownCombiner` | A non-key projection is not a direct call to a catalogued aggregator; names the offending expression. For a bare column or `ANY_VALUE` under window-forward, names `MAX_BY(value, ordering)` as the fix. |
| `KeyedGroupByContainsPartitionColumn` | The `GROUP BY` contains the driving source's `partition_column` and the model declares no `timeseries:` block — ambiguous between the partition shape and the key-embedded time-partitioned shape; suggests both fixes: `grain: partition` + `timeseries:`, or declaring `timeseries:` on the model to stay `grain: key`. |
| `KeyedForbidsWindowFunctions` | The outer SELECT uses `OVER (...)`. The keyed state *is* the window. |
| `KeyedForbidsNondeterministic` | The SQL uses `RANDOM()`, `UUID()`, or another per-row non-deterministic function. Time-dependent functions (`NOW()`/`CURRENT_*`) are admitted in payload positions and run as-is — the columns they feed carry no equivalence promise (`incremental_models.md` §"The equivalence invariant") — but stay refused in a `unique_key`/`GROUP BY` or other membership position, where they would make row identity itself time-of-run-dependent. |
| `KeyedSqlNotParseable` | The model body cannot be parsed into the shape the classifier reads. |
| `KeyedMultipleDrivingSources` | More than one timeseries-tagged source in the FROM clause; lists the candidates. |
| `KeyedOnceWriteUnproven` | A once-write (`COALESCE`) column — bare key-derived, single-reduction, fallback-bearing, or multi-candidate — has no once-write provenance proof for one or more of its candidate columns; names the column, the unproven candidate(s), and the three fixes (key-derived form; declaring the dependency — `functional_dependencies: [{key: [...], determines: <col>}]`, `model_properties.md`; remodelling). |
| `KeyedStateColumnCollision` | A decomposed-state column name (`<output>__<part>`, §"Decomposed state (rung 2) in keyed models") collides with a declared or projected user column; names both and the reserved suffix. |
| `KeyedRetractableContribution` | An enrichment join's per-key contribution is retractable — it feeds a decrementing aggregate or a value that must be un-seen — and the repair family cannot admit a per-group recompute for the retraction; names the failing repair obligation. Steers to `refresh: materialized_view` or DAG composition. Never fires on the join spelling alone (§"Enrichment joins", `incremental_models.md` §"The repair family"). |
| `KeyedSnapshotSourceUnsupportedColumn` | A column family inadmissible under snapshot-reconcile appears in a model with no clocked driving source; names the column, the family, and why the current-snapshot oracle cannot hold (§"Admission matrix (column family × source shape)"). |
| `KeyedSnapshotPostureUnsupported` | No clocked driving source, and no single unambiguous source to reconcile against either (two or more unclocked candidates in the FROM clause) — neither run shape can be derived (§"The two run shapes (derived, never declared)"). |
| `KeyedReprocessedWindow` | A run window covers a ledgered window of a non-re-run-tolerant model, or `--auto` detects changed input under an already-merged window, and the repair family cannot admit a per-group recompute for the change; names the failing repair obligation and points at `--full-refresh` (§"Reprocessing", `incremental_models.md` §"The repair family"). |
| `KeyedRecurrenceBoundViolated` | Runtime, declared-recurrence route only: a merged delta row matched (or would duplicate) a stored key outside the run's derived slice. The run's transaction rolls back; reports the violation count and sample keys (§"Key temporal locality (the time-partitioned output)"). |

## Semantics

One chapter per shape. A profile section owns only what is meaningful inside that shape;
everything else it composes by name. Each chapter opens with a **composition table** naming the
required properties, consumed world-facts, default-plan transforms, and the upheld invariant
specialisation; the chapter's normative content is exactly that table plus its own local
machinery below it, and it never re-specifies a capability a capability spec or a shared
section of `incremental_models.md` already owns.

### The partition grain (`grain: partition`)

The partition-addressed shape: a complete table with a monotone `partition_column`, kept
current by partition DELETE+INSERT (the recompute-a-region quadrant of the plan matrix,
`incremental_models.md` §"The plan matrix"), its default plan. Declared surface: §"Partition-grain
declaration (`grain: partition`)".

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: partition` — a complete table with a monotone `partition_column`, addressed by partition, not by key | `models.md` §"Refresh axis" |
| **Properties (required)** | event-time monotonicity trace; column nullability gate; unified bound/reach derivation; frame-reach taxonomy; injection-point/pushdown-depth; partition alignment (scoped); driving-fact/anchor resolution; determinism + nondeterminism predicate + taint; body-structure classifier; set-operation distribution; static-seed detection; window-independence/ordered-execution | `model_properties.md` |
| **World-facts (consumed)** | timeseries clock; source mutation profile and lateness margin; column-scoped equivalence contract (`columns.<c>.contract`) | `timeseries.md`, `sources.md`, `models.md` |
| **Default plan (recompute-a-region quadrant)** | source-filter pushdown; partition DELETE+INSERT; output-window derivation (skew inversion); outer output-clamp; two-layer widened-scan + exact output clamp | `model_transforms.md` |
| **Admission** | every check below is one instance of `incremental_models.md` §"Per-cell admission" for the recompute-a-region quadrant over a partition-grain output (§"Safety checks (per-cell admission for recompute-a-region)") | this spec |
| **Invariant upheld** | per-partition equivalence — the strengthening of the equivalence invariant and the plan's `S`-vector refinement | `incremental_models.md` §"The equivalence invariant"; §"Per-partition equivalence" |

The machinery below is partition-grain-**local**.

#### Execution model (DuckDB)

For run window `[start, end)`, the recompute-a-region quadrant drives: partition **DELETE**
over the **derived output window** — the run window pushed through the partition-column
relation, identity when it tracks event time, skew-inverted when derived and skewed from the
driving date column (**Form B** — the label the derivation code,
`crates/smelt-logical/src/maintenance/locality.rs`, and sibling specs use for this
derived-output-window inversion); the **outer output-clamp** (a `partition_column` range filter at
the outermost SELECT); per-source `partition_column` **filter pushdown** on each `smelt.<path>`
reference (non-`timeseries:` sources are lookups, read in full); and **INSERT** of the result.
Skew widens the DELETE beyond the run window itself — a one-day skew running `[D, D+1)` derives
output window `[D−1, D+2)`, so the DELETE also covers the skew-reached prior-day partition the
INSERT writes; deleting only the run window would strand it stale forever.

The outer clamp drops exactly for the **transparent slice** (one timeseries source, zero-margin
`Bounded(_, 0, 0)`, no skew), where pushdown already is the output clamp; a margin, skew, or
second source keeps it distinct from the scan window, each written partition's scan sized from
the output window's own reach, never the run window's. DELETE range and output clamp derive
from one window so the contract is idempotent for any write-window width; the output window is
a range to be **covered**, not one mandated statement — backfill chunking (§"First-run and
backfill") splits it into sequential DELETE+INSERT pairs, each sized from its own chunk's reach.

#### Strategy enum (backend-internal)

Strategy is not declared — it is derived per cell. `DeleteInsert` is the only physical strategy
today; DuckDB always uses it. A pure partition grain (no declared identity) has no keyed
addressing; keyed `MERGE` is the addressing a *dimension-change* cell derives on a composed
clock-and-identity output (`incremental_models.md` §"Per-cell write addressing") — per-cell,
not tied to a grain. The backend trait carries `insert_into_from_query`/`insert_overwrite` as
the capability that would admit an append-only or overwrite strategy for a shape whose
invariants permit it; no plan derivation selects them yet.

#### Run window vs partition granularity

The CLI `[--event-time-start, --event-time-end)` range declares a **run window**, not a
per-partition invocation: a daily-partitioned 30-day run is one engine query, one
partition-aligned DELETE over the 30 partitions, one INSERT; per-partition equivalence holds
regardless of run-window size.

Declared `timeseries.granularity` (`g_run`) must be at least as coarse as the granularity
implied by `partition_column`'s own truncation/grid transform (`g_part`), derived from the SQL
rather than trusted: a daily truncation implies `g_part = day`, rejecting an hourly
`granularity` as a DELETE+INSERT misalignment. `g_run >= g_part` is checked under the closed
coarseness ordering (hour < day < week < month < quarter < year); an opaque `g_part` skips the
comparison (undecided, not disproved). A sub-`g_part` run window is rejected with a diagnostic
naming the model's partition granularity and spelling out the coarsened run window that would
be accepted — never silently widened (auto-coarsening was rejected: it recomputes more than
the operator asked for; `docs/research/20260816-open-questions-triage.md`).

#### Batch safety classification

The optimiser rolls the per-source bound map into one class per model:

| Class | Meaning | Execution |
|---|---|---|
| `FullyBatchSafe` | all sources `Bounded(_, 0, 0)` | single query for any run window |
| `BoundedSafe(n)` | all `Bounded`, `n = max(before + after) > 0` | auto-sized chunks (3× the model's bounded lookback span `n`, clamped to 7–90 partitions) |
| `PerPartitionOnly` | any source `Unbounded` (cumulative) | one partition at a time, sequential |

`n` is rendered in the source's partition-column unit, the same value source-filter pushdown
reads. A `NotDerivable` source **refuses at planning time** (`MaintenanceReachNotDerivable`)
rather than being classed — no silent downgrade to full refresh (`incremental_models.md`
§"Validator, not chooser"). A `FullyBatchSafe` batch spanning more than 30 partition periods
warns, recommending `--per-partition`/`--batch-size <n>` (either suppresses it).

#### First-run and backfill

A first run and a backfill follow the same DELETE+INSERT contract (DELETE no-ops when the
partition is absent), chunked per the batch-safety class as above, sequential in temporal
order. A **self-referential** model (§"Window independence and self-referential models") cannot
bootstrap with a `CREATE TABLE` select, since its first batch reads the target via
`smelt.<self>`; the runtime instead materialises an empty target with the inferred schema
first, then runs every batch — first included — as ordinary DELETE+INSERT, matching
hand-seeding. Forced or `--per-partition` execution advances calendar-unit batches
(`Month`/`Quarter`/`Year`) by true boundaries; `Day`/`Week` use fixed steps. Output rows may be
finer-grained than `partition_column`, written in full per batch. Each chunk's DELETE+INSERT is
one transaction — INSERT failure rolls back only that chunk's DELETE, earlier committed chunks
stay (each chunk is idempotent) — and a run halts at the first failed chunk, exits non-zero,
and resumes correctly on re-run of the same range. smelt does not auto-re-run partitions on
late data; interim mitigations are trailing `--event-time-end` behind known latency, or
overlapping re-process ranges (a planned per-column `data_latency:` mechanism is §Known
Divergences). The contract-level statement is the derived horizon (`incremental_models.md`
§"Windowed maintenance and the horizon"): a late arrival past the clamp is silently excluded —
surfacing it is a model-author/data-quality concern.

#### Per-partition equivalence

For every partition `p` in `[run_start, run_end)`:
`partition_grain_run(model, [run_start, run_end)).where(partition_column = p) == full_refresh(model).where(partition_column = p)`
— the partition-grain strengthening of the equivalence invariant (`incremental_models.md`
§"The equivalence invariant"), independent of run-window size, for **local** columns (those
depending only on source rows visible within the model's source-filter ranges). A column
depending on history outside them (cumulative aggregation, connected-components,
backward-fill) is **not** equivalent, forces `Unbounded`/`PerPartitionOnly`, and is correct
only as-of-the-run. Bit-identical on deterministic columns; a `contract: plausible` column need
only be a *plausible* full-refresh value — never extended to a column governing which rows
exist, their partitioning, or dedup (§"Safety checks (per-cell admission for
recompute-a-region)").

#### Safety checks (per-cell admission for recompute-a-region)

The optimiser rejects (`PartitionGrainNotSafe`) SQL that breaks the partition-DELETE-then-INSERT
contract. Each check discharges one `incremental_models.md` §"Per-cell admission" obligation
via a shared `model_properties.md` proof, individually disabled via
`safety_overrides.allow_<check>: true` (opt-in, recorded):

| Check | Admitted when |
|---|---|
| **Window functions** | `PARTITION BY <keys> ⊇ partition_column`, or a bounded `RANGE BETWEEN INTERVAL '…' PRECEDING` frame with no `PARTITION BY`/`UNBOUNDED` hazard (`safety_overrides.allow_window_functions`). |
| **`HAVING`** | enclosing `GROUP BY` key ⊇ `partition_column`. |
| **`DISTINCT`** | `partition_column` projected in the same scope. |
| **`LIMIT`** | never — survival depends on which other rows are present, which differs run vs full refresh. |
| **Subqueries** (FROM/JOIN) | rejected unless overridden; a `WITH` CTE is *not* gated — CTE bodies flow through bound derivation via the body-structure classifier. |
| **Non-deterministic functions** | confined to a `contract: plausible` payload column (below). |

Checks are evaluated **per scope** — a `UNION` branch is judged against its own key set, never
a sibling's or the outer query's. A non-deterministic value is admitted only flowing
**exclusively** into a `plausible` column, never read back to place/filter/group/dedup a row;
the taint check hard-excludes the `event_time_column`/`partition_column` expression, any
`unique_key` column, and any row-set-membership/grouping position, regardless of opt-in.
`NOW()`/`CURRENT_*` are admitted as a direct projection without `plausible` — they execute
as-is at run time, never compile-time-pinned, and the columns they feed carry no equivalence
promise (the determinism scoping of the invariant, `incremental_models.md` §"The equivalence
invariant"); `RANDOM()`/`UUID()` always require it; declaring an excluded
column `plausible` is a configuration error, and `allow_nondeterministic` drops the guardrail
wholesale (discouraged).

#### Event-time outer-visibility

The outer clamp needs `event_time_column` **accessible** at the outermost SELECT. A plain
`UNION`/`INTERSECT`/`EXCEPT`, a `UNION ALL` with unprovable branches, or a subquery FROM not
projecting it, is rejected (`EventTimeColumnNotVisibleAtOuterSelect`) before execution. A
`UNION ALL` is **exempt** when every branch traces `Traceable` back to a real source's own
partition column; a `StaticSeed` branch is named and rejected, a `NotTraceable` branch keeps
the whole-model clamp.

#### Observing the per-source clamp

Because lookback is derived, not declared, the derived clamp is surfaced so authors can confirm
the analyzer read their SQL as intended: `smelt explain --json`'s per-cell `source_bounds` map
reports each source's `source_partition_col` and derived `(before, after)` offsets, resolving
the scan window when a concrete run window is given; editor hover (LSP) on a `smelt.<path>`
reference shows the same clamp alongside the schema/column readout. `Bounded(c, 0, 0)` reads
partition-by-partition, no lookback/lookforward; `Bounded(c, before, after)` reads the window
`c ∈ [run_start − before, run_end + after)`; `Unbounded` reads all history and forces
`PerPartitionOnly`; a lookup reads in full; a `NotDerivable` source surfaces the planning-time
refusal instead of a window.

#### Functions inside partition-grain bodies

Function expansion (`expansion.md`) runs **before** every analysis stage here, so a `LAG()`
inside a `smelt.define` body and one inlined at the call site are indistinguishable — the outer
clamp and pushdown both operate on the expanded CST. **Opaque calls remain black boxes**: bound
derivation cannot read through `smelt.extern`/built-ins, so time-dependence hidden behind one
is `NotDerivable` and refused unless a bound is provable from the surrounding SQL.
`smelt.metric()` calls are refused outright (`PartitionGrainForbidsMetrics`): how metric
expansion composes with time-filter injection is deliberately unspecified until the metrics
surface settles, and an unspecified composition must refuse loudly rather than execute
unpredictably.

#### Window independence and self-referential models

Whether windows build **in parallel** or must build **sequentially in temporal order** is the
window-independence/ordered-execution property, derived from the dependency graph, never
declared. **Window-independent (default):** lookback reaches only into sources, never the
model's own earlier partitions, so a backfill may split into any order including parallel.
**Window-dependent → ordered:** a **self-referential** model (reading its own prior partitions
via `smelt.<self>`) still executes as partition DELETE+INSERT — the same stateless/stateful
spine separating the two grains: stateful-ordered in execution, yet keeping the partition-grain
shape — but its windows build sequentially in strict temporal order, its backfill may not be
parallelised or reordered, and an edge the planner cannot prove converges
partition-by-partition is refused at planning time. A Form B skew anchored on a *non-self*
source rebases an `Ordered` model's write window exactly as a window-independent model's; the
self-edge itself is never a skew anchor — its own bounding relation is a distinct convergence
mechanism, even sharing the `partition_column` name.

#### State ownership

The project-wide doctrine — which state structures smelt keeps, the correctness/observability
split, residency, and optionality — is owned by `state.md`; this section states only its
partition-grain instance. smelt does not track watermarks, offsets, or run history for
partition-grain models — the backend owns computational state (DuckDB: table state +
transactions; Delta/Spark: transaction log + MERGE; Flink: checkpoints). Optional run-state
tracking with gap detection is opt-in via `state.mode: intervals` (`virtual_environments.md`;
layout owned by `run_state.md`). The key grain's transactional merge ledger (§"The
transactional frontier write (merge ledger)") is not an exception to this: it is a
correctness structure under `state.md` §"The residency rule" — engine-resident and
transactional with the write it describes — which is exactly what "the backend owns
computational state" requires.

#### `partition_column` validation

Owned by `timeseries.md` §"Constraints & Invariants" rule 1: `partition_column` must appear in
the output `SELECT` (and `GROUP BY` when grouping), else `MalformedTimeseries`; this profile
consumes that guarantee rather than re-checking it.

### The key grain (`grain: key`)

The key-addressed shape: keyed state, one row per `unique_key`, kept current by keyed
`merge_into` (the fold-a-delta quadrant of the plan matrix, `incremental_models.md` §"The plan
matrix"), its default plan. Declared surface: §"Key-grain declaration (`grain: key`)".

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: key` — the end-state per key, addressed by `unique_key`, not by partition | `models.md` §"Refresh axis" |
| **Properties (required)** | algebraic discriminants (is-monoid/needs-inverse/decomposable/value-vs-order-monotone) defining the column families; driving-fact/anchor resolution; event-time monotonicity trace of the driving source; once-write provenance; join-contribution monotonicity; input-delta discovery; key temporal locality for a time-partitioned output | `model_properties.md` |
| **World-facts (consumed)** | timeseries clock of a clocked driving source; source mutation profile; a declared key-recurrence bound where the route is declared rather than derived | `timeseries.md`, `sources.md` |
| **Default plan (fold-a-delta quadrant)** | keyed `merge_into` (target-as-replica) sequenced by the windowed-keyed-maintenance driver, source-filter pushdown on the driving source; the transactional merge ledger; dimension-driven horizon-bounded MERGE for enrichment shapes; slice-pruned merge target under established key temporal locality | `model_transforms.md` |
| **Admission** | every check is one instance of `incremental_models.md` §"Per-cell admission" for the fold-a-delta quadrant over a key-grain output (§"Admission matrix (column family × source shape)") | this spec |
| **Invariant upheld** | end-state equivalence — the end-state specialisation of the equivalence invariant; oracle is the model's own SQL | `incremental_models.md` §"The equivalence invariant"; §"End-state equivalence: the SQL is the oracle" |

The machinery below is key-grain-**local**.

#### The two run shapes (derived, never declared)

The run shape is the keyed application of the input-consumption axis (`models.md`
§"Input-consumption axis"), derived from the driving source. **Window-forward** — exactly one
FROM-clause source declares `timeseries:` (the driving source, by the driving-fact/anchor
proof; zero clocked sources means snapshot-reconcile, two or more is
`KeyedMultipleDrivingSources`): the run steps over covered source partitions in temporal order,
pushdown injects the window, the delta SELECT executes, and `merge_into` folds it with the
per-column combiner map; non-timeseries sources are read in full each step, and a missing
target is created from the first step's delta (`CREATE TABLE AS SELECT`).
**Snapshot-reconcile** — no clocked source: the run re-scans the source whole, aggregates per
key, and `merge_into`s it — matched overwritten, unmatched inserted, and a key **absent from
the incoming scan deleted**: it has departed the upstream (§"Departed keys and deletion").
Out-of-order, parallel, or sliced-backfill application is admitted **iff** the
model is order-independent (§"Derived execution postures"); otherwise windows apply
sequentially.

#### Departed keys and deletion

What happens to a key that disappears from the upstream is **derived from the source posture,
never declared**, and the default always preserves full-refresh equivalence:

- **Snapshot-reconcile:** a key present in the target but absent from the incoming scan has
  departed. The reconcile write deletes it — an anti-join of stored keys against the scanned
  snapshot, executed in the same transaction as the merge — so the stored table equals the
  oracle exactly.
- **Window-forward over an append-only source:** keys never depart — nothing upstream can
  remove a row — so retaining every key ever seen is exactly what a full refresh produces.
  There is nothing to delete.
- **A windowed scan over a mutable source:** departure is not observable — the departed key
  simply stops appearing in windows, and there is no tombstone to consume. Equivalence is
  maintained by whole-region recompute, or the shape is refused; it is never maintained by
  silently retaining the key.
- **A change feed with delete events:** the delete is applied as a delete once change-feed
  fold machinery consumes the feed's delta shape (`incremental_models.md` §Future Extensions);
  until then the posture's full re-derivation already removes the key.

Retaining departed keys as queryable history is an opt-in **declared relaxation** of the
equivalence invariant, owned by the contract lattice (`incremental_models.md` §"The contract
lattice") — never the silent default. Decision record:
`docs/research/20260816-open-questions-triage.md`.

#### Derived execution postures

Three model-level properties fold from the column families, derived and surfaced by
`smelt explain`, never declared. **Re-run tolerance:** may a merged window be blindly re-merged
over unchanged input? Holds iff every column is idempotent (no additive-fold column); an
additive model double-counts and must be refused (§"The transactional frontier write (merge
ledger)"). **Order-independence:** may windows apply out of order or in parallel? Holds iff
every combiner is order-independent — extremal/lattice, decomposed-fold, and proven once-write
qualify; order-monotone overwrite does not (only up to ordering-key ties, §"Ordering ties
(order-monotone overwrite)"), forcing sequential execution. **Reprocessing refusal:** a window
whose input changed since merging must not be re-merged for **any** family — an irreversible
fold cannot un-see a removed contribution, an overwrite cannot retract a superseded-by-nothing
value (§"Reprocessing").

#### The transactional frontier write (merge ledger)

Every **window-forward** keyed model maintains a per-model frontier — a backend table recording
each merged window — written **in the same transaction** as that window's `merge_into`. This is
one of the two named realisations of the frontier (`incremental_models.md` §"The frontier").
**Additive-fold** (not re-run tolerant): a run whose window is already recorded is refused
(`KeyedReprocessedWindow`) exactly; crash resume merges only unrecorded windows, so an
interrupted run resumes correctly by re-running the same range. **Re-run-tolerant:** a recorded
window may be re-merged (a no-op); the frontier serves reprocessing detection and `--auto`
bookkeeping, not refusal. The two grades differ in classification, not existence: for an
additive-fold model the frontier is a correctness structure and always exists; for a
re-run-tolerant model it is bookkeeping, written automatically whenever the project's state
mode supports it (`state.md`), so `--auto` staleness always has a record to consult.
Snapshot-reconcile models keep no frontier — each run is self-contained. This realisation is backend-resident and transactional with the write it
describes — a **correctness structure** in `state.md`'s classification (`state.md` §"The
state-structure inventory"), distinct from the opt-in run-state observability surface
(`run_state.md`), and the model realisation of `state.md` §"The residency rule".

#### Admission matrix (column family × source shape)

The key-grain instance of `incremental_models.md` §"Per-cell admission": each cell discharges
obligations 2 ("faithful fold") and 3 ("combiner algebra class") for one
`(column family × run shape)` pair. Fold families consume **events** (replayable,
retraction-free); overwrite families consume **observations** (current-snapshot semantics):

| Column family | window-forward | snapshot-reconcile |
|---|---|---|
| additive fold | ✓ (ledger-enforced) | ✗ — catalogue boundary (combiner is `+`; see below) |
| extremal / lattice fold | ✓ | ✗ — observer semantics |
| order-monotone overwrite | ✓ | ✗ — observer semantics |
| once-write | ✓ (provenance proof) | ✗ — observer semantics |
| decomposed fold | ✓ (ledger-enforced, graded additive) | ✗ — same catalogue boundary as additive fold |
| plain overwrite | ✗ — order-dependent over events (`KeyedUnknownCombiner` names the `MAX_BY` fix) | ✓ (current-snapshot semantics) |

The additive-fold and decomposed-fold ✗ cells are a **catalogue boundary**, not an equivalence
failure: the additive family's combiner is `+` by family design, while the snapshot posture's
valid maintenance would be overwrite-with-recompute — each run re-aggregates the whole current
snapshot per key, so overwriting the matched row's value would equal the full refresh
exactly. The catalogue does not currently assign that combiner to
aggregate spellings, so the cell refuses rather than silently switching a family's combiner.

The three *observer semantics* ✗ cells are equivalence failures, not double-count hazards
(those families re-merge safely): `MIN(price)` over snapshots computes *min ever observed*
against a full refresh's *current* min; `MAX_BY(attr, updated_at)` keeps a stale incumbent if a
mutation regresses the ordering value; `COALESCE`-once-write captures *first observed*,
unrecoverable from the snapshot — each refused (`KeyedSnapshotSourceUnsupportedColumn`) rather
than admitted silently. The replayable-feed obligation binds each **fold-contributing source**
(one whose columns feed the cumulative combiner), not every FROM-clause source: a mutable
source consumed only through a covered enrichment cell is admitted regardless of its own
mutation profile; a source that is **both** a fold input and a mutable enrichment stays refused
(`MaintenanceNoAdmissibleTechnique`) — admission fails closed.

#### End-state equivalence: the SQL is the oracle

Because the body is required to be the aggregation itself (§"Key-grain declaration
(`grain: key`)"), the oracle is executable for every admitted model — its **own SQL**.
**Window-forward:** for any set `S` of processed driving-source partitions and any admitted
ordering, stored state equals the model SQL evaluated over `source.where(partition ∈ S)`
(overwrite columns hold up to ordering-key ties). **Snapshot-reconcile:** the stored table
equals the model SQL over the current snapshot exactly — matched keys overwritten, new keys
inserted, departed keys deleted (§"Departed keys and deletion"); keeping departed keys is an
opt-in declared relaxation, never part of the oracle.

#### No write-eligibility clamp

A run merges **every** delta row it scans, into whatever key it names, however old. A derivable
forward reach is reported (`smelt explain`) but never gates admission or bounds which keys a
run may touch — no scanned input is ever silently dropped (`incremental_models.md` §"Windowed
maintenance and the horizon").

#### Decomposed state (rung 2) in keyed models

Rung 2 of the algebraic maintenance ladder (`incremental_models.md` §"The algebraic maintenance
ladder") says the user value can be `π(state)` for a richer monoid element and a pure
presentation map `π`. This section fixes where that state physically lives for the key grain,
which column families it licenses, and how it stays invisible to consumers — the piece every
decomposed-state admission in §"The column-family catalogue" cites by name. **Physical
layout:** state columns live in the *same* stored table as the presented columns, named
`<output>__<part>` (e.g. `total_spend__sum`, `total_spend__count`); the presented column is
materialised alongside them at merge time, computed by `π` from that row's own state (§"Key-grain
design" — the rejected separate-table alternative).

**Presentation projection.** State columns are excluded from the model's public schema
(`smelt.ref()` expansion, `SELECT *`, declared-schema checks, downstream type inference); a
collision with a declared or projected user column is a fail-loud `KeyedStateColumnCollision`
(§"Diagnostics"), never a silent rename. A wildcard reading a state-bearing model is rewritten
at compile time to its presented columns (sibling relations keep their own `<rel>.*`); a
hand-written `__part` name is an ordinary unresolved-column diagnostic, and an unresolvable
wildcard while a state-bearing model is in scope fails loud, naming the model and the wildcard.

**The state-shape catalogue.** Each decomposable family has one fixed, hand-encoded state shape
and presentation map; there is no general decomposition procedure:

| Family | State columns (`__` suffix) | Combiner over state | Presentation map `π` |
|---|---|---|---|
| `AVG(x)` | `sum`, `count` | pairwise `+` on each column | `sum / count`, `NULL` when `count = 0` |
| `STDDEV_*(x)` / `VAR_*(x)` | `n`, `sx` (`Σx`), `sxx` (`Σx²`) — the **moment-sum triple** | pairwise `+` on each column | per-family closed form over `(n, Σx, Σx²)` (population vs. sample divisor and `sqrt` per the specific function), `NULL` below the family's minimum `n` (`0` population, `1` sample) |
| `MAX_BY(v, o)` / `MIN_BY(v, o)` | `v`, `o` (the hidden ordering value) | keep the pair whose `o` is greater (`MAX_BY`) / lesser (`MIN_BY`); on equality the incumbent wins, matching §"Ordering ties (order-monotone overwrite)" | `v` — `o` is never presented |
| once-write | `value`, `written` (boolean) | `written` is `OR`; `value` is `COALESCE(target.value, delta.value)` — the incumbent's value survives once written, the delta only ever fills a state row that was never written | family-specific, below |

The moment-sum triple can lose precision for large `n` with small variance; a numerically
stable pairwise `(n, mean, M2)` state is an admissible future re-encoding with the same monoid
structure.

Each family's state combiner keeps the grade its rung-1 form carried, in the transactional
frontier write (§"The transactional frontier write (merge ledger)"): `AVG`'s and
`STDDEV_*`/`VAR_*`'s `SUM`-shaped state tuples grade **additive**, like `SUM`/`COUNT` itself;
`MAX_BY`/`MIN_BY` keeps its ordering-key-tie carve-out (§"Ordering ties (order-monotone
overwrite)"); `MAX_BY`/`MIN_BY` and once-write (order-independent given its provenance proof)
keep the **idempotent** grade.

**Once-write's `π` widens what the family admits**: separating the *raw* reduction from the
presented value means the reduction is never fallback-tainted, so a fallback or preference
order applies fresh on every read instead of being baked in and re-merged incorrectly by a
later window (§"The column-family catalogue" enumerates the admitted spellings and their
functional dependencies). The fallback-bearing and multi-candidate spellings each get one
`(value, written)` state pair per candidate, `written = (value IS NOT NULL)`, folded
independently; `π` returns the first written candidate in declared preference order, else the
fallback — a pure function of state, so merge order cannot leak into which candidate wins. The
bare key-derived spelling needs no decomposed state: a key column is already non-null by
construction, so plain `COALESCE(target, delta)` already computes the presented value.
`smelt explain` renders state columns as internal state, distinct from the public schema
(`incremental_models.md` §"CLI").

#### Key temporal locality (the time-partitioned output)

A keyed model may time-partition its output with a `timeseries:` block (`timeseries.md`; named
columns must be projections of the model, and `event_time_column` may name the partition column
itself). Admission requires **key temporal locality**: every stored row a run's deltas can
touch must lie within a computable **slice** of the output's time axis, letting the target scan
be pruned and downstream consumers window over the output. Preconditions: window-forward run
shape (snapshot-reconcile establishes no locality); `partition_column` names a `unique_key`
column or a non-key projection provably NOT NULL from a key's first row, in the
extremal-fold/overwrite/once-write family; `granularity` equal to the driving source's.

One of three **routes** establishes it: **(1) key-embedded** — `partition_column` is a
`unique_key` column; slice = scan window widened by lateness/skew margins. **(2)
key-determined** — the partition projection is a per-key constant under once-write provenance;
slice = the delta's own partition values, exact regardless of key age. **(3)
recurrence-bounded** — a **key-recurrence bound** `r` (same-keyed rows lie within `r` on event
time), derived from the SQL where decidable, else declared (`sources.md`, `key_recurrence`);
slice = scan window widened backward by `r` plus margins, admitted only **checked** — the run
verifies at merge time that no delta row matches or would duplicate a stored key outside the
slice, failing transactionally (`KeyedRecurrenceBoundViolated`) on violation; a declaration can
bound work, never silently drop data. **Pruning is not a write clamp** — no-op elimination on
the target scan only, never on which delta rows merge, and the same governing principle applies
as elsewhere: only proofs prune, a declared bound is admitted only checked, no unproven bound
ever refuses a write. Under routes 1–2 a key's partition value never changes; under route 3 it
may move (superseded by a late row), updated in place within the slice by the bound.

The composed shape — key-addressed **and** time-partitioned — is not "keyed with an
optimisation": the two declared facts must never be read as exclusive alternatives (the
declared-shape surface, `incremental_models.md` §Surface), because several capabilities hold
only in the composed form. In the table, the **settle bound** is the derived interval after
which a written slice provably receives no further changes (defined in `incremental_models.md`
§"CLI"):

| Capability | Bare keyed | Composed keyed + time |
|---|---|---|
| **Merge-target scan pruning** | none — whole key space | pruned to the slice; exact under routes 1–2, widened by `r` plus margins under route 3 |
| **Propagation admissibility** (graph layer) | interval propagation unavailable; admitted only where the output-delta verdict is `KeyedUpsert` (key-addressed edges, `incremental_models.md` §"The graph layer"); a `General` verdict refuses | admitted as a clocked node — the only way a keyed stage sits *inside* an interval-propagation chain |
| **Key→partition dirt projection** (graph layer) | n/a | exact for routes 1–2; widened by `r` plus margins for route 3 |
| **No-op write elimination** (statement emission) | whole key space | bounded by the pruned slice |
| **Settle × observed-delta composition** | n/a — no settle bound | static settle bound (route 1: lateness margin; route 3: `r` plus margins; route 2: never) composes with the dynamic observed delta to skip settled/empty-delta slices |
| **Consumer-visible output shape** | a lookup table, read in full each run | a clocked, time-partitioned keyed table; a re-written slice is *changed input* (§"Interaction with `--auto` / staleness") |

With locality established the invariant is checkable **per-slice** — stored rows equal the
model SQL over source rows within the slice's derived reach (implementation status: §Known
Divergences).

#### The maintenance boundary

On the algebraic maintenance ladder (`incremental_models.md` §"The algebraic maintenance
ladder") the keyed families sit on rungs 1 and 2 — every combiner folds `(state, delta)` with
no inverse and no history re-read. Additive and decomposed-fold are additionally **groups**
(invertible) — what a future subtract-then-add reprocessing path would exploit;
extremal/lattice, order-monotone-overwrite, and once-write are monoids but not groups (a
contribution cannot be un-seen), why reprocessing is refused for them. Rungs 3–4 (group-rung
retraction; the opt-in bounded-domain multiset) grow this shape without changing its contract;
beyond the ladder is delegated to `refresh: materialized_view`.

#### Reprocessing

If a merged window's source data changes, re-running the ordinary reprocessing path produces
incorrect state for every family. The change **routes to the repair family first**
(`incremental_models.md` §"The repair family") — a retraction or mutation with discoverable
affected keys and a bounded per-group slice recomputes just those groups, no reprocessing
refusal raised. The rule refuses at planning time only when a repair obligation itself fails,
`KeyedReprocessedWindow` naming the failing obligation and pointing at `--full-refresh` or a
manual cascade rebuild. Subtract-then-add for all-invertible models is a future path (§Known
Divergences).

#### Ordering ties (order-monotone overwrite)

`MAX_BY(value, ordering)`'s pairwise combiner: the delta wins iff
`delta.ordering > target.ordering` (strict); **on equality the incumbent wins** — deterministic
given processing history but not order-independent across windows, forcing sequential execution
(§"Derived execution postures"). Recommended practice: a composite, provably tie-free ordering
expression; the classifier cannot verify uniqueness.

#### Enrichment joins

A fact-to-dimension join is admitted when its per-key contribution is **provably monotone**
(the join-contribution monotonicity proof, `model_properties.md`): it feeds only
extremal/order-monotone/once-write columns and does not fan into a decrementing aggregate. The
line is monotone-vs-retractable **semantics, not join-vs-union spelling** — the join form
normalises to the same keyed-monoid merge as union; only a genuinely retractable contribution
is refused (`KeyedRetractableContribution`). A re-scanned existence flag requires the dimension
source declared `append_only`; extremal milestones are safe regardless. Where a dimension
batch's forward reach `H` is **derivable** from the SQL, the dimension-driven horizon-bounded
MERGE may clamp the enrichment recompute to `[event_ts, event_ts + H]` — a scan-side bound that
cannot under-cover because it is derived; where not derivable, the enrichment evaluates through
the ordinary widened scan. No declared value ever truncates a recompute or a write.

#### Key-grain output shape

One row per `unique_key`, column names the projection's aliases. By default there is no
`partition_column`/`event_time_column`/`timeseries:` — downstream consumers see a lookup table
read in full each run. With an admitted `timeseries:` block (§"Key temporal locality (the
time-partitioned output)") the output is instead a clocked, time-partitioned keyed table.

#### Functions inside keyed bodies

Function expansion runs **before** the classifier — projection reading, GROUP-BY inspection,
FROM-clause walking, family classification, and pushdown all operate on the expanded CST. A
`smelt.define`-resolved call is admitted iff its expanded body produces a catalogued aggregator
at the outermost expression position, with no privileged treatment for pattern functions
(§"The column-family catalogue"). Opaque calls (`smelt.extern`, non-inlinable built-ins) are
rejected via `KeyedUnknownCombiner`.

#### Interaction with `--auto` / staleness

**Window-forward:** re-run-tolerant models re-step stale windows (safe by idempotence);
additive models refuse re-processing of ledgered windows (`KeyedReprocessedWindow`) and steer
to `--full-refresh`. **Snapshot-reconcile:** the model is always-stale; every `--auto` run
reconciles.

## Design

Each paragraph records one load-bearing decision and what was rejected. Deeper derivations live
in `docs/research/` and are cited by full path. The family-wide decisions (validator-not-chooser,
per-cell addressing, widen-never-narrow, and their kin) live in `incremental_models.md` §Design;
this section owns the shape-local ones.

### Partition-grain design

**Logical SQL is pure; the framework injects the time filter.** A model body never contains
`is_incremental()` or conditional full-vs-incremental branching — the same SQL is both
descriptions; the framework injects the outer clamp and drives pushdown. Jinja-style
`is_incremental()` branching was rejected because it splits one model into two implicit ones
that drift; the trade-off — a per-model filter shape — is policed by the batch-safety analysis.

**DELETE+INSERT over partition columns, not MERGE, as the default.** MERGE was rejected as the
default because it requires a `unique_key` (not every model has one) and carries cross-engine
subtleties; it stays in the strategy enum for backends that opt in. DELETE+INSERT is idempotent
under fixed input and aligns with the partition-column safety analysis.

**Three-class batch-safety taxonomy.** A binary safe/unsafe flag was rejected — too many real
workloads are bounded-safe and need auto-chunking. A continuous safety score was rejected — the
user-facing decision is qualitative and maps directly to three backend execution shapes.

**Derive lookback from the model's SQL, not from frontmatter.** A `lookback_days:` annotation
would let declaration and logic drift; the trade-off — a model with implicit time logic refuses
eligibility and must be rewritten into a derivable form — is the right outcome. Deriving
removes the artifact the author would read to confirm behaviour, so the derived clamp is made
observable (§"Observing the per-source clamp") as the deliberate counterpart.
(`docs/research/20260521-incremental-as-planner-rule.md`.)

**smelt does not own state — scoped to the partition grain.** Owning a watermark store was
rejected: it duplicates engine state and opens a sync-correctness window. This is distinct
from the per-source *propagation* watermark (`run_state.md` §"Per-source watermark") — an
observability record of what forward propagation has already consumed, not a computational
substitute for backend transaction state; it is opt-in, degrades to full recompute in its
absence, and names no engine concept this paragraph's rejection covers. The key grain's
transactional merge ledger is not a counterexample but the doctrine's model correctness
structure (`state.md` §"The residency rule") — backend-resident, written in the same
transaction as the merge it describes, so it cannot drift from the state it records. Consequence: a backend may only select a physical strategy that
preserves the declared shape's invariants — `DeleteInsert` is the only one plan derivation
selects today. (`docs/research/20260705-keyed-collapse-application.md` D7.)

**Non-determinism is opted in per column, and confined by proof.** Whether a column is
acceptable-to-vary is a value judgement only the author holds, so it is declared
(`columns.<c>.contract: plausible`) — the one place derive-don't-declare correctly yields. A
whole-model `allow_nondeterministic` boolean was rejected: it drops the guardrail keeping
non-determinism out of the skeleton roles. The per-column opt-in keeps the guardrail and still
proves, via taint flow, that the tolerance did not leak.
(`docs/research/20260703-model-updates.md` §9.2.)

### Key-grain design

**One shape; the column family is the pattern.** The running-aggregate, latest-value, and
milestone patterns share the output shape, invariant, transform, and key derivation — they
differ only in per-column combiner algebra, and every consequence (re-run tolerance, ordering,
ledger, reprocessing) is derivable from the SQL. By the litmus rule (`models.md` §Design),
facts that change only execution posture under an unchanged contract are derived, never
declared, so they must not multiply the refresh enum. Splitting them into peer modes was also
rejected because combiner intent is **per column, not per model** — one table can mix an
additive fold, an overwrite, and two extremal milestones.
(`docs/research/20260705-unified-keyed-refresh.md`;
`docs/research/20260705-keyed-collapse-application.md`.)

**The SQL is the oracle.** The body must be the aggregation itself so that
`full_refresh(model SQL)` is an executable correctness oracle for every admitted model. A
bare-projection surface with mode-imposed dedup was rejected: its full refresh is not one row
per key, so the invariant would have no executable oracle. The plain-overwrite family
(`ANY_VALUE`) exists to give the snapshot posture an honest aggregated spelling under this
rule. (`docs/research/20260705-model-refresh-review.md` §1.1.)

**Derive `unique_key` and combiners from the SQL, not frontmatter.** The `GROUP BY` names the
key; each projection names its aggregator; the combiner is a fixed lookup. A config block
restating them re-introduces metadata-vs-SQL drift.
(`docs/research/20260521-incremental-as-planner-rule.md`.)

**No write-eligibility clamp.** A horizon-clamped merge (only keys newer than `run_start − H`
eligible) was rejected: it silently drops *scanned* inputs — the one silent-data-loss point in
the maintained family — and is unneeded for correctness, since merge work is proportional to
delta size. What a clamp would buy (settled-key GC, a work bound) is deferred optimisation that
must arrive as a package with late-fact accounting; slice pruning under key temporal locality
is not such a clamp — every scanned delta row still merges.
(`docs/research/20260705-keyed-collapse-application.md` D6.)

**Decomposed state lives in the presented table, not a second relation.** A rung-2 typed delta
needs somewhere to keep state richer than its presented value. A separate `<model>__state`
table plus a presentation view was rejected: it would make `ref()` sometimes resolve to a table
and sometimes to a view, and add a second relation to every backend's DDL and atomic-swap path
for no benefit — the presentation map is a per-row pure function of the same row's own state.
State columns live in the same stored table instead, under a reserved `__` suffix excluded
from the public schema.

**The time-partitioned keyed output is locality-gated, not a new mode.** The composed
(key, time) output absorbs the shapes that fall between the bare profiles — event-grain dedupe
over a bounded redelivery window, per-(key, period) aggregates, and the clock-sink problem
where a keyed stage strips the timeseries property from the DAG. A peer mode was rejected: the
form shares the key grain's invariant, oracle, driver, ledger, and column families, differing
by one derived/declared world-fact, which earns a gate rather than a peer under the litmus
rule. The gate exists because without locality the merge target is the whole key space and an
output clock would promise a partition structure the writes do not respect; the declared route
is runtime-checked because an over-optimistic recurrence bound would otherwise re-import the
silent truncation the no-clamp rule prevents. (`docs/research/20260705-keyed-time-superset.md`;
`docs/research/20260705-model-refresh-review.md` §3.2.)

**Observer semantics are refused, not smuggled.** Folding state observations (a mutable
snapshot) into `MIN`/`MAX`/once-write columns yields min-ever / first-observed values no full
refresh can reproduce — a genuinely different contract that admitting silently would put behind
one mode. The refused cells name the observer contract as the future opt-in path
(`incremental_models.md` §Future Extensions).

**Deletion is derived from the source posture, not declared.** A declared per-model retention
policy as the primary deletion mechanism was rejected: what deletion *can* mean is fixed by
what the source can signal (a snapshot reveals departure, a window cannot, a change feed says
it outright), so a knob would either restate the derivable answer or promise one the posture
cannot honour — and it would make the equivalence-preserving behaviour opt-in rather than the
default. The derived rule (§"Departed keys and deletion") keeps the default equal to a full
refresh with no knob; the one genuine preference — keeping departed keys as history — is a
declared contract-lattice relaxation, where every other deliberate weakening of the invariant
already lives. (`docs/research/20260816-open-questions-triage.md`.)

**Ties: honest boundary, not fake proof.** Incumbent-wins plus mandatory sequential execution
makes overwrite columns deterministic-given-history without claiming an order-independence no
static analysis can prove. A last-processed combiner (no ordering column, order-dependent for
all rows) was rejected outright; the snapshot posture's plain-overwrite family serves that need
where it is well-defined.

**No `safety_overrides:`.** The partition grain offers per-check overrides because some of its
rejections guard partial-correctness properties a modeller may knowingly waive. Every keyed
rejection guards the equivalence invariant itself — a bypass would produce silently
order-dependent or double-counted state; the escape from a rejection is to remodel, or to move
to `refresh: materialized_view`.

**One windowed executor, shared.** The window-forward step loop is the
windowed-keyed-maintenance driver (`model_transforms.md`), parameterised by
`(classifier, merge-SQL builder)`. Per-pattern copies of the loop were rejected as four-way
drift risk; a consequence is that every consumer inherits the driver's granularity support
(day and week today — widening is driver work, §Future Extensions).

## Constraints & Invariants

### Partition-grain constraints

1. **The logical model is pure SQL.** No `is_incremental()`, no macros, no conditional
   branches; the framework injects the time filter.
2. **`timeseries:` is required for `grain: partition`** — a hard error at workspace load
   otherwise (`models.md` §"Constraint violations").
3. **Strategy is not on the model.** The backend chooses the physical strategy for the
   recompute-a-region quadrant's execution.
4. **smelt does not manage computational state** (partition-grain-scoped doctrine; project-wide
   classification in `state.md`); watermarks, offsets, and run history live in the backend.
   The key grain's transactional merge ledger is a correctness structure, engine-resident by
   rule (§"Key-grain design"; `state.md` §"The residency rule"). A backend may select only a physical
   strategy that preserves the declared shape's invariants; `DeleteInsert` is the only one plan
   derivation selects today.
5. **Output-filter injection is per-model; source-filter pushdown is per-reference.**
6. **Per-partition equivalence with full refresh holds** on all local, deterministic columns; a
   `contract: plausible` column need only be a plausible full-refresh value; globally-dependent
   columns are not equivalent (§"Per-partition equivalence").
7. **Idempotence under fixed input:** re-running the same run window on unchanged sources
   converges to the same output state.
8. **Granularity is closed under partition arithmetic.** Run windows align to whole granularity
   units; the declared granularity must be at least as coarse as the derived partition grid
   (`g_run >= g_part`); violations reject, never silently widen (§"Run window vs partition
   granularity").
9. **Safety-check overrides are explicit.** A `safety_overrides` entry names the specific check
   it bypasses; there is no global disable.
10. **No silent downgrade to full refresh.** A rejected or `NotDerivable` model is refused at
    planning time with a diagnostic (`incremental_models.md` §"Validator, not chooser").
11. **`event_time_column` must be accessible at the outermost SELECT**, unless every
    `UNION ALL` branch traces `Traceable`; otherwise `EventTimeColumnNotVisibleAtOuterSelect`
    (§"Event-time outer-visibility").
12. **Non-determinism stays in the payload.** Admitted only into `contract: plausible` columns
    (plus `NOW()`/`CURRENT_*` direct projections, which run as-is and carry no equivalence
    promise); never in `event_time_column`, `partition_column`, a `unique_key` column, or any
    membership/grouping position. Declaring an excluded column `plausible` is a configuration
    error.

### Key-grain constraints

1. **Opt-in is `refresh: incremental` + declared identity** (storage implied `table`);
   `unique_key` is required and must restate the `GROUP BY`. No config block;
   `safety_overrides:` is a hard error.
2. **A `timeseries:` block is admitted iff key temporal locality is established**; otherwise
   refused (`KeyedForbidsTimeseries`).
3. **The body is an aggregated `GROUP BY` query; every non-key projection classifies into
   exactly one column family.** The combiner is a fixed lookup; authors never declare
   combiners.
4. **The catalogue is closed and the classifier fail-closed.** Unrecognised aggregators,
   composite expressions, unproven once-write columns, and retractable contributions are
   refused — never approximated.
5. **End-state equivalence holds with the model's own SQL as the oracle**, with one named
   carve-out: ordering-key ties on overwrite columns. Departed keys are handled by the
   posture-derived deletion rule (§"Departed keys and deletion") — deleted under
   snapshot-reconcile, structurally impossible under append-only window-forward — never by a
   silent retention carve-out.
6. **No write-eligibility clamp.** A run merges every delta row it scans; no scanned input is
   silently dropped. Slice pruning under locality is no-op elimination (or a
   transactionally-checked declared bound), never a write clamp. Any future clamp or
   settled-key GC must ship together with late-fact accounting.
7. **The run shape is derived from the driving source** (clocked ⇒ window-forward; unclocked ⇒
   snapshot-reconcile), surfaced by `smelt explain`, never declared.
8. **The admission matrix is enforced per column.** Fold and once-write families require a
   clocked (replayable) driving source; the plain-overwrite family requires the snapshot
   posture.
9. **Window-forward models maintain the transactional merge ledger**, written atomically with
   each window's merge. Additive-fold models refuse a ledgered window's re-run;
   re-run-tolerant models may re-merge. Snapshot-reconcile models keep no ledger.
10. **Ordering and parallelism follow the derived postures.** Out-of-order/parallel/sliced
    backfill only for order-independent models; overwrite columns force sequential temporal
    order.
11. **Reprocessing changed input is refused for every family** when detected; the mitigation is
    `--full-refresh` or a manual cascade rebuild.
12. **Exactly one clocked driving source under window-forward.** Zero selects
    snapshot-reconcile; two or more is refused.
13. **Without an admitted `timeseries:` block the output has no `partition_column`** and is
    consumed as a lookup; with one, it is a clocked, time-partitioned keyed table.
14. **The windowed step loop is the shared driver**, never a per-pattern copy
    (`model_transforms.md`).
15. **Key temporal locality is established only by the three named routes.** Derived routes
    prune by proof; the declared route prunes only under the transactional runtime check
    (`KeyedRecurrenceBoundViolated`). A violated declaration fails the run; it never silently
    drops.

## Known Divergences / Open Questions

Live gaps between this spec and the implementation, and questions where intent itself is
undecided, as of `last_reviewed`. Completed work is not recorded here — history lives in git
and §References → Plans. Family-wide gaps (plan, graph layer, contract lattice) live in
`incremental_models.md` §Known Divergences; definition-delta gaps in `definition_deltas.md`.

### The partition grain

- **One classification call site reads the outer SQL body**: the bound-`NotDerivable` refusal
  gate classifies on the outer `model.sql`, so a lookback living only inside a function body
  with no outer filter would diverge (no such case exists in the repo). Tracked:
  `docs/plans/20260530-thread-fn-registry-classification.md`.
- **The window-function batch-safety check runs on unexpanded outer SQL** — an `OVER` inside a
  `smelt.define` body is invisible to it. Tracked:
  `docs/plans/20260530-thread-fn-registry-classification.md`.
- **Per-source clamp observability is partly emitted (Open Question)** — `smelt explain --json`
  doesn't resolve the run-relative scan window when a run window is supplied; the editor-hover
  readout is unimplemented; specified ahead of a tracking plan.
- **Per-column `data_latency` is unimplemented**; the two interim mitigations
  (§"First-run and backfill") are the only options.
- **Non-deterministic row-set-membership or grouping is out of scope** — always rejected;
  admitting it needs a frozen-per-window-membership design
  (`docs/research/20260703-model-updates.md` §9.1a).
- **CTE-only `event_time_column` references are not yet detected**: a CTE alias that fails to
  project it escapes the outer-visibility check and fails at execution. Tracked:
  `docs/plans/20260616-smelt-feedback-fixes.md`.
- **Schema evolution on the partition grain is largely a definition delta now** — an output
  schema change is specified by `definition_deltas.md` (and unwired there, per its §Known
  Divergences); the residual open question here is a `partition_column` rename, a
  skeleton-position change whose refusal path has no fixture or diagnostic surfaced ahead of a
  run.
- **The `PartitionGrainForbidsMetrics` refusal is unimplemented** — §"Functions inside
  partition-grain bodies" refuses `smelt.metric()` in a partition-grain body, but no
  classifier or diagnostic produces the code today, so the combination's behaviour is
  effectively undefined at runtime. Decision record:
  `docs/research/20260816-open-questions-triage.md`.
- **Per-`ModelDef` overrides for generator-emitted models are not part of the closed field set
  in v1.** Tracked: `docs/plans/20260509-meta-language-overall.md`.
- **The sub-`g_part` rejection does not yet name the coarsened window** — §"Run window vs
  partition granularity" requires the refusal to spell out the run window that would be
  accepted; today it hard-rejects without the suggestion. (Reject-with-suggestion over
  auto-coarsening was decided 2026-08-16; `docs/research/20260816-open-questions-triage.md`.)
- **Monotone-integer `partition_column` has no end-to-end run** — the trace and bound
  derivation admit it, but run windows, backfill chunking, scan-filter injection, and the
  explain clamp rendering are date-typed throughout. Tracked:
  `docs/plans/20260704-model-updates-l4-batched.md`.
- **`NOW()`/`CURRENT_*` are still compile-time-pinned** — §"Safety checks" admits them running
  as-is with no equivalence promise on the columns they feed; the implementation still freezes
  them to one per-run timestamp. Decision record:
  `docs/research/20260816-open-questions-triage.md`.

### The key grain

- **A window-forward keyed run with no event-time window silently full-refreshes instead of
  refusing** — the CLI surface (`incremental_models.md` §"CLI") requires both
  `--event-time-start` and `--event-time-end`; the runtime's no-window arm instead drops and
  recreates the target from the whole-source SELECT (including when only one flag is supplied).
  The end state matches the full-refresh oracle, so nothing is silently wrong, but no test
  asserts the refusal, and user docs describe the fallback rather than the required-flags rule.
- **The once-write classifier has no nullability route around the fallback case** — the only
  route to decomposed `(value, written)` state is the FD-backed proof, since the NOT-NULL
  derivation proves not-null only for a partition/driving-clock-derived column; the key-derived
  route still requires a bare `unique_key` column reference, not an arbitrary key-derived
  *expression*; admission reads whole-scope fan-out/set-operation facts, so any fan-out or
  undiscriminated set operation anywhere in scope refuses every candidate. Decision record:
  `docs/research/20260705-keyed-collapse-application.md`; tracking:
  `docs/outcomes/20260809-rung2-state-shapes/outcome.md`,
  `docs/plans/20260705-keyed-collapse.md`, `docs/plans/20260809-keyed-frontier.md`.
- **Re-run-tolerant keyed models do not yet write the frontier** — §"The transactional
  frontier write (merge ledger)" has it written for every window-forward model whenever the
  project's state mode supports it; the runtime only creates the ledger table for
  additive-graded models, so a fully idempotent model has no merge record for `--auto` to
  consult. Decision record: `docs/research/20260816-open-questions-triage.md`.
- **`KeyedRetractableContribution` has no implementation (Open Question)** — the code is
  specified but no classifier, diagnostic variant, or test produces it.
- **`safety_overrides:` on a key-addressed model is not a hard error** — §"Key-grain
  declaration (`grain: key`)" makes it one, but frontmatter validation only checks the
  double-declaration case, so the block parses on a keyed model and is ignored.
- **The reconciliation ledger's fold is transactional on DuckDB only (Open Question)** — the
  default `Backend::fold_ledger_delta` is best-effort check-then-act across separate
  statements; only the DuckDB backend overrides it with a real transaction.
- **`smelt explain` prints neither the per-column guarantee ledger nor the derivable forward
  reach (Open Question)** — the cell/addressing/clamp/locality and edge sections are the whole
  of the rendered plan today.
- **Key temporal locality route 2 admits only a declared functional dependency (Open
  Question)** — the key-derived-expression sub-route is never consulted, so a provably
  key-derived partition projection still refuses without the declaration.
- **Locality machinery gaps**: the per-input scope-map explain surface is specified but
  unbuilt; Route 2's declared-FD sub-route is unreachable for an arbitrary
  non-clock-derived dimension column, so no runnable end-to-end route-2 fixture exists yet
  (`docs/plans/20260705-keyed-collapse.md`); Route 2's `IN (SELECT DISTINCT …)` slice
  predicate is unexercised against a real backend due to a DuckDB MERGE binder limitation
  (confirmed v1.4.4/v1.5.4); plan derivation admits routes only where it can determine the
  driving source's granularity; declared-vs-derived recurrence precedence and
  order-independent key-set comparison are implementation choices the spec text
  underdetermines. Tracked: `docs/plans/20260715-composed-axes-conditional-maintenance.md`.
- **The derived execution postures are internal, and one of the three is not derived at all** —
  order-independence is not derived as a named verdict anywhere: every window-forward run
  applies its windows sequentially regardless of family, forgoing the parallel/out-of-order
  application §"Derived execution postures" admits. Neither the derived run shape nor any
  posture is printed by `smelt explain`, which names that as their surface.
- **The generative conformance pool cannot stage NULL payloads (Open Question)** — the
  generated row type's payload field (`GenRow::val`) is a non-nullable `i64`, so the once-write
  family's NULL direction (a key whose first window carries only a NULL payload) is covered by
  one targeted test case rather than by the generated pool that proves every other keyed
  family.
- **The pattern-function template file does not exist** — `smelt.latest`, `smelt.once`, and
  `smelt.current` are specified as a shipped `smelt.define` template file (§"The column-family
  catalogue"), but no such file ships; each family is reachable only through its hand-written
  SQL spelling. Decision record: `docs/research/20260816-open-questions-triage.md`; tracked:
  `docs/plans/20260705-keyed-collapse.md`.
- **`NOW()`/`CURRENT_*` are still rejected in keyed models** — `KeyedForbidsNondeterministic`
  fires for them today, where §Diagnostics admits them in payload positions running as-is
  with no equivalence promise. Decision record:
  `docs/research/20260816-open-questions-triage.md`.
- **Departed keys are still retained under snapshot-reconcile** — §"Departed keys and
  deletion" has the reconcile write deleting a key absent from the incoming scan; the
  implementation retains it unchanged, forever. There are no production users, so the
  behaviour change ships without a compatibility path. The opt-in retention relaxation (the
  contract-lattice point for keeping departed keys as history) is likewise unbuilt. Decision
  record: `docs/research/20260816-open-questions-triage.md`; earlier analysis:
  `docs/research/20260705-keyed-collapse-application.md` §5.
- **Ladder rungs 3–4 remain specified ahead of this profile's use of them** — group-rung
  retraction (rung 3) and the bounded-domain multiset (rung 4) are out of scope for the rung-2
  work above; rung 3 additionally depends on the change-feed consumption design. Deferred by
  `docs/plans/20260809-keyed-frontier.md` §Scope,
  `docs/outcomes/20260809-rung2-state-shapes/outcome.md` §"Out of scope".
- **The `key_per_partition` grain derives no plan** — declaring it is refused at config parse,
  and the derived label (clock + identity with `partition_column ∈ unique_key`) refuses again
  at plan derivation (`MaintenanceUnsupportedGrain`); trajectory support is tracked by
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`.

## Future Extensions

Ideas for widening the admission space that are **not decided**. Nothing here is surface;
none of it may be relied on or implemented against until it graduates into §Surface/§Semantics
via its own spec diff. Deferral decisions recorded 2026-08-16:
`docs/research/20260816-open-questions-triage.md`.

- **Multi-source snapshot-reconcile.** Admitting a join of two or more unclocked snapshot
  sources needs a proven multi-source scan design; today the loud
  `KeyedSnapshotPostureUnsupported` refusal is the intended behaviour, revisited when a real
  workload hits it.
- **Self-referential keyed models** (`state = state + delta − decay`, the model reading its
  own previous output). Rejected by design: without an explicit input-vs-carried-state
  distinction the full-refresh oracle does not exist. Revisit after the posture-derived
  deletion doctrine (§"Departed keys and deletion") is implemented — deletion of carried
  state interacts directly.
- **Deletion-adjacent locality relaxations**, re-triaged together once posture-derived
  deletion is implemented: a derived recurrence bound licensing slice pruning under
  snapshot-reconcile; relaxing the granularity-equality precondition (a daily driver feeding
  weekly output partitions); slice-scoped deletion.
- **Wider driver granularities.** The shared windowed driver understands `day` and `week`;
  widening (month first, hour later) is driver work every consumer inherits, taken up when a
  workload demands it.
- **Exact `--auto` staleness for all-invertible models.** The current over-approximation is
  safe and accepted; "exactly the changed windows" needs the group rung's delta-history
  mechanism.
- **Promoting the pattern functions to built-ins.** `smelt.latest`/`smelt.once`/
  `smelt.current` ship as a `smelt.define` template file; a registry surface is worth its
  cost only if adoption proves the names.
- **Metric expansion in partition-grain bodies.** The `PartitionGrainForbidsMetrics` refusal
  stands until the composition of metric expansion with time-filter injection is specified,
  when metrics work resumes.

## References

### The partition grain

- **Code**:
  - `crates/smelt-core/src/config.rs` — `PartitionGrainConfig`, `Granularity`, `Weekday`
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction, `ModelMetadata`
  - `crates/smelt-logical/src/rules/incremental.rs` — partition-grain detection + safety checks (in `smelt-logical`; `smelt-planner` re-exports)
  - `crates/smelt-logical/src/types.rs` — safety-override types
  - `crates/smelt-runtime/src/transformer.rs` — `inject_time_filter`, `inject_source_filters`, `is_transparent_single_source`
  - `crates/smelt-backend/src/lib.rs` — `Backend::delete_partitions`, `Backend::insert_into_from_query`, `Backend::delete_and_insert_transactional` (per-chunk transaction boundary)
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `DeleteInsert` impl
  - `crates/smelt-dialect/src/dialect.rs` — `BackendCapabilities::supports_merge`
- **Tests**: batched safety unit tests in `crates/smelt-logical/src/rules/incremental.rs`; CLI
  integration tests in `crates/smelt-cli/tests/incremental_*.rs`; the per-partition
  full-refresh-equivalence harness
- **User docs**: [`docs-site/docs/guide/incremental-models.md`](../../docs-site/docs/guide/incremental-models.md), [`docs-site/docs/guide/materializations.md`](../../docs-site/docs/guide/materializations.md)
- **Plans (history)**:
  - [`docs/plans/20260322-incremental-model-support.md`](../plans/20260322-incremental-model-support.md) — comprehensive plan; many phases still open
  - [`docs/plans/20260325-materialization-types.md`](../plans/20260325-materialization-types.md)
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — the mode-vertical master the spec family re-cuts as a composition
  - [`docs/plans/20260707-maintenance-plan-impl.md`](../plans/20260707-maintenance-plan-impl.md) — lands the target frontmatter surface and diagnostics
- **Research**:
  - [`docs/research/20260521-incremental-as-planner-rule.md`](../research/20260521-incremental-as-planner-rule.md) — design direction this spec absorbs
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — batched eligibility audit; §9.2 non-determinism derivation
  - [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — the maintenance-framework design
  - [`docs/research/20260705-refresh-as-maintenance-plan/`](../research/20260705-refresh-as-maintenance-plan/) — the shape-profile demotion and per-cell admission this spec composes
- **Legacy reference**: `docs/DESIGN.md` §"Incremental Table Builds" — superseded for current
  behavior; useful for design rationale
- **Related specs**: `incremental_models.md` (the shared machinery this spec's profiles
  compose); `state.md` (state-ownership doctrine; the merge ledger's correctness
  classification); `definition_deltas.md`; `model_properties.md`; `model_transforms.md`; `models.md`;
  `timeseries.md`; `sources.md`; `expansion.md`; `functions.md`; `materialized_view.md`;
  `multi_backend.md`; `run_state.md`; `virtual_environments.md`; `diagnostics.md`;
  `architecture.md`; `cli.md`.

### The key grain

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy`);
  `crates/smelt-logical/src/rules/cumulative.rs` (the built classifier seed — combiner lookup,
  GROUP-BY key derivation, driving-source resolution);
  `crates/smelt-runtime/src/maintenance_driver.rs` (the windowed-keyed-maintenance driver,
  `WindowedKeyedRule`); `crates/smelt-runtime/src/cumulative.rs` (per-window merge execution);
  `crates/smelt-backend/src/lib.rs` (`merge_into`), impls in
  `crates/smelt-backend-duckdb`/`-spark`.
- **Tests**: the cumulative classifier unit tests (`smelt-logical/src/rules/cumulative.rs`);
  the keyed end-state-equivalence harness; `smelt-backend-duckdb` `merge_into` tests.
- **User docs**: `docs-site/docs/reference/cumulative-aggregate.md` (the key-grain reference
  page — column families, the once-write proof, the two run shapes, the diagnostic codes);
  `docs-site/docs/guide/materializations.md` (author-facing walkthrough);
  `docs-site/docs/guide/incremental-models.md` §"The composed shape (key + time)" documents the
  composed (key-addressed *and* time-partitioned) form and its three locality routes;
  `docs-site/docs/examples/web-analytics/deduplication.md` is the worked tutorial — a
  redelivery-prone feed deduplicated by a keyed extremal fold under a declared recurrence
  bound, contrasted against the partition-grain `QUALIFY`-window workaround the preceding
  tutorial page builds.
- **Plans (history)**: `docs/plans/20260523-cumulative-aggregate.md` (the built seed);
  `docs/plans/20260704-model-updates.md` (the mode-vertical master the spec family re-cuts as a
  composition); `docs/plans/20260705-keyed-collapse.md` (the keyed-collapse sub-plan);
  `docs/plans/20260707-maintenance-plan-impl.md` (lands the target frontmatter surface and
  diagnostics); `docs/plans/20260809-keyed-frontier.md` (the column-family union, the named
  ledger reprocessing refusal, and the snapshot-reconcile run shape).
- **Research**: `docs/research/20260705-keyed-time-superset.md` (key temporal locality, the
  time-partitioned output, per-input scope maps);
  `docs/research/20260705-model-refresh-review.md`;
  `docs/research/20260705-unified-keyed-refresh.md`;
  `docs/research/20260705-keyed-collapse-application.md` (the decision record this spec
  encodes); `docs/research/20260704-monotone-join-maintenance.md` (the monotone-vs-retractable
  boundary); `docs/research/20260703-model-updates.md`;
  `docs/research/20260705-refresh-as-maintenance-plan/` (the shape-profile demotion and
  per-cell admission this spec composes).
