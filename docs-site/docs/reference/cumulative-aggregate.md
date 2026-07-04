# cumulative_aggregate Reference

The cumulative-aggregate merge loop is a stateful-merge materialization. The output has one row per `GROUP BY` key, where each row's columns reflect the combined state across every processed source partition. The unique key and the per-column combiners are derived from the SELECT.

## Frontmatter

Use `materialization: table` together with `refresh: cumulative` to enable this mode:

```sql
---
materialization: table
refresh: cumulative
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
| Cross-partition combiner | a fixed lookup off the per-partition aggregator |
| Driving source | the single `timeseries:`-tagged source in the FROM clause |

There is no way to override these — they are read from the SQL on every run.

## Aggregator allowlist

Each non-key projection must be a direct call to one of:

| Per-partition aggregator | Cross-partition combiner | Rendered SQL |
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

Each allowed aggregator is commutative and associative — that's the property that lets the rule merge partitions in any order and still produce the same final state.

**Out of v1**: `AVG`, `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST`, `COUNT(DISTINCT ...)`, `APPROX_COUNT_DISTINCT`. Composite expressions over aggregates (e.g. `SUM(x) + 1`) are also refused — split into separate projections and compute derived values downstream.

## Execution

For a run window `[run_start, run_end)`:

1. Classify the model SQL and derive the unique key, per-column combiners, and driving source.
2. Step over the driving source's partitions in temporal order. For each partition `D`:
    - Inject `<driving_source>.<partition_col> ∈ [D, D + granularity)` onto the driving source reference.
    - Compile the per-partition delta SELECT and run it through the engine.
    - First partition: `CREATE TABLE AS` the delta. Subsequent partitions: emit a `MERGE INTO` with the per-column combiners.

!!! warning "Granularity restriction"
    The driving source must declare `granularity: day` or `granularity: week`. Any other granularity — `hour`, `month`, `quarter`, or `year` — is rejected at runtime with the error `cumulative_aggregate v1 supports day and week granularity; got <Granularity>`.

Running without a run window (`smelt run` without `--event-time-start`/`--event-time-end`) falls back to a single-shot full refresh: the target table is dropped and recreated from the SELECT over the entire source.

## Cross-partition equivalence

For any set of source partitions `S = {D₁, …, Dₙ}` and any ordering π over `S`:

```
cumulative_aggregate_run(model, π(S))
  == full_refresh(model, source.where(partition ∈ S))
```

Reordering merges across source partitions does not change the final state. This is the load-bearing contract the rule upholds — and the reason the allowlist is restricted to commutative-associative aggregators.

## Diagnostic codes

| Code | When it fires |
|---|---|
| `CumulativeRequiresGroupBy` | SELECT has no `GROUP BY` — there is no unique key to derive |
| `CumulativeUnknownAggregator` | A non-key projection is not a direct call to an allowlisted aggregator |
| `CumulativeGroupByContainsPartitionColumn` | `GROUP BY` contains the driving source's `partition_column` (would produce the per-partition shape, not the cumulative one) |
| `CumulativeForbidsWindowFunctions` | Outer-body `OVER (...)` clause |
| `CumulativeForbidsNondeterministic` | Non-deterministic function in the outer body (`NOW()`, `RANDOM()`, …) |
| `CumulativeNoDrivingSource` | No source in the FROM clause declares `timeseries:` |
| `CumulativeMultipleDrivingSources` | More than one `timeseries:`-tagged source in the FROM clause |
| `CumulativeForbidsTimeseries` | Model declares both `refresh: cumulative` and a `timeseries:` block |
| `CumulativeForbidsBatched` | Model declares both `refresh: cumulative` and a `batched:` block |

There is no `safety_overrides:` block for cumulative models. Rejected constructs break the cross-partition equivalence contract, not partial correctness — there is no opt-in escape hatch.

## Reprocessing

v1 does not support per-partition reprocessing. If a past partition's source data changes after the partition has already been merged, the cumulative table is stale until the operator runs with `--full-refresh` (truncate and rebuild). Per-key combine plus an already-merged delta would double-count under a second pass; the rule refuses to silently double-count.

## Output shape

A cumulative aggregate model's output has:

- One row per `unique_key` value (the `GROUP BY` column list).
- Per-key columns whose values reflect the combined state across every processed source partition.
- **No** `partition_column`. **No** `event_time_column`. **No** `timeseries:` declaration on the model itself.

Downstream consumers see the cumulative output as a lookup — there is no partition information to push down. Joins to the cumulative table read it in full each run, identical to the treatment of any non-`timeseries:` source.

## Related references

- [Materializations guide](../guide/materializations.md#cumulative_aggregate) — author-facing walkthrough.
- [Incremental Models](../guide/incremental-models.md) — the sibling materialization for per-partition output.
- [Timeseries reference](timeseries.md) — `timeseries:` block declared on the *source* a cumulative model reads from.
