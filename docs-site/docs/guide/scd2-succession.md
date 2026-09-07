# The succession grain (SCD2-style history)

A **succession** model turns an append-only change stream into a keyed history table — one row
per key per event, each row's validity window closed out by the row that follows it for the same
key. This is the classic slowly-changing-dimension type-2 (SCD2) pattern, but smelt never asks
you to declare it: the shape is **recognised from the model's own SQL**, the same way the key
grain and the partition grain are recognised from a declared `unique_key:` or `timeseries:`.

## Why it needs no declaration

A succession model's identity is the pair `(k, t)` — a key and the clock value at which that
key's row was recorded. There is no `GROUP BY`, so there is no aggregation to declare a
`unique_key:` for; there is no model-level `timeseries:` to declare either, since the clock
comes from the *driving source's* declared clock. The one fact left to communicate is the SQL
shape itself — a window-function projection over an append-only source — and smelt's
keyed-succession classifier reads it directly.

## The admitted SQL outline

```
SELECT <row-local columns, including k and t>,
       <LEAD(t)/LAG(t) OVER (PARTITION BY k ORDER BY t) and scalar expressions over them>
FROM smelt.<one append-only, clocked source>
[WHERE <one deterministic row-local predicate>]   -- optional lateness clamp
[QUALIFY NOT <a NOT NULL boolean column>]          -- optional delete-flag filter
```

