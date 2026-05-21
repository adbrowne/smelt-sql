---
feature: incremental_models
status: experimental
last_reviewed: 2026-05-21
owners: [andrew]
---

# Incremental Models

> **Scope.** Incremental materialization for time-partitioned models: the `incremental:` frontmatter block, the partition-based DELETE+INSERT execution strategy on DuckDB, safety checks the optimizer enforces, per-source lookback derivation from the model's SQL, source-filter pushdown, and the rules around what may be expressed in a logical incremental model. The time-dimension declaration (`event_time_column`, `partition_column`, `granularity`) lives in `timeseries.md` — this spec consumes it.
>
> **Status: experimental.** The DuckDB DELETE+INSERT path is implemented and tested. MERGE on Spark/Databricks, schema-evolution, state tracking with gap detection, and per-column `data_latency` are planned (see `docs/plans/20260322-incremental-model-support.md`) and recorded under Known Divergences below.

## Surface

### YAML frontmatter (in `.sql` files)

```sql
---
materialization: table
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
incremental:
  enabled: true
  unique_key: [order_id]      # optional; backends with MERGE can use it
  safety_overrides:           # optional; bypass specific safety checks
    allow_window_functions: false
    allow_having: false
    allow_subqueries: false
---

SELECT order_date, customer_id, SUM(amount) AS total
FROM smelt.orders
GROUP BY order_date, customer_id
```

`materialization` is optional. When omitted from frontmatter, the model falls back to the per-model entry in `smelt.yml` (`models.<name>.materialization`), and finally to the project-level `default_materialization` key (whose own default is `view`). Incremental models must resolve to `materialization: table` — declaring `incremental:` on a non-table materialization is a configuration error or a warning depending on the resolved kind (see `smelt_yml.md`).

A model declaring `incremental:` must also declare `timeseries:` (`timeseries.md`). Missing the `timeseries:` block produces a `TimeseriesRequiredForIncremental` diagnostic at workspace load.

### `smelt.yml` (project-level overrides)

Frontmatter wins over `smelt.yml` when both set the same field.

```yaml
models:
  daily_revenue:
    materialization: table
    timeseries:
      event_time_column: order_date
      partition_column: order_date
      granularity: day
    incremental:
      enabled: true
```

### CLI

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

- Both flags are required for any incremental execution. Format: ISO-8601 (`2026-03-20`, `2026-03-20T00:00:00Z`).
- The end bound is exclusive: `--event-time-end 2026-03-25` does not include `2026-03-25`.
- The supplied `[start, end)` range is the **run window**. It must be a positive integer multiple of `timeseries.granularity` aligned to granularity boundaries (`timeseries.md` § "Granularity arithmetic"). Run-window size may exceed partition granularity — a `--event-time-start 2026-03-20 --event-time-end 2026-04-19` run on a daily-partitioned model covers 30 partitions in one engine query and 30 partition writes (Semantics § "Run window vs partition granularity").
- `backbuild` uses the model's classified batch safety (see Semantics) to expand or split the requested range.

### Granularity values

See `timeseries.md` § "Granularity values" for the closed enum. `incremental:` consumes the granularity declared in the model's `timeseries:` block.

### Strategy enum (backend-internal)

Strategy is **not** declared on the model. Backends pick a strategy given the model's config and their capabilities:

```rust
enum IncrementalStrategy {
    DeleteInsert,    // DELETE matching partitions + INSERT
    Merge,           // UPSERT keyed by unique_key (requires backend MERGE support)
    Append,          // insert-only; no dedup
    InsertOverwrite, // replace entire partitions atomically
}
```

DuckDB currently always uses `DeleteInsert`.

## Semantics

### Execution model (DuckDB, current)

For an incremental run with `[start, end)` (the **run window**):

1. **DELETE** from the output table where `partition_column >= start AND partition_column < end`, using the `partition_column` declared in the model's `timeseries:` block.
2. **Inject** an AST-level `WHERE partition_column >= start AND partition_column < end` filter on the model's logical SQL at the outermost SELECT. The injection is per-model (whole-query); it constrains the model's *output* to the run window.
3. **Push down** per-source filters onto each `smelt.<path>` reference inside the body, derived from the model's SQL (Semantics § "Source-filter pushdown"). Sources without a `timeseries:` declaration are not pushdown candidates — they are read in full.
4. **INSERT** the resulting query's output into the output table.

