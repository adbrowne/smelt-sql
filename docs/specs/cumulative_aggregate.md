---
feature: cumulative_aggregate
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Cumulative Aggregate Materialization

> **What this is.** A normative spec for the `cumulative_aggregate` materialization — a stateful-merge planner rule that collapses a timeseries source into one row per key, where each row reflects state across all processed source partitions. Covers the frontmatter selector, the classifier, the per-partition delta-SELECT shape, the cross-partition combine semantics, the equivalence contract, and the rules around what may be expressed. Out of scope: incremental DELETE+INSERT (`incremental_models.md`), the `timeseries:` declaration this rule consumes from its source (`timeseries.md`), full model frontmatter schema (`models.md`), the backend `merge_into` primitive (described in `architecture.md` §"Backend primitives" — `cumulative_aggregate` is one caller).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).

## Surface

### YAML frontmatter (in `.sql` files)

```sql
---
materialization: cumulative_aggregate
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

The materialization name is the entire opt-in. `cumulative_aggregate` is a sibling choice alongside `view`, `table`, `materialized_view`, `ephemeral`, `test`, and `incremental`. No other frontmatter key is read or required by the rule.

`materialization: cumulative_aggregate` **forbids** a `timeseries:` block on the model — the output has no partition column (Semantics §"Output shape"). `materialization: cumulative_aggregate` **forbids** an `incremental:` block on the model — the two are different rules with different equivalence contracts (`incremental_models.md`).

### `smelt.yml` (project-level overrides)

```yaml
models:
  device_user_edges:
    materialization: cumulative_aggregate
