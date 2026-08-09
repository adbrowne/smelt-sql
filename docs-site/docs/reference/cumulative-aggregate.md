# Key-grain patterns

`refresh: incremental` + `grain: key` is a stateful-merge shape: one row per `GROUP BY` key, where each row's columns reflect the combined state across every processed source window. The unique key and the per-column combiners are derived from the SELECT.

## Frontmatter

Use `materialization: table` together with `refresh: incremental` + `grain: key` to enable this shape:

```sql
---
materialization: table
refresh: incremental
grain: key
---
SELECT
    device_id,
    user_id,
    COUNT(*)      AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

There is no additional configuration block — the SQL is the entire specification.

## What's derived from the SQL

| Derived field | Comes from |
|---|---|
| `unique_key` | the `GROUP BY` column list |
| Per-column aggregator | each non-key projection's outer function |
| Cross-window combiner | a fixed lookup off the per-partition aggregator |
| Driving source | the single `timeseries:`-tagged source in the FROM clause |
| Run shape | whether that clocked source exists — one means window-forward, none means snapshot-reconcile (see [Run shapes](#run-shapes)) |

There is no way to override these — they are read from the SQL on every run.

## Aggregator allowlist

Each non-key projection must be a direct call to one of:

| Per-partition aggregator | Cross-window combiner | Rendered SQL |
|---|---|---|
| `COUNT(...)` | `SUM` | `target.c + delta.c` |
| `SUM(...)`   | `SUM` | `target.c + delta.c` |
| `MIN(...)`   | `MIN` | `LEAST(target.c, delta.c)` |
| `MAX(...)`   | `MAX` | `GREATEST(target.c, delta.c)` |
| `BOOL_AND(...)` | `BOOL_AND` | `target.c AND delta.c` |
| `BOOL_OR(...)`  | `BOOL_OR`  | `target.c OR delta.c` |
| `BIT_AND(...)`  | `BIT_AND`  | `target.c & delta.c` |
| `BIT_OR(...)`   | `BIT_OR`   | `target.c \| delta.c` |
| `BIT_XOR(...)`  | `xor()`    | `xor(target.c, delta.c)` |
| `MAX_BY(v, ord)` / `MIN_BY(v, ord)` | ordering value wins, incumbent on a tie | `CASE WHEN delta.ord > target.ord THEN delta.v ELSE target.v END` |
| `COALESCE(...)` | first non-null wins | `COALESCE(target.c, delta.c)` |
| `ANY_VALUE(...)` | incoming row wins | `delta.c` |
| `AVG(...)`, `STDDEV_*(...)`, `VAR_*(...)` | pairwise fold over hidden state | recomputed from the folded state on every read |

The first nine rows are the **fold families**: additive (`COUNT`, `SUM`, `BIT_XOR`) and extremal/lattice (`MIN`, `MAX`, the boolean combiners, `BIT_AND`/`BIT_OR`). Each is commutative and associative; that's the property that lets the rule merge windows in any order and still produce the same final state. The two differ on re-running a window that was already merged: the extremal/lattice combiners are idempotent, so a repeat merge converges, while the additive ones do not — `COUNT`/`SUM` would double-count, and `BIT_XOR` is self-inverse, so a repeat merge would *cancel* that window's contribution (`x xor d xor d == x`). Either way the result would diverge from a full refresh, which is why an additive model keeps a ledger and refuses a reprocessed window (see [Reprocessing](#reprocessing)).

`MAX_BY`/`MIN_BY` is the **order-monotone overwrite family**. No companion projection is required: the ordering expression's value is kept as internal state, invisible to consumers, and the merge compares the stored ordering value against the incoming one directly off that hidden state. Ties keep the incumbent. `smelt explain <model>` shows the hidden state a model stores (see [`smelt explain`](smelt-explain.md#internal-state-columns)).

`COALESCE` is the **once-write family** — see [Once-write columns](#once-write-columns) below; it is admitted only when its provenance can be proven. The fallback-bearing and multi-candidate spellings fold through hidden state too — `smelt explain <model>` shows it (see [`smelt explain`](smelt-explain.md#internal-state-columns)).

`ANY_VALUE` is the **plain-overwrite family** — see [Run shapes](#run-shapes) below: it is admitted only when the model has no clocked driving source (snapshot-reconcile); it is refused when a clocked source is present.

`AVG`/`STDDEV_*`/`VAR_*` are the **decomposed-fold family**. Each decomposes into hidden state columns that fold additively — `AVG(x)` into `(sum, count)`, the variance/stddev functions into `(n, Σx, Σx²)` — invisible to consumers; the presented value is recomputed from the folded state on every read rather than merged directly. Because the hidden state is additive, a decomposed-fold model keeps a ledger and refuses a reprocessed window, the same as the additive fold families above. Admitted window-forward only; refused under snapshot-reconcile for the same observer-semantics reason as the other aggregate families. `smelt explain <model>` shows the hidden state a model stores (see [`smelt explain`](smelt-explain.md#internal-state-columns)).

Composite expressions over aggregates (e.g. `SUM(x) + 1`) are refused for every family above — split into separate projections and compute derived values downstream. **Out of v1**: `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST`, `COUNT(DISTINCT ...)`, `APPROX_COUNT_DISTINCT`.

## Once-write columns

A once-write column keeps the **first non-null value ever written** for a key: the merge is `COALESCE(target.c, delta.c)`, so once a key's value is set no later window can change it. That is only equivalent to a full refresh when the value is genuinely a per-key constant — otherwise "first written" depends on which window happened to arrive first, and the run is no longer reorder-independent. The rule therefore demands a **provenance proof** before admitting the column, and refuses `KeyedOnceWriteUnproven` when it cannot get one.

Two shapes are admitted:

**Key-derived** — the coalesced value is a bare reference to one of the model's own `GROUP BY` key columns. It is a per-key constant by construction, so no declaration is needed:

```sql
SELECT
    device_id,
    COALESCE(device_id, 'n/a') AS first_seen_device