This is idempotent under fixed input: re-running the same `[start, end)` produces the same final state. Per-partition equivalence holds: for any partition `p ∈ [start, end)`, the rows of the output where `partition_column = p` equal the rows a full-refresh run would produce filtered to the same partition (Semantics § "Per-partition equivalence").

### Run window vs partition granularity

The CLI's `[--event-time-start, --event-time-end)` declares a **run window**, not a per-partition invocation. The run window must be a positive integer multiple of `timeseries.granularity` aligned to granularity boundaries (`timeseries.md` § "Granularity arithmetic"); inside that constraint, the run-window size and the partition-granularity unit are independent.

For a daily-partitioned model run with a 30-day window:

- The engine query is **one** query covering the whole 30-day range. Source FROMs are filtered to the union of the run window and any per-source pushdown bounds; the outermost WHERE constrains the output to the 30-day run window.
- The backend write is **one** DELETE over the 30 partitions followed by INSERT of the engine result. Per-partition idempotence is preserved by partition-aligned DELETE.

Backfilling 60 days of a daily-partitioned model is one `smelt run --event-time-start D --event-time-end D+60d` invocation, not 60 successive daily invocations. The per-partition equivalence property (below) holds regardless of run-window size.

### First-run and backfill

A first run (no existing output table) and a backfill (a re-run of a range that has already been written) follow the same DELETE+INSERT contract — the DELETE is a no-op when the partition is absent. The planner picks a chunking shape from the model's batch-safety class (§"Batch safety classification"):

| Class                | Chunking                                                                                   |
|----------------------|--------------------------------------------------------------------------------------------|
| `FullyBatchSafe`     | A single DELETE+INSERT pair covers any `[start, end)`. No chunking.                        |
| `BoundedSafe(n)`     | Auto-sized sub-ranges (the existing 3× context, clamped 7–90 partitions rule). Each sub-range is one DELETE+INSERT pair, executed sequentially in temporal order. |
| `PerPartitionOnly`   | One partition per iteration, sequential, temporal order. Each partition is one DELETE+INSERT pair. |

**Per-chunk transaction boundary.** Each chunk's DELETE+INSERT is one backend transaction. INSERT failure rolls back the chunk's DELETE. Earlier committed chunks **do not** roll back — partial-progress is intentional, since each chunk is idempotent under the same `[start, end)`.

**Failure mode.** A run halts at the first failed chunk and exits non-zero. Re-running the same `[start, end)` resumes correctly because every committed chunk is idempotent: the next attempt will re-DELETE+INSERT the failed (or any later) range from the same input data and converge to the same final state.

**Late-arriving data (interim guidance).** smelt does **not** automatically re-run partitions when data arrives late. Two interim mitigations:

1. Trail `--event-time-end` behind real-time by the source's known latency window — i.e., always run with an end bound far enough in the past that late-arriving rows are already present.
2. Run with overlapping ranges — e.g., a daily run that always re-processes the last 7 days' partitions — accepting the redundant work for the correctness guarantee.

A planned automated mechanism is the per-column `data_latency:` annotation (Known Divergences below); until that lands, the two mitigations above are the only options.

### Per-source bound derivation

For each `smelt.<path>` reference inside the model body (after function expansion — see § "Functions inside incremental bodies"), the optimizer derives a **bound tuple** describing the inverse image of the run window in the source's partition-column space:

```
BoundResult =
  | Bounded { source_partition_col, before: Duration, after: Duration }
  | Unbounded                  -- analyzable but ∞ (cumulative-across-history)
  | NotDerivable               -- analyzer can't read this pattern
```

`source_partition_col` is the source's declared `timeseries.partition_column` (or, when narrower, a column the source declares as partition-aligned). `before` and `after` are durations expressed in that column's unit. A `Bounded(c, 0, 0)` source has no lookback or lookforward — it is read partition-by-partition. A `Bounded(c, 1d, 0)` source has a 1-day lookback. A `Bounded(c, 24h, 24h)` source spans 24 hours either side — the pattern that arises when a model rebases UTC timestamps into local-date partitions.

**Two forms the optimizer reads, both standard SQL:**

- **Form A — window-frame `RANGE BETWEEN INTERVAL '…' PRECEDING/FOLLOWING`.** The literal `INTERVAL` is the lookback (or lookforward) for the source backing the projected column.

  ```sql
  LAG(event_ts) OVER (
      PARTITION BY device_id ORDER BY event_ts
      RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW
  ) AS prev_ts
  ```