```

Frontmatter wins over `smelt.yml` when both set `materialization`. The same forbid-`timeseries:` / forbid-`incremental:` constraints apply.

### CLI

`cumulative_aggregate` consumes the same `--event-time-start`/`--event-time-end` flags as incremental execution — the run window names the source partitions that will be merged in. Format and alignment rules follow `incremental_models.md` §"CLI". The flags apply to the driving source's `partition_column` / `granularity` (Semantics §"Driving source"), not to any column on the cumulative output.

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

### Aggregator allowlist

The classifier accepts non-key projections that are direct calls to one of:

| Per-partition aggregator | Cross-partition combiner |
|---|---|
| `COUNT(...)` | `SUM` |
| `SUM(...)` | `SUM` |
| `MIN(...)` | `MIN` |
| `MAX(...)` | `MAX` |
| `BOOL_AND(...)` | `BOOL_AND` |
| `BOOL_OR(...)` | `BOOL_OR` |
| `BIT_AND(...)` | `BIT_AND` |
| `BIT_OR(...)` | `BIT_OR` |
| `BIT_XOR(...)` | `BIT_XOR` |

Any other aggregate function, or any non-aggregate non-key expression, in the projection list is rejected by the classifier (Semantics §"Classifier checks"). The combiner column is a fixed lookup off the per-partition aggregator name; authors do not declare combiners.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `CumulativeRequiresGroupBy` | Error | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `CumulativeForbidsTimeseries` | Error | The model declares both `materialization: cumulative_aggregate` and a `timeseries:` block. |
| `CumulativeForbidsIncremental` | Error | The model declares both `materialization: cumulative_aggregate` and an `incremental:` block. |
| `CumulativeUnknownAggregator` | Error | A non-key projection is not a direct call to an aggregator in the allowlist. The diagnostic names the offending aggregator and points at the projection. |
| `CumulativeGroupByContainsPartitionColumn` | Error | The `GROUP BY` list contains the driving source's `partition_column`. The diagnostic suggests switching to `materialization: incremental` + `timeseries:` instead. |
| `CumulativeForbidsWindowFunctions` | Error | The outer SELECT body uses `OVER (...)`. The cumulative state *is* the window; window functions in cumulative SQL are nonsensical. |
| `CumulativeNoDrivingSource` | Error | No `smelt.<path>` reference in the FROM clause has a `timeseries:` declaration on the resolved target. |
| `CumulativeMultipleDrivingSources` | Error | More than one timeseries-tagged source appears in the FROM clause. The diagnostic lists the candidate sources. |
| `CumulativeForbidsNondeterministic` | Error | The SQL uses `NOW()`, `RANDOM()`, or other non-deterministic functions outside stable contexts. Cross-partition combine requires deterministic per-partition output. |

## Semantics

### Execution model

For a `cumulative_aggregate` model with a run window `[run_start, run_end)`:

1. **Classify the model's SQL** (§"Classifier checks") and derive:
   - `unique_key` — the columns named in `GROUP BY`.
   - `aggregator_columns` — a map from each non-key projection's output column name to its `(per_partition_agg, cross_partition_combiner)` pair, looked up from the Surface §"Aggregator allowlist" table.
   - `driving_source` — the single timeseries-tagged source in the FROM clause (Semantics §"Driving source").
2. **Step over source partitions** in temporal order. For each source partition `D` covered by the run window:
   - **Source-filter pushdown** injects `<driving_source>.<partition_column> >= D AND <driving_source>.<partition_column> < D + granularity` onto the driving source's reference. Sources without `timeseries:` are not pushdown candidates; they are read in full each partition.
   - **Execute the per-partition delta SELECT** through the engine, producing one delta row per `unique_key` value present in this partition's input.
   - **Backend `merge_into` call** with the derived `unique_key` and a per-column combiner map. Matched rows: target's value is combined with delta's value via the cross-partition combiner (`target.event_count + delta.event_count`, `LEAST(target.first_seen, delta.first_seen)`, `GREATEST(target.last_seen, delta.last_seen)`, etc.). Unmatched rows: insert as-is.
3. If the output table does not exist when the first partition is merged, the rule creates it from the first partition's delta SELECT (`CREATE TABLE AS SELECT`); subsequent partitions are merged into it.

A run window covering N source partitions performs N `merge_into` calls in temporal order. Each `merge_into` is one backend transaction. Earlier committed partitions do not roll back on a later failure — partial progress is intentional. Re-running the same `[run_start, run_end)` over unchanged source data converges to the same final state **only for idempotent combiners** (`MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR` — where re-applying an already-merged partition's delta is a no-op). `SUM`, `COUNT`, and `BIT_XOR` are **not** idempotent: re-merging an already-applied partition double-counts, so retry-after-partial-failure for these aggregators relies on the reprocessing machinery in Semantics §"Reprocessing semantics", not a blind re-run.

### Output shape

A cumulative aggregate model's output has:

- One row per `unique_key` value (where `unique_key` is the `GROUP BY` column list).
- Per-key columns whose values reflect the cross-partition combine of every processed source partition's contribution.
- **No** `partition_column`. **No** `event_time_column`. **No** `timeseries:` declaration on the model itself.

Downstream consumers see the cumulative output as a lookup table — there is no partition information to push down. Downstream models join to it and read it in full each run, identical to the treatment of any non-timeseries source (`incremental_models.md` §"Source-filter pushdown").

### Driving source

The classifier walks the inlined outer SELECT's FROM clause (after function expansion, per `expansion.md`) and collects every `smelt.<path>` reference whose resolved target declares a `timeseries:` block. The result must be exactly one such source — the **driving source**.

| Cardinality of timeseries-tagged sources | Outcome |
|---|---|
| 0 | Rejected: `CumulativeNoDrivingSource`. The error message suggests declaring `timeseries:` on the source or switching the materialization. |
| 1 | Accepted. The driving source's `partition_column` and `granularity` parameterise the per-partition step loop and the source-filter pushdown. |
| ≥ 2 | Rejected: `CumulativeMultipleDrivingSources`. A future plan may add explicit `driven_by:` disambiguation for same-granularity sources (Known Divergences). |

The driving source's `granularity` must be `day` or `week`. Any other granularity — `hour`, `month`, `quarter`, or `year` — is rejected at runtime by the per-partition step loop (see Known Divergences).

Non-timeseries sources in the FROM clause (lookups) are allowed and are read in full on every partition step.

### Classifier checks

`cumulative_aggregate` is rejected at planning time if any of these hold on the inlined outer SELECT (after function expansion):

1. **No `GROUP BY` clause** — `CumulativeRequiresGroupBy`.
2. **Non-key projection is not an allowlisted aggregator call** — `CumulativeUnknownAggregator`. Each projection that is not in the `GROUP BY` must be a direct call to one of the Surface §"Aggregator allowlist" functions, optionally with `AS <output_name>`. Composite expressions over aggregates (`SUM(x) + 1`, `MIN(x) / MAX(y)`) are rejected; authors must add columns for the underlying aggregates and compute derived values downstream.
3. **`GROUP BY` contains the driving source's `partition_column`** — `CumulativeGroupByContainsPartitionColumn`. Including the partition column in the key produces the per-partition shape, not the cumulative shape; the diagnostic suggests switching to `materialization: incremental` + `timeseries:`.
4. **Window functions in the outer body** — `CumulativeForbidsWindowFunctions`. Any `OVER (...)` clause on a projection in the outermost SELECT.
5. **Non-deterministic functions in the outer body** — `CumulativeForbidsNondeterministic`. `NOW()`, `CURRENT_TIMESTAMP`, `RANDOM()`, etc.

Additionally, the `Surface §"Diagnostic codes"` rejections for `CumulativeForbidsTimeseries`, `CumulativeForbidsIncremental`, `CumulativeNoDrivingSource`, and `CumulativeMultipleDrivingSources` fire at workspace load or planning time as named.

There is no `safety_overrides:` block for `cumulative_aggregate`. The rejected constructs cannot be bypassed because they break the cross-partition equivalence contract, not just the per-partition equivalence contract — there is no partial-correctness escape hatch the way `incremental:` has one for `allow_window_functions`.

### Cross-partition equivalence

For any set of source partitions `S = {D₁, …, Dₙ}` and any ordering π over `S`:

```
cumulative_aggregate_run(model, π(S))  ==  full_refresh(model, source.where(partition_col ∈ S))
```

The output state depends only on the *set* of processed source partitions, not on the order they were processed in. This is the load-bearing property the classifier upholds: every allowlisted aggregator has a commutative and associative combiner, and `GROUP BY` produces a stable key, so reordering merges does not change the final state.

This contract is **structurally different** from incremental's per-partition equivalence (`incremental_models.md` §"Per-partition equivalence"). Incremental promises that slicing the output by `partition_column = p` matches a full refresh's slice; cumulative has no `partition_column` to slice by, so it promises end-state equality after processing a set of source partitions.

### Reprocessing semantics

If a source partition `D` has already been merged in and the source data at `D` changes, re-running the cumulative model over `[D, D + granularity)` does **not** produce a correct cumulative state, because the prior delta from `D` is already baked into the target table and a second merge adds it again.

The rule rejects reprocessing at planning time when it can detect it (the partition has been merged before and the run window includes it). The error message points at the two mitigations:

1. **Full refresh.** Re-run with `--full-refresh` (truncate-and-rebuild from the source). This is the v1-correct path.
2. **Cascade rebuild** — manual, no built-in support: truncate the target and re-run the cumulative model over every source partition from `D` onward.

Subtract-then-add (keeping per-partition deltas in a side table) is a candidate future shape for reversible aggregators (`SUM`, `COUNT`, `BIT_XOR`); see Known Divergences.

### Source-filter pushdown

For the **driving source**, the rule injects a per-partition WHERE filter equivalent to:

```
WHERE <driving_source>.<partition_column> >= D
  AND <driving_source>.<partition_column> <  D + granularity