FROM smelt.silver.events_parsed
GROUP BY device_id
```

**Declared functional dependency** — the coalesced value is a single-column `MAX(col)` or `MIN(col)`, optionally followed by more `MAX`/`MIN` candidates and then a single fallback argument, and the model declares that the grouping key determines each candidate's column:

```sql
---
materialization: table
refresh: incremental
grain: key
functional_dependencies:
  - key: [device_id]
    determines: signup_referrer
---
SELECT
    device_id,
    COALESCE(MAX(signup_referrer)) AS first_referrer
FROM smelt.silver.events_parsed
GROUP BY device_id
```

The declaration is a world fact about the **source data**, not about the output: `determines` must name the source payload column inside the `MAX(...)`/`MIN(...)`, and `key` must name the columns the model actually groups on — not a projection's output alias. A declaration naming the output alias, or naming a column the model does not group by, proves nothing and does not admit. A `key` that is a *subset* of the grouping columns is a stronger statement and is accepted.

A declaration only widens the undecidable case; it never overrides a structural disproof. The column is still refused when the model's body contains a row-multiplying join, or an undiscriminated `UNION ALL` (a dependency holding in each branch need not hold in the union).

**The fallback is kept out of stored state.** `COALESCE(MAX(col), 'unknown')` admits: the merge never applies `COALESCE(target, delta)` to the fallback-tainted value directly. Instead the column folds through hidden decomposed state — the raw reduction and a `written` flag — and the fallback is applied fresh in a presentation expression on every read (`CASE WHEN written THEN value ELSE 'unknown' END`). Because the state's `value` column only ever holds the raw (possibly-NULL) reduction, a window whose rows all carry a NULL payload for a key never locks the fallback in: the real value a later window delivers still displaces it. Each candidate still needs its own provenance proof; the fallback itself must be a literal or a reference to one of the model's own `GROUP BY` key columns (anything else cannot be recomputed purely from the stored row on every read, and is refused).

Multiple candidates are also admitted, applied in the order written: `COALESCE(MAX(a), MAX(b), 'unknown')` keeps a `(value, written)` state pair per candidate and prefers `a` over `b` over the fallback on every read, so the cross-window merge no longer needs to preserve a preference order itself — each candidate's own state is independently first-non-null. Every candidate still needs its own declared functional dependency (or must itself be key-derived); the first candidate whose provenance cannot be proven refuses the whole column, naming that candidate.

The key-derived shape above keeps its fallback unconditionally: a key column is never NULL within its own group, so `COALESCE(device_id, 'n/a')` cannot mask a value a later window would supply, and needs no decomposed state.

Anything else — a coalesced expression that is neither of the two shapes, a bare non-key column, a candidate that is a multi-argument or non-`MAX`/`MIN` inner aggregate, or more than one argument following the last candidate — is refused. The three fixes named by the diagnostic are: make the value key-derived, declare the functional dependency, or remodel the column out into its own model.

A `COALESCE(...)` used as a null-safe composite `GROUP BY` key (`GROUP BY COALESCE(device_id, 'n/a')`) is a key column, not a once-write column, and needs no proof.

Once-write columns are admitted **window-forward only**; under snapshot-reconcile they are refused (`KeyedSnapshotSourceUnsupportedColumn`) — "first observed over history" is not a statement about the current snapshot.

## Run shapes

The run shape is derived from the FROM clause, never declared:

- **Window-forward** — exactly one `timeseries:`-tagged source in the FROM clause (the driving source). Folds (`SUM`, `MIN`, `MAX`, …), the order-monotone overwrite family (`MAX_BY`/`MIN_BY`), and the once-write family (`COALESCE`, given its provenance proof) are admitted here; `ANY_VALUE` is refused.
- **Snapshot-reconcile** — zero clocked sources. The model re-scans its source whole on every run instead of stepping over partitions. `ANY_VALUE` (plain overwrite, incoming row wins) is admitted here; folds, `MAX_BY`/`MIN_BY`, and `COALESCE` once-write columns are refused — re-folding a mutable snapshot double-counts (additive) or computes a value observed over history rather than the current one (extremal, order-monotone, once-write).
- Two or more clocked sources refuses (`KeyedMultipleDrivingSources`).

## Execution

**Window-forward**, for a run window `[run_start, run_end)`:

1. Classify the model SQL and derive the unique key, per-column combiners, and driving source.
2. Step over the driving source's partitions in temporal order. For each partition `D`:
    - Inject `<driving_source>.<partition_col> ∈ [D, D + granularity)` onto the driving source reference.
    - Compile the per-partition delta SELECT and run it through the engine.
    - First partition: `CREATE TABLE AS` the delta. Subsequent partitions: emit a `MERGE INTO` with the per-column combiners.

!!! warning "Granularity restriction"
    The driving source must declare `granularity: day` or `granularity: week`. Any other granularity — `hour`, `month`, `quarter`, or `year` — is rejected at runtime with the error `windowed-keyed-maintenance driver supports day and week granularity; got <Granularity>`.

A window-forward model expects both `--event-time-start` and `--event-time-end`; they address the *driving source's* partition column. Running it without them (or with only one) is not an incremental run at all — it currently falls back to a single-shot full refresh: the target table is dropped and recreated from the SELECT over the entire source. Supply both flags whenever you mean to maintain the table rather than rebuild it.

**Snapshot-reconcile** models never take `--event-time-start`/`--event-time-end` — supplying either is rejected fail-loud, naming the run shape. Every run re-scans the source whole: the first run creates the target table from the SELECT; every subsequent run `MERGE`s the whole-source scan into the existing target — matched keys are overwritten (incoming row wins), unmatched keys inserted, and a key present in the target but **absent** from the incoming scan is **retained unchanged** (there is no `DELETE`; removing a stored row entirely needs an explicit mechanism, out of scope today). No reconciliation ledger is kept — each run is a self-contained reconciliation.

## End-state equivalence

For any set of source partitions `S = {D₁, …, Dₙ}` and any admitted ordering π over `S`:

```
key_grain_run(model, π(S))
  == full_refresh(model, source.where(partition ∈ S))