- **Form B — explicit WHERE/JOIN time filters with literal `INTERVAL` offsets.** The offset is the lookback (or lookforward) on the named source column. Supports both same-column lookback and cross-column rebasing (the source's time column differs from the model's partition column):

  ```sql
  FROM bronze.events b
  JOIN users u ON b.user_id = u.user_id
  WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '1 day'
                           AND m.event_date_local + INTERVAL '1 day'
  ```

`BETWEEN` and paired `>=` / `<` forms are equivalent and both read.

**Aggregation across multiple references to the same source** — take the union: `before = max(before_i)`, `after = max(after_i)`. Any `Unbounded` reference forces the union to `Unbounded`; any `NotDerivable` reference forces the union to `NotDerivable`.

**Aggregation across distinct sources** — each source independent. A model referencing two timeseries sources and a lookup table produces two bound entries (one per timeseries source) and one no-entry (the lookup, read in full).

**Sources without `timeseries:` are lookups.** No bound is derived; pushdown skips them; they are read in full each run.

### Batch safety classification

The optimizer rolls the per-source bound map into a single class per incremental model:

| Class               | Meaning                                                                 | Execution                                                |
|---------------------|-------------------------------------------------------------------------|----------------------------------------------------------|
| `FullyBatchSafe`    | All timeseries sources `Bounded(_, 0, 0)`; no temporal dependencies     | Single query for any run window                          |
| `BoundedSafe(n)`    | All timeseries sources `Bounded`, with `n = max(before + after)` > 0    | Auto-sized chunks (3× context, clamped 7–90 partitions)  |
| `PerPartitionOnly`  | One or more timeseries sources `Unbounded` (cumulative-across-history)  | One partition at a time, sequential                      |

`n` for `BoundedSafe` is rendered in the source's partition-column unit and is the same value pushed to source-filter ranges (Semantics § "Source-filter pushdown").

A model with **any** `NotDerivable` source is **refused at planning time**, not assigned to a class — the optimizer cannot prove the partition-DELETE+INSERT contract is safe. The diagnostic names the offending construct (the bare `LAG` without a RANGE clause, the un-bounded computed-expression join, the projection through a non-catalog operation) and the source-map points at the original SQL location. The author rewrites using Form A or Form B (above) or removes the dependency. There is no silent downgrade to full-refresh.

### Source-filter pushdown

For each `Bounded(c, before, after)` source reference in the model body, the optimizer injects:

```
WHERE c >= run_start - before
  AND c <  run_end + after
```

on that source's FROM clause. `run_start` and `run_end` come from the run window (Semantics § "Run window vs partition granularity"); the WHERE is added to the same source reference's compiled SQL, not duplicated at the outer query.

The outer WHERE injection (Execution model step 2) and source-filter pushdown are independent:

- The outer WHERE constrains the **model's output** to the run window, using the model's own `timeseries.partition_column`.
- Source-filter pushdown constrains each **source read** to the union of the run window and the source's `(before, after)` bound, using the source's own `timeseries.partition_column`.

Sources without `timeseries:` are not pushdown candidates — they are read in full each run.

**Pushdown is per-reference.** A model that joins the same source twice (a self-join, or two references reaching the same source through inlined function bodies) emits a pushdown filter on each reference; the filter on each is the union bound from § "Per-source bound derivation".

### Per-partition equivalence

For every partition `p` in the model's run window `[run_start, run_end)`:

```
incremental_run(model, [run_start, run_end))
  .where(partition_column = p)
== full_refresh(model).where(partition_column = p)
```

This is the formal contract the incremental rule upholds. It is independent of run-window size — a 60-day run window and 60 successive single-day runs produce equivalent per-partition output.

The equality holds for **local** columns of the output — columns whose value depends only on source rows visible within the model's source-filter ranges. Columns whose value depends on history outside the source-filter ranges (cumulative aggregations such as connected-components or backward-fill) are **not equivalent** — the per-partition value reflects state at the time the partition was run, not the final cumulative state after all partitions are processed. Such columns force the source to `Unbounded` and the model to `PerPartitionOnly`; running them remains correct as-of-the-run, just not equivalent to a full refresh that re-runs every partition with the final cumulative input.

### Safety checks (rejected by default)

The optimizer rejects an incremental model if its SQL uses constructs that break the partition-DELETE-then-INSERT contract or produce non-deterministic output:

- Window functions (`OVER (...)`), **unless** the window is partition-aligned (see below).
- `HAVING`
- `LIMIT`
- Subqueries (`SELECT ... FROM (SELECT ...)`)
- Non-deterministic functions (`RANDOM()`, `NOW()` outside of stable contexts, etc.)
- `DISTINCT`

Each check can be individually disabled via `incremental.safety_overrides.allow_<check>: true`. Disabling is opt-in and recorded.

#### Partition-aligned window functions

A window function `OVER (PARTITION BY <keys>)` is admissible without a safety override when `<keys>` is a **superset** of the model's `timeseries.partition_column`. For a model with `partition_column: event_date`, both `OVER (PARTITION BY event_date)` and `OVER (PARTITION BY event_date, session_seq)` are admitted; `OVER (PARTITION BY user_id)` (which does not contain `event_date`) is rejected.

The superset requirement ensures every window is evaluated within a single partition: the DELETE+INSERT contract deletes and re-inserts whole partitions, so a window whose scope crosses partition boundaries can produce different results on partial data. A partition-aligned `PARTITION BY` prevents cross-partition scope.

An `OVER (...)` with **no** `PARTITION BY` clause is always rejected by this check.

### Functions inside incremental bodies

An incremental model body may call transparent functions (`smelt.define`-resolved) and opaque calls (`smelt.extern` declarations, canonical built-ins, source references). Function expansion (`expansion.md`) is a logical pass that runs **before** every analysis stage in this spec — per-source bound derivation, batch-safety classification, and source-filter pushdown all see the expanded CST. From their point of view, a `LAG()` inside a `smelt.define` body and one inlined at the call site are indistinguishable: each is read with the same Form A / Form B vocabulary.

Two interactions remain:

1. **Per-model WHERE injection happens at the outer expanded query.** The framework's injected `WHERE partition_column >= start AND partition_column < end` clause is applied once at the outermost SELECT (Execution model step 2). Because it is applied *after* function expansion, the filter sees any columns produced inside an expanded function body without a separate pushdown rule for transparent calls.

2. **Source-filter pushdown reaches inside function bodies via expansion.** The pushdown WHEREs (Semantics § "Source-filter pushdown") are added to `smelt.<path>` references in the expanded CST. References that originated inside a `smelt.define` body receive the same pushdown as references in the outer body.

**Opaque calls (`smelt.extern`, canonical built-ins) remain black boxes.** Bound derivation cannot read through them. An incremental model whose source-of-time-dependence is hidden behind an `smelt.extern` call is `NotDerivable` and refused at planning time, unless the analyser can prove a bound from the surrounding SQL (a WHERE clause around the opaque call, an explicit RANGE-windowed projection of its result). Authors of opaque calls with hidden temporal dependencies must either expose the dependency in surrounding SQL or accept that the model cannot run incrementally.

Cross-link: `planner_integration.md` §"Optimization boundary: transparent vs black-box".

### `partition_column` validation

The optimizer requires `partition_column` to appear in both the `SELECT` list and the `GROUP BY` (when grouping is present). A model whose SELECT does not project `partition_column` is rejected before execution.

### State ownership

smelt does not track watermarks, offsets, or run history for incremental models. The backend owns computational state:

- DuckDB: table state and transaction history.
- Future Delta/Spark: transaction log; MERGE strategy.
- Future Flink: checkpoints.

Optional run-state tracking with gap detection is planned (Phase 5 of the incremental plan) and is opt-in.

## Design

This section captures the load-bearing rationale behind the incremental model surface and the alternatives that were considered and rejected.

**Logical SQL is pure; the framework injects the time filter.** A model body never contains `is_incremental()`, `{% if execution_mode == ... %}`, or any other conditional that branches on whether the run is full or incremental. The same SQL is the full-build and the incremental-build description, and the framework injects a `WHERE partition_column >= start AND partition_column < end` clause at the per-model boundary. *Jinja-style `is_incremental()` branching* (the dbt shape) was rejected because it splits one model into two implicit ones — the full-build version and the incremental version — and they drift: a developer fixes a join in the incremental branch and forgets the full-build branch, or a backfill produces different aggregates than a fresh build. Pure logical SQL means there is one source of truth; full and incremental are both derivations of it. The trade-off is that incremental models must accept the framework's filter shape (per-model, on the outer query); the planner's batch-safety analysis polices the fit.

