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

The first nine rows are the **fold families** — additive (`COUNT`/`SUM`) and extremal/lattice (`MIN`, `MAX`, the boolean and bitwise combiners). Each is commutative and associative; that's the property that lets the rule merge windows in any order and still produce the same final state.

`MAX_BY`/`MIN_BY` is the **order-monotone overwrite family**. The ordering expression must also be projected in the same SELECT as its own running `MAX(ord)`/`MIN(ord)` column (or the value expression must *be* the ordering expression, as in `MAX_BY(x, x)`) — the merge compares the stored ordering value against the incoming one, so it has to be a column of the table. Without that companion projection the column is refused (`KeyedUnknownCombiner`, naming the missing projection). Ties keep the incumbent.

`COALESCE` is the **once-write family** — see [Once-write columns](#once-write-columns) below; it is admitted only when its provenance can be proven.

`ANY_VALUE` is the **plain-overwrite family** — see [Run shapes](#run-shapes) below: it is admitted only when the model has no clocked driving source (snapshot-reconcile); it is refused when a clocked source is present.

**Out of v1**: `AVG`, `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST`, `COUNT(DISTINCT ...)`, `APPROX_COUNT_DISTINCT`. Composite expressions over aggregates (e.g. `SUM(x) + 1`) are also refused — split into separate projections and compute derived values downstream.

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

**Declared functional dependency** — the coalesced value is a single-column `MAX(col)` or `MIN(col)` **with no fallback argument**, and the model declares that the grouping key determines `col`:

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

**No fallback after the reduction.** `COALESCE(MAX(col), 'unknown')` is refused, declaration or not. The merge is "first non-null wins", so the delta must be NULL exactly when the key has no value yet. A fallback makes it total: a window in which every row for a key has `col IS NULL` writes `'unknown'` into the target, and no later window can ever displace it — while a full refresh would return the real value that later window carried. The declaration says `col` is a per-key *constant*; it does not say `col` is never NULL, and this family is literally "first **non-null**", so NULLs within a key are expected. Drop the fallback and apply the default downstream instead:

```sql
-- downstream model, or a reader-side projection
SELECT device_id, COALESCE(first_referrer, 'unknown') AS first_referrer
FROM smelt.gold.device_first_touch
```

The key-derived shape above keeps its fallback: a key column is never NULL within its own group, so `COALESCE(device_id, 'n/a')` cannot mask a value a later window would supply.

A second candidate value (`COALESCE(MAX(a), MAX(b))`) is also refused. It preserves NULLs, but the cross-window merge does not preserve your preference for `a` over `b`: a window carrying only `b` writes `b`'s value and locks it in ahead of an `a` that arrives later.

Anything else — a coalesced expression that is neither of the two shapes, a bare non-key column, a multi-argument or non-`MAX`/`MIN` inner aggregate — is refused. The three fixes named by the diagnostic are: make the value key-derived, declare the functional dependency, or remodel the column out into its own model.

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

Running a window-forward model without a run window (`smelt run` without `--event-time-start`/`--event-time-end`) falls back to a single-shot full refresh: the target table is dropped and recreated from the SELECT over the entire source.

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
| `KeyedOnceWriteUnproven` | A `COALESCE` once-write column has no [provenance proof](#once-write-columns), or carries a fallback argument after its `MAX`/`MIN` reduction — names the column and the three fixes: make it key-derived, declare the functional dependency, or remodel the column into its own model; for the fallback case, drop it and apply the default downstream |

There is no `safety_overrides:` block for `grain: key` models. Rejected constructs break the end-state equivalence contract, not partial correctness — there is no opt-in escape hatch.

## Reprocessing

Reprocessing an already-merged window is refused when detected. If a past window's source data changes after the window has already been merged, the key-grain table is stale until the operator runs with `--full-refresh` (truncate and rebuild). Re-merging additive columns over an already-merged delta would double-count under a second pass; the rule refuses to silently double-count.

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