```

on the source reference in the inlined SELECT, where `D` ranges over the source partitions covered by the run window. The injection happens once per partition step, not once per run.

For **non-driving timeseries sources** (forbidden by the v1 multiple-driving-source rule), no pushdown happens — but the configuration is rejected before pushdown runs.

For **non-timeseries sources** (lookups), no pushdown happens — they are read in full on each partition step. This mirrors `incremental_models.md` §"Source-filter pushdown" — `timeseries:` on a source is the universal opt-in for being a pushdown target.

### Functions inside cumulative bodies

Function expansion (`expansion.md`) runs **before** the classifier and the driving-source walk. Classification, projection-list reading, GROUP-BY inspection, FROM-clause walking, and source-filter pushdown all operate on the expanded CST. A `smelt.define`-resolved aggregator call inside an outer-body projection is invisible to the classifier unless its expanded body produces an allowlisted aggregator at the outermost expression position; in that case it is admitted on the same terms as a hand-written allowlisted call.

Opaque calls (`smelt.extern` declarations, canonical built-ins that the analyser cannot inline) in the outer projection list are rejected — the classifier cannot prove they are allowlisted aggregators, so `CumulativeUnknownAggregator` fires.

### Interaction with `--auto` / staleness

`--auto`'s "what's stale" analysis for a cumulative model returns:

- **All reversible aggregators** (`SUM`, `COUNT`, `BIT_XOR`): "exactly the changed source partitions". A v1 implementation cannot honour this without delta-history bookkeeping; v1 returns "any partition ≥ the earliest stale partition" for safety. See Known Divergences.
- **Any irreversible aggregator** (`MIN`, `MAX`, `BIT_AND`, `BIT_OR`, `BOOL_AND`, `BOOL_OR`): "any partition ≥ the earliest stale partition; force a full refresh if earlier-than-current partitions are stale". Refusing reprocessing is the v1 policy (Semantics §"Reprocessing semantics").

### `unique_key` and column naming

The rule derives `unique_key` from the `GROUP BY` column list. The column names in the cumulative output are the projection list's `AS` aliases (or the source column names when no alias is given). Authors writing a cumulative model think about the cumulative output's column names directly: `COUNT(*) AS event_count` produces a column called `event_count` whose value reflects the cumulative count across all merged source partitions.

## Design

This section captures the load-bearing rationale.

**Cumulative is a separate rule from incremental, not a strategy knob.** dbt conflates the two under `materialized='incremental'` and dispatches by `incremental_strategy`. This is the single most common source of confusion in dbt because the `strategy:` knob silently changes the equivalence contract — same frontmatter, different invariants. smelt picks the opposite shape: name the contract in the materialization name. `materialization: incremental` is per-partition-equivalent with a partitioned output; `materialization: cumulative_aggregate` is cross-partition-equivalent with a per-key output. The two are different rules because they uphold different contracts on different output shapes. Deeper rationale: `docs/research/20260522-cumulative-as-its-own-rule.md` §"Why per-partition equivalence is the wrong frame for cumulative".

**Derive `unique_key` and aggregators from the SQL, not from frontmatter.** The `GROUP BY` already names the key. Each non-key projection already names its per-partition aggregator. The cross-partition combiner is a fixed lookup table off the per-partition aggregator (`COUNT → SUM`, `MIN → MIN`, etc.). There is no information the rule needs that isn't already in the SELECT. *A `cumulative_aggregate:` block with `unique_key:` and `aggregators:` keys* was rejected because it re-introduces the metadata-vs-SQL drift problem the predecessor incremental work explicitly removed (`docs/research/20260521-incremental-as-planner-rule.md`, "derive lookback from the SQL"). The same principle applies here: if a thing is in the SQL, do not also put it in YAML. The frontmatter collapses to one line: `materialization: cumulative_aggregate`.

**Cumulative output is not itself a timeseries.** The output has a unique key and aggregated columns, but no `partition_column` and no `granularity` — it has collapsed all source partitions into a single per-key row. The model therefore does not declare `timeseries:`; the rule reads the partition shape from the driving source's `timeseries:` declaration. Downstream consumers see the cumulative table as a lookup. *Allowing a `timeseries:` block on a cumulative model and letting it produce a per-partition output* was rejected because that shape is already what `incremental:` produces — there is no new behaviour, only ambiguity. The forbid-`timeseries:` rule (`CumulativeForbidsTimeseries`) makes the boundary structural.

**Fixed aggregator allowlist, not a registry.** v1 ships exactly the SQL aggregators that are provably commutative and associative under standard semantics (`COUNT`, `SUM`, `MIN`, `MAX`, `BIT_*`, `BOOL_*`). *Letting authors register custom combiners* was rejected for v1 because (a) the v1 web_analytics motivating example needs only the standard allowlist, (b) custom combiners would need a workspace-level registry surface that is out of scope here, and (c) extending the allowlist is additive and can be done without a spec change. `AVG`, `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST`, `APPROX_COUNT_DISTINCT` are intentionally out — `AVG` is rewritable to `SUM/COUNT` at read time but the rule does not perform that rewrite in v1.

**One driving source for v1, not multi-source.** Cumulative SQL with multiple timeseries-tagged sources is rejected (`CumulativeMultipleDrivingSources`). *Accepting multiple sources and looping over the finest granularity* was deferred because (a) it adds significant rule complexity (cross-source filter coordination) and (b) the motivating examples do not need it. An explicit `driven_by:` disambiguation field on the model is the natural follow-on, but adding it changes the "frontmatter is one line" property — it should land only when a concrete second-source motivator surfaces.

**Refuse reprocessing in v1, not subtract-then-add.** The simplest correctness contract is "merging a partition twice is undefined". Reprocessing requires either delta history (a side table per cumulative model) or a cascade rebuild from the changed partition forward. Both are more code than v1 should ship. *Silently double-counting reprocessed partitions* is the dbt-merge-strategy footgun and was rejected outright. The rule refuses to merge a partition that was already merged, with a diagnostic that points the author at `--full-refresh`.

**No `safety_overrides:` block.** Incremental's classifier offers per-check overrides (`allow_window_functions`, `allow_having`, etc.) because some rejected constructs only break *full-refresh equivalence*, and authors can knowingly accept partial-correctness. Cumulative's rejected constructs break the cross-partition equivalence contract itself — there is no partial-correctness fallback. A `safety_overrides:` knob would be a footgun: bypassing `CumulativeUnknownAggregator` for `STRING_AGG` would produce silently order-dependent output that is impossible to debug. The classifier is strict by design.

**`materialization: cumulative_aggregate` lives alongside `incremental`, not under it.** The `Materialization` enum (`models.md` §"Materialization modes") gains one variant. `IncrementalStrategy::Merge` is dropped — the variant was a placeholder for the cumulative-as-strategy shape this spec rejects. The DuckDB `merge_into` backend primitive stays; it becomes the cumulative rule's physical strategy. The trait method, the implementation, and its tests do not move.

## Constraints & Invariants

1. **Frontmatter is the materialization name alone.** A `cumulative_aggregate` model declares `materialization: cumulative_aggregate` and nothing else specific to this rule. There is no `cumulative_aggregate:` configuration block.
2. **`timeseries:` and `incremental:` are forbidden on cumulative models.** Diagnostics: `CumulativeForbidsTimeseries`, `CumulativeForbidsIncremental`.
3. **`unique_key` is derived from `GROUP BY`.** A cumulative model without `GROUP BY` is rejected (`CumulativeRequiresGroupBy`).
4. **Per-column cross-partition combiner is a fixed lookup off the per-partition aggregator.** Authors do not declare combiners; the rule looks them up from the allowlist table.
5. **Allowlist is closed.** Aggregators outside the table are rejected. Composite expressions over aggregates are rejected. No `safety_overrides:` bypass.
6. **Exactly one driving source.** The classifier requires exactly one timeseries-tagged source in the inlined FROM clause.
7. **Cross-partition equivalence holds for any ordering.** Reordering merges across source partitions does not change the final cumulative state.
8. **No `partition_column` on the cumulative output.** Downstream consumers treat the cumulative table as a lookup.
9. **Reprocessing is refused in v1.** A run window that overlaps already-merged source partitions is rejected with a diagnostic; `--full-refresh` is the v1 mitigation.
10. **No silent downgrade.** A classifier rejection refuses the model at planning time. No fallback to full-refresh, no fallback to incremental, no warning-then-continue.

## Known Divergences / Open Questions

- **`AVG` rewrite.** Out of scope for v1. The classifier refuses `AVG(...)`. A future plan may rewrite it at planning time to `SUM/COUNT` and surface the average as a derived column.
- **Multi-source disambiguation (`driven_by:`).** A cumulative model reading multiple timeseries-tagged sources is rejected in v1 (`CumulativeMultipleDrivingSources`). A future plan may add an explicit `driven_by: smelt.<source>` field on the frontmatter to pick among same-granularity candidates. Different-granularity sources are deferred indefinitely.
- **Self-referential cumulative.** A SELECT that joins to its own cumulative target (e.g., `cumulative_state += sum(new_partition) - decay`) reads "prior cumulative value" and is recursive. Rejected in v1 by the general "exactly one driving source" rule when the target itself is in the FROM clause. A future plan may admit this pattern with explicit input/state distinction.
- **Reprocessing via delta history.** A future plan may store per-partition deltas for cumulative models with reversible aggregators (`SUM`, `COUNT`, `BIT_XOR`), enabling subtract-then-add for reprocessing. The model's frontmatter does not declare reversibility; the classifier infers it from the projection list.
- **`--auto` staleness fidelity.** v1's staleness response for a cumulative model is conservative ("any partition ≥ the earliest stale"); the per-projection fidelity ("exactly the stale partitions for fully-reversible models") needs the delta-history mechanism above.
- **Schema evolution.** Adding a new non-key column to a cumulative table requires backfilling the new aggregator over the entire processed source history. v1 does not support this; the model must be rebuilt. Tracked under `schema_evolution.md` future work.
- **Sibling rules (`scd2`, `latest_value`, `accumulating_snapshot`).** These follow the same derive-from-SQL principle but uphold different contracts. None are speced today; `cumulative_aggregate` is the first member of this family. See `docs/research/20260522-cumulative-as-its-own-rule.md` §"Sibling rules beyond cumulative_aggregate".
- **External sources without `timeseries:`.** A cumulative model whose only source is a non-timeseries external table has no partition shape to step over and is refused (`CumulativeNoDrivingSource`). The diagnostic suggests declaring `timeseries:` on the source.
- **Granularity restricted to `day` and `week`.** The v1 per-partition step loop accepts only `day` and `week` as the driving source's granularity. Any other granularity — `hour`, `month`, `quarter`, or `year` — is rejected at runtime with the error `cumulative_aggregate v1 supports day and week granularity; got <Granularity>`. This is a not-yet-supported limitation of the step loop arithmetic, not a permanent design boundary. Tracked in `docs/plans/20260611-docs-gap-remediation.md`.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — `Materialization` enum (gains `CumulativeAggregate` variant); `IncrementalStrategy::Merge` (variant to drop)
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction, validation that `timeseries:` / `incremental:` are absent when `materialization: cumulative_aggregate`
  - `crates/smelt-logical/src/rules/cumulative.rs` — the cumulative classifier (pure rule-data, in `smelt-logical`; `smelt-planner` re-exports — see architecture.md §"Constraints & Invariants" (Layered single-ownership))
  - `crates/smelt-planner/src/rules/` — host for the per-partition step loop (rule *application*)
  - `crates/smelt-backend/src/lib.rs` — `merge_into` trait method (physical primitive the rule calls)
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `merge_into` implementation
- **Tests**:
  - `crates/smelt-backend-duckdb/src/lib.rs::test_merge_into_upsert`, `test_merge_into_insert_only` — backend primitive coverage
  - Cumulative classifier unit tests and per-partition equivalence tests (to be added alongside the implementation plan)
- **User docs**: `docs-site/docs/guide/materializations.md` (to be updated to add the `cumulative_aggregate` mode alongside the existing five)
- **Plans (history)**:
  - `docs/plans/20260523-cumulative-aggregate.md` — implementation plan derived from this spec
- **Research**:
  - `docs/research/20260522-cumulative-as-its-own-rule.md` — the rationale for the rule shape; predecessor of this spec
  - `docs/research/20260521-incremental-as-planner-rule.md` — sibling research; the "derive from SQL, not YAML" principle this spec inherits
  - `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md` — Gap #5, the original motivation
- **Related specs**:
  - `incremental_models.md` — the sibling rule with per-partition equivalence and timeseries output
  - `timeseries.md` — the source-side declaration this rule consumes
  - `models.md` — `Materialization` enum host; frontmatter table
  - `expansion.md` — function expansion pass; runs before the classifier
  - `architecture.md` — `smelt.<path>` addressing; backend primitive contract