**DELETE+INSERT over partition columns, not MERGE, for v1.** DuckDB's incremental strategy is `DeleteInsert` — drop the matching partitions, run the filtered query, insert the result. *MERGE* was rejected as the v1 default for two reasons: it requires a `unique_key` (which not every model has), and the MERGE pathway has cross-engine subtleties (Spark MERGE on Delta vs. Parquet vs. iceberg, DuckDB's late-2024 MERGE quirks) that we do not want to litigate before the simpler DELETE+INSERT shape is widely deployed. DELETE+INSERT is idempotent under fixed input, easy to reason about ("re-run the same range, get the same result"), and aligns with the partition-column safety analysis the planner already does. MERGE remains in the `IncrementalStrategy` enum (Surface §"Strategy enum") for backends that want to opt in; it is not the default for any backend today.

**smelt does not own state.** Watermarks, run history, gap detection, and offset bookkeeping live in the backend (DuckDB transactions; Delta logs; Flink checkpoints) — the framework only generates SQL. *Owning a watermark store* (a `.smelt/state/<model>.json` of last-run boundaries) was rejected as a v1 requirement because it duplicates state the backend already tracks, opens a sync-correctness window between smelt's view and the engine's view, and locks adoption: a workspace with a half-broken watermark store is harder to migrate than a workspace whose state lives entirely in the database. Optional run-state tracking with gap detection is planned (Phase 5 of the incremental plan) and is opt-in; the core stays state-free.

**Three-class batch-safety taxonomy: `FullyBatchSafe` / `BoundedSafe(n)` / `PerPartitionOnly`.** The planner classifies every incremental model into exactly one of these classes (Semantics §"Batch safety classification"). *A binary "safe / unsafe" flag* was rejected because too many real workloads are bounded-safe — a `LAG()` over a 7-day window, a self-join with a 24-hour interval — and a binary classifier either rejects them all or accepts them all without the auto-chunking that bounded-safe models need. *A continuous safety score* (a numeric "lookback days") was rejected because the user-facing decision is qualitative ("can I run this in chunks? how big?") and a numeric score is harder to surface in diagnostics. The three classes map directly to backend-execution shapes: `FullyBatchSafe` runs in one query for any range, `BoundedSafe(n)` runs in auto-sized chunks, `PerPartitionOnly` runs one partition at a time.

**Derive lookback from the model's SQL, not from frontmatter.** The per-source bound map (Semantics §"Per-source bound derivation") is computed from window-frame `RANGE BETWEEN INTERVAL` clauses (Form A) and explicit WHERE/JOIN time filters (Form B) in the model's SQL — including SQL inlined from `smelt.define` bodies via expansion. *A per-model `lookback_days:` YAML annotation* was rejected because it puts the time-dependency declaration in metadata, separated from the SQL or function logic that creates the need; declaration and logic can drift, and an author can change a function's time window without remembering to update the YAML. *A function-level `lookback = …` declaration* on `smelt.define` was rejected because function expansion already lets the analyser read the Form A / Form B patterns inside function bodies — the declaration would duplicate information already statically present in the SQL. The trade-off is that models with implicit time logic (a bare `LAG` with no RANGE clause, a computed-expression join with no date filter) refuse incrementality and the author must rewrite using a derivable form. This is arguably the right outcome: a model the planner can't analyse for lookback is one it can't reason about for correctness. Deeper rationale: `docs/research/20260521-incremental-as-planner-rule.md`.

**Per-source bounds, not a single per-model lookback.** A model can read multiple sources with independent time dependencies — one with a 1-day lookback, one with no lookback, one a non-timeseries lookup. *A single per-model `lookback_days` value* was rejected because it forces over-reading the sources that don't need lookback to satisfy the source that does, and silently mis-handles cross-column rebasing (UTC source → user-local partition column). The per-source `(before, after)` shape generalises cleanly: same-column lookback, cross-column rebasing, range-joins, and unbounded cumulatives all collapse into the same machinery. Different sources on the same model get independent pushdown.

## Constraints & Invariants