```

Reordering merges across source partitions does not change the final state: for the additive and extremal/lattice folds because their combiners are commutative and associative, for `MAX_BY`/`MIN_BY` up to ordering-value ties, and for once-write columns because the proof makes the value a per-key constant, so "first written" cannot depend on arrival order. This is the load-bearing contract `grain: key` upholds — and the reason each column family is admitted only on terms that preserve it.

## Diagnostic codes

| Code | When it fires |
|---|---|
| `KeyedRequiresGroupBy` | SELECT has no `GROUP BY` — there is no unique key to derive |
| `KeyedUnknownCombiner` | A non-key projection is not a direct call to an allowlisted aggregator |
| `KeyedGroupByContainsPartitionColumn` | `GROUP BY` contains the driving source's `partition_column` (would produce the partition-grain shape, not the key-grain one) |
| `KeyedForbidsWindowFunctions` | Outer-body `OVER (...)` clause |
| `KeyedForbidsNondeterministic` | Non-deterministic function in the outer body (`NOW()`, `RANDOM()`, …) |
| `KeyedMultipleDrivingSources` | More than one `timeseries:`-tagged source in the FROM clause |
| `KeyedForbidsTimeseries` | A `grain: key` model declares a `timeseries:` block but none of the three [key temporal locality](../guide/incremental-models.md#the-composed-shape-key-time) routes admits it |
| `KeyedSnapshotPostureUnsupported` | No clocked driving source, and no single unambiguous source could be resolved to derive the snapshot-reconcile run shape either (e.g. more than one candidate source, none clocked) |
| `KeyedSnapshotSourceUnsupportedColumn` | A fold-family, order-monotone-overwrite, or once-write column is used under the snapshot-reconcile run shape — re-fold this family with `--event-time-start`/`--event-time-end` over a `timeseries:`-tagged source instead, or express the column as `ANY_VALUE(...)` |
| `KeyedOnceWriteUnproven` | A `COALESCE` once-write column — bare key-derived, single-reduction, fallback-bearing, or multi-candidate — has no [provenance proof](#once-write-columns) for one or more of its candidate columns, or its fallback is not a literal or a `GROUP BY` key reference — names the unproven candidate (or the fallback) and the three fixes: make it key-derived, declare the functional dependency, or remodel the column into its own model |
| `KeyedSqlNotParseable` | The model SELECT could not be parsed into the shape the classifier reads |
| `KeyedReprocessedWindow` | A run window covers a window this model already merged, and the model is not re-run tolerant (it has an additive column) — see [Reprocessing](#reprocessing) |
| `KeyedRecurrenceBoundViolated` | Runtime, declared-`key_recurrence` route only: a merged delta row matched a stored key outside the run's derived slice, violating the source's declared bound. The run's transaction rolls back |

There is no `safety_overrides:` block for `grain: key` models. Rejected constructs break the end-state equivalence contract, not partial correctness — there is no opt-in escape hatch.

## Reprocessing

Reprocessing an already-merged window is refused when detected, with the error `KeyedReprocessedWindow` naming the model, the partition, the window bounds, and the `--full-refresh` remedy. A model carrying an additive (`COUNT`/`SUM`/`BIT_XOR`) column is not re-run tolerant: each merged window is recorded in a small per-model reconciliation-ledger table, written in the same backend transaction as the merge itself, and a run whose window is already recorded is refused rather than folded a second time. A model whose columns are all idempotent (`MIN`/`MAX`, `MAX_BY`/`MIN_BY`, once-write) needs no ledger — re-merging a window converges to the same state — so none is created for it and no window is refused.

If a past window's source data changes after the window has already been merged, the key-grain table is stale until the operator runs with `--full-refresh` (truncate and rebuild). Snapshot-reconcile models keep no ledger at all: each run is a self-contained reconciliation, so there is nothing to reprocess.

## Output shape

A `grain: key` model's output has:

- One row per `unique_key` value (the `GROUP BY` column list).
- Per-key columns whose values reflect the combined state across every processed source window.
- By default: **no** `partition_column`, **no** `event_time_column`, and **no** `timeseries:` declaration on the model itself.
- A model **may** additionally declare `timeseries:` to time-partition its keyed output — the composed (key + time) shape — when key temporal locality can be established; see [Timeseries reference](timeseries.md#interaction-with-grain-key) and the [composed-shape guide](../guide/incremental-models.md#the-composed-shape-key-time).

Downstream consumers see the key-grain output as a lookup — there is no partition information to push down. Joins to the table read it in full each run, identical to the treatment of any non-`timeseries:` source.

## Related references

- [Materializations guide](../guide/materializations.md#refresh-axis) — author-facing walkthrough.
- [Incremental Models](../guide/incremental-models.md) — the sibling shape (`grain: partition`) for per-partition output.
- [Timeseries reference](timeseries.md) — `timeseries:` block declared on the *source* a `grain: key` model reads from.
