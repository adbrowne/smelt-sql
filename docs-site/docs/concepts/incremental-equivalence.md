# Per-partition equivalence

Every incremental model in smelt upholds a formal correctness contract called
**per-partition equivalence**:

```
incremental_run(model, [run_start, run_end))
  .where(partition_column = p)
== full_refresh(model).where(partition_column = p)
```

For any partition value `p` in the run window, running the model incrementally
for that window and then filtering to `p` must produce the same rows as a
full-table rebuild filtered to `p`. This holds regardless of run-window size:
a single 60-day run and 60 successive one-day runs produce equivalent
per-partition output.

## Local vs global columns

The equality holds for **local** columns — columns whose value depends only on
source rows visible within the model's source-filter ranges for that partition.

**Global** columns — columns whose value depends on source rows _outside_ the
current partition's filter range — are not covered by this contract. Their
per-partition value in an incremental run reflects the state of the input data
at the time that partition was written, not the final global state after all
partitions are processed.

| Column kind | Incremental vs full-refresh |
|---|---|
| Local — no cross-partition aggregation | Always equal per partition |
| Global — cumulative aggregation or cross-partition join | May differ per partition (as-of-day-D) |

## The as-of-day-D property

When an incremental model writes a partition for date D, it uses whatever
cumulative state is available at run time. If a later run (D+1, D+2, …)
introduces new data that would have changed day D's global aggregation, day D's
rows are **not** retroactively rewritten: smelt's DELETE+INSERT-per-partition
strategy only rewrites the current run's target partitions.

This is the **as-of-day-D** property. It mirrors how production streaming
pipelines emit daily snapshots: each day's output reflects the world as it was
known at the end of that day, not as it is known today.

A single full-window rebuild, by contrast, uses the complete accumulated data
in one pass and produces a global-final snapshot for every partition.

## Example: web analytics identity resolution

The [`examples/web_analytics`](https://github.com/andrewbrowne/smelt-sql/tree/main/examples/web_analytics)
pipeline demonstrates this property concretely.

The `marts/daily_active_users_by_method` model has both local and global
columns:

| Column | Kind | Day-by-day vs full-window |
|---|---|---|
| `total_events` | local | always equal |
| `dau_raw` | local | always equal |
| `dau_forward_only` | local | always equal |
| `identified_events_raw` | local | always equal |
| `identified_events_forward_only` | local | always equal |
| `dau_backward_fill` | global | usually differ |
| `dau_connected_components` | global | usually differ |
| `identified_events_backward_fill` | global | usually differ |
| `identified_events_connected_components` | global | usually differ |

`backward_fill` and `connected_components` depend on the cumulative
`(device, user)` edge set across all dates. When day D's run materialises the
eventstream, the LEFT JOINs to the identity views see only the edges visible up
to D. Day D+1 may add edges that would have changed day D's identity mapping,
but day D's rows are not retroactively rewritten.

The local columns (`raw`, `forward_only`) depend only on events within a single
session or a single partition's source-filter range; they are unaffected by
later edge additions and remain exactly equal between the two pipeline modes.

## CI verification

The Rust integration test
`crates/smelt-cli/tests/per_partition_equivalence.rs` verifies this property
on every CI run. It runs both pipelines (full-window single rebuild and a
7-day day-by-day replay) against the `examples/web_analytics` example and
asserts:

1. **Local columns are exactly equal** per partition — any mismatch is a bug.
2. **Global columns diverge** on at least one partition — verifying the
   as-of-day-D property is observable rather than accidentally vacuous.

The test uses a 7-day window (2026-03-19 .. 2026-03-26) at scale-factor 0.01
to keep CI runtime under ~30 s while still exercising cross-device identity
links.

The Python script `examples/web_analytics/verify_incremental_equivalence.py`
provides the same verification with configurable `--days` and `--scale-factor`
flags for manual exploration.