1. **Logical model is pure SQL.** No `is_incremental()`, no `{{ ... }}` macros, no conditional branches. The same SQL describes both the full-build and the incremental-build behavior; the framework injects the time filter.
2. **`timeseries:` is required for `incremental:`.** A model declaring `incremental:` without a `timeseries:` block (per `timeseries.md`) produces `TimeseriesRequiredForIncremental` at workspace load. The time-dimension lives in `timeseries:`; the incremental rule consumes it.
3. **Strategy is not on the model.** Frontmatter declares `unique_key`; the time-dimension fields live in `timeseries:`; the backend chooses `DeleteInsert` / `Merge` / etc. Model files do not name a strategy.
4. **smelt does not manage computational state.** Watermarks, offsets, and run-history live in the backend. The framework only generates SQL artifacts.
5. **Output-filter injection is per-model; source-filter pushdown is per-reference.** The outer `WHERE` constraining the model's output to the run window is applied once at the outermost SELECT. Source-filter pushdown WHEREs are applied per `smelt.<path>` reference in the expanded body, derived from per-source bounds (Semantics § "Source-filter pushdown").
6. **Per-partition equivalence with full refresh.** For every partition `p` in the run window, the incremental rule's output `where(partition_column = p)` equals the full-refresh output `where(partition_column = p)`, for all columns whose value depends only on rows visible within the model's source-filter ranges. Globally-dependent columns (cumulative aggregations) are not equivalent — see Semantics § "Per-partition equivalence".
7. **Idempotence under fixed input.** For a given backend and unchanged source data, running the same run window repeatedly converges to the same output table state.
8. **Granularity is closed under partition arithmetic.** A run window must align to whole granularity units; partial-unit ranges are rejected.
9. **Safety check overrides are explicit.** A safety override must name the specific check it bypasses. There is no global "disable all safety checks" switch.
10. **No silent downgrade to full-refresh.** A model that the safety classifier rejects or whose bound derivation produces `NotDerivable` is refused at planning time with an explanatory diagnostic; it does not silently fall back to full-table execution.

## Known Divergences / Open Questions

- **`-- @materialize: incremental` annotation never implemented.** `docs/DESIGN.md` § "Configuration Syntax" still shows the annotation form as an alternative; YAML frontmatter is the only implemented surface. The DESIGN.md prose marks this as a future option but the surface section in this spec is authoritative.
- **Migration to `timeseries:` block pending.** The current implementation reads `event_time_column`, `partition_column`, and `granularity` from inside the `incremental:` block. Migration to the `timeseries:` block specified here (and in `timeseries.md`) is the next plan derived from `docs/research/20260521-incremental-as-planner-rule.md`. The cutover is one-shot — no transitional dual-form support.
- **Source-filter pushdown not yet wired.** Per-source bound derivation (Semantics § "Per-source bound derivation") is implemented and exposed via `smelt explain --json`. Per-source pushdown WHEREs (Semantics § "Source-filter pushdown") are not yet injected — the planner derives the bounds but does not yet rewrite source FROMs. Tracked in `docs/plans/20260521-incremental-timeseries-and-derived-bounds.md` (Phase 5).
- **Bound derivation runs on the outer SQL body, not the expanded CST.** The current analysis reads Form A / Form B patterns from the outer model SQL only; patterns inside `smelt.define` function bodies are invisible unless the caller concatenates the expanded body into the SQL string it passes to the analyser. A model whose sole RANGE clause is inside a function body will derive `Bounded(_, 0, 0)` rather than reflecting the inner bound (it will not produce `NotDerivable`, so it will not be refused — but the derived bound may be narrower than reality). The spec requires derivation on the expanded CST (per `expansion.md`); wiring this requires feeding the post-expansion `LogicalNode` tree into the bound analyser. Tracked in `docs/plans/20260521-incremental-timeseries-and-derived-bounds.md`.
- **Per-column `data_latency` not implemented.** Plan calls for declaring `data_latency` on upstream sources for late-arriving data; not yet available.
- **MERGE strategy is DuckDB-only-future.** `BackendCapabilities.supports_merge` is `true` for DuckDB / Spark / PostgreSQL, but the planner emits `DeleteInsert` for all three today. A Spark MERGE pathway is in the plan but unbuilt.
- **Three execution paths in `crates/smelt-cli/src/main.rs`.** Legacy, optimizer+incremental, incremental-only — Phase 1 of the incremental plan unified `IncrementalConfig` but the CLI dispatch is still tri-modal. Should converge.
- **Granularity conversion boilerplate.** A duplicate `Granularity` enum existed in `smelt-planner/src/types.rs` and was reconciled with `smelt-core`; check for residual conversion code in `main.rs` (lines around 669–683 in the plan reference) when next touching this area.
- **No interval / run-state tracking.** Skipped runs currently produce silent gaps (same failure mode as dbt). Tracking is planned but opt-in; see `run_state.md` anchor in `architecture.md` §"Specs not yet authored".
- **Schema evolution is unspecified.** A `partition_column` rename or an output schema change has no defined handling today.
- **`smelt.metric()` interaction.** The interaction between metric expansion and time-filter injection is not fully spelled out for incremental models that consume metrics.
- **Diagnostic codes pre-`diagnostics.md`.** Codes listed in this spec are owned here until a `diagnostics.md` spec lands. `diagnostics.md` will define ownership rules, severity tiers, stability tiers, and suppression. Code names may be renamed under that spec. (See `architecture.md` §"Specs not yet authored".)
- **Generator-emitted incremental models are landed.** A `ModelDef` value emitted by a generator file (per `meta_language.md` §"Multi-model production") may carry `materialization: 'incremental'`, and the emitted model is subject to every rule in this spec on equal terms with a hand-authored incremental model — batch-safety classification, per-source filter injection, DELETE+INSERT execution, the first-run-and-backfill path. The `incremental:` block is inherited from the generator file's file-wide frontmatter (`EmittedModelDef.incremental_config`); per-`ModelDef` overrides of `incremental.partition_column`, `incremental.granularity`, etc. are not part of the closed `ModelDef` field set in v1 (a future spec edit may add them). A single generator emits a single `incremental:` configuration shared by every emitted incremental model; users who need divergent per-emission incremental settings split the generator into multiple files. Tracked in `docs/plans/20260509-meta-language-overall.md`.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — `IncrementalConfig`, `Granularity`, `Weekday`
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction, `ModelMetadata`
  - `crates/smelt-core/src/sources.rs` — `SourceColumnDef` (future home of `data_latency`)
  - `crates/smelt-planner/src/rules/incremental.rs` — detection + safety checks
  - `crates/smelt-planner/src/types.rs` — safety-override types
  - `crates/smelt-cli/src/transformer.rs` — `inject_time_filter()`
  - `crates/smelt-cli/src/executor.rs` — `execute_model_incremental()`, `execute_plan_incremental()`
  - `crates/smelt-cli/src/main.rs` — CLI dispatch (incremental paths around the `run` / `backbuild` subcommands)
  - `crates/smelt-backend/src/lib.rs` — `Backend::delete_partitions()`, `Backend::insert_into_from_query()`
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `DeleteInsert` impl
  - `crates/smelt-dialect/src/dialect.rs` — `BackendCapabilities::supports_merge`