and nothing else: no join, CTE, subquery, or set operation in `FROM`; no `DISTINCT`,
`GROUP BY`, `HAVING`, `ORDER BY`, or `LIMIT`; no aggregate, ranking, or frame-based window.
`customer_history` in [`examples/scd2_succession/`](https://github.com/adbrowne/smelt-sql/tree/main/examples/scd2_succession)
is the worked example:

```sql
---
refresh: incremental
---
SELECT
    customer_id,
    tier,
    region,
    effective_ts AS valid_from,
    LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) AS valid_to,
    LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) IS NULL AS is_current
FROM smelt.sources.customer_changes
QUALIFY NOT is_deleted
```

`customer_id` is the key `k`, `effective_ts` is the clock `t`. The `LEAD` derives `valid_to` (and,
via a scalar expression over it, `is_current`) by looking one event ahead in each customer's own
sequence. No `unique_key:` or `timeseries:` appears anywhere in this frontmatter — the grain is
derived, not declared.

## The derived `(k, t)` identity

The output has one row per `(customer_id, effective_ts)` pair — every event a customer generates
produces its own history row. This differs from the key grain (one row per key, latest value
wins) and the partition grain (one row per time-partitioned slice): a succession table's rows
accumulate, they don't collapse.

## The delete filter

A CDC delete flag must still close out its predecessor's validity interval before the deleted
row itself disappears from the output — that's why the filter is a `QUALIFY`, evaluated *after*
the window functions run, rather than a `WHERE` evaluated before them. `QUALIFY NOT is_deleted`
lets `LEAD(effective_ts)` see the delete event (so the previous row's `valid_to` closes
correctly) and only then drops it from the materialized table. Writing `WHERE NOT is_deleted`
instead is accepted as a legitimate pre-window clamp, but it means something different: the
delete's timestamp never reaches its predecessor, so the previous version is left open forever.
Because that's the classic mistake, smelt warns (`SuccessionPreFilterNegatesFlag`) whenever a
pre-window filter is a bare negated boolean column, without refusing the model — you may have
meant it as an ordinary lateness clamp.

## The optional pre-window clamp

A single deterministic, row-local `WHERE` before the window functions bounds which events count
at all — for example, dropping versions that land implausibly late:

```sql
FROM smelt.sources.customer_changes
WHERE ingested_at < effective_ts + INTERVAL '7 days'
```

A row the clamp drops never enters the sequence, in the oracle or in the maintained state, so
equivalence holds by construction. Nothing requires a clamp — the write footprint is the same
either way — it's a semantic choice, not a performance one.

## Arrival- vs event-time partitioning

The driving source's `timeseries.partition_column` is the model's **run axis** — what a run
window covers. The clock in the window functions' `ORDER BY` is the **succession clock**, and it
must trace back to the source's declared `event_time_column`. These two columns may differ, and
which case you're in decides how late events reach the grain:

- **Arrival-partitioned** — the source's `partition_column` is a landing/ingest date, distinct
  from its `event_time_column`. `customer_changes` in the worked example partitions by
  `ingested_date` but clocks by `effective_ts`. A late event is simply an event in a later
  arrival window — no special handling.
- **Event-time-partitioned** — `partition_column` and `event_time_column` are the same column. A
  late event surfaces as an observed delta on an already-closed partition, and that partition's
  window is safely re-presented (the grain is re-run tolerant — see below).

`smelt explain <model>` prints which posture a model has, so you can see whether a late event
costs a closed-partition re-presentation.

Both postures are re-run tolerant and order-independent (but never concurrent): a window may be
folded again, or windows may apply out of temporal order, and the result is always the same as
folding them in order exactly once.

## The tombstone ledger

Beside the presented table, a succession model keeps one piece of hidden state: the
**tombstone ledger**, a table holding `(k, t)` for every delete event ever folded. Non-delete
events are represented by their presented rows alone; the ledger exists only because a delete
event has no presented row, yet its timestamp is still load-bearing for its predecessor's
`valid_to`. `smelt explain` reports the ledger as internal state, not part of the model's public
schema — you never query it directly, and it degrades together with the presented table on
`--full-refresh` or `smelt repair`.

## What `smelt explain` shows

```
$ smelt explain customer_history
model customer_history  (emits: event history keyed by [customer_id], event-addressed by (customer_id, effective_ts))
  grain: succession
  identity: (customer_id, effective_ts)
  technique: succession-patch
  run axis: ingested_date (arrival-partitioned)
  clock: effective_ts
  posture: re-run tolerant; order-independent but serial
  internal state: tombstone ledger customer_history__tombstones (customer_id, effective_ts) — not part of the model's public schema
```

`smelt explain <model> --json` carries the same facts under a `succession` object
(`key_columns`, `clock_column`, `run_axis`, `partitioning`, `lead_columns`, `lag_columns`,
`delete_flag`, `pre_window_filter`, `tombstone_ledger`, and the `rerun_tolerant`/
`order_independent`/`concurrent` execution postures), plus a `delta_signature` object with
`shape: "keyed_succession"` and `addressing: "event"`.

## Refusal codes

Every clause outside the admitted outline is refused by a named diagnostic, never silently
demoted to another grain. Each names the offending clause and a fix.

| Code | Fix |
|---|---|
| `SuccessionWindowFunctionNotLead` | Only `LEAD(t)`/`LAG(t)` over the clock column, default offset, are admitted — remove the extra offset/default argument, or window over the clock column instead. |
| `SuccessionPartitionKeyMismatch` | Make every window function partition by the same, `NOT NULL`-proven key set. |
| `SuccessionOrderNotMonotoneClock` | `ORDER BY` must be the single, ascending, `NOT NULL` column that traces to the source's `event_time_column` — remove extra sort keys or `DESC`. |
| `SuccessionRowLocalColumnViolation` | Every non-window column must be row-local — remove the aggregate or nested window function, or move it into a window expression. |
| `SuccessionIdentityNotProjected` | Project the key column and the clock column row-locally so `(k, t)` is recoverable from the output. |
| `SuccessionSingleSourceOnly` | Remove the join/CTE/subquery/set operation — the `FROM` clause must be exactly one source. |
| `SuccessionDrivingSourceNotAppendOnly` | Declare `mutation_profile: append_only` and a `timeseries:` block on the driving source. |
| `SuccessionPreFilterNotRowLocal` | The pre-window `WHERE` must be one deterministic, row-local predicate over the source's own columns. |
| `SuccessionDeleteFilterMisplaced` | Use exactly `QUALIFY NOT <flag>` for a `NOT NULL` boolean delete flag, not a `WHERE`. |
| `SuccessionPreFilterNegatesFlag` (warning) | Advisory only — if the negated column is a delete flag, prefer `QUALIFY NOT <col>` so its timestamp still closes its predecessor's interval. |
| `SuccessionPatternUnrecognized` | The model matches no admitted grain — declare `unique_key:`/`timeseries:` to reach another grain, or use `refresh: full`/`refresh: materialized_view`. |
| `SuccessionClockTie` (runtime) | Two non-identical events (or a delete and a non-delete) landed at the same `(k, t)` — the run rolls back; resolve the upstream duplicate/collision. |

See the [diagnostics reference](../reference/diagnostics.md#succession-grain) for full
descriptions, and [`docs/specs/incremental_shapes.md` §"The succession grain"](../../../docs/specs/incremental_shapes.md)
for the normative spec.