- **Tests**: 17 optimizer unit tests in `crates/smelt-planner/src/rules/incremental.rs`; CLI integration tests in `crates/smelt-cli/tests/incremental_*.rs`; 7 optimizer integration tests; 13 metadata tests
- **User docs**: [`docs-site/docs/guide/incremental-models.md`](../../docs-site/docs/guide/incremental-models.md), [`docs-site/docs/guide/materializations.md`](../../docs-site/docs/guide/materializations.md)
- **Plans (history)**:
  - [`docs/plans/20260322-incremental-model-support.md`](../plans/20260322-incremental-model-support.md) — comprehensive plan; many phases still open
  - [`docs/plans/20260325-materialization-types.md`](../plans/20260325-materialization-types.md)
- **Research**:
  - [`docs/research/2026-05-20-incremental-gaps-from-web-analytics.md`](../research/2026-05-20-incremental-gaps-from-web-analytics.md) — gaps catalogued during web_analytics conversion
  - [`docs/research/20260521-incremental-as-planner-rule.md`](../research/20260521-incremental-as-planner-rule.md) — design direction this spec absorbs
- **Related specs**:
  - [`timeseries.md`](timeseries.md) — declares `event_time_column`, `partition_column`, `granularity` (the time-dimension surface this spec consumes)
  - [`expansion.md`](expansion.md) — function expansion pass; runs before every analysis stage in this spec
  - [`sources.md`](sources.md) — host of `timeseries:` on external sources
  - [`models.md`](models.md) — frontmatter table; lists `timeseries:` and `incremental:` keys
  - [`architecture.md`](architecture.md) — planner role
  - future `materializations.md` for the broader materialization surface
- **Legacy reference**: `docs/DESIGN.md` § "Incremental Table Builds" — superseded by this spec for current behavior; useful for design rationale
