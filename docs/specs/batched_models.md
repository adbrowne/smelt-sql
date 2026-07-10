---
feature: batched_models
status: experimental
last_reviewed: 2026-07-11
owners: [andrew]
---

# Batched Models

> **What this is.** The shape profile for `refresh: incremental` + `grain: partition` (`models.md` §"Refresh axis"): a partition-addressed table, one row per `(partition_column, …)`, kept current by the derived per-cell maintenance plan (`maintenance_plan.md`) rather than a declared strategy. "Batched" names the shape's default plan corner — recompute-a-region per touched partition, driven by DELETE+INSERT — not a mode the modeller selects: per `models.md` §Design ("Strategy content is derived; shape and grain stay declared"), which technique realizes which part of the output is a property of `(column-group × trigger)` cells, never of the model as a whole. This spec states which shared **properties** (`model_properties.md`) the partition grain requires, which **transforms** (`model_transforms.md`) its default plan drives, and defines in full the machinery that is partition-grain-**local**: the batch-safety classification, backfill chunking, column-locality of the equivalence, event-time outer-visibility, and the `safety_overrides`/per-column contract surface. It does **not** re-specify a shared capability. Out of scope, with their own homes: the equivalence invariant, the composition contract, the plan matrix, per-cell admission, and the graph layer (`maintenance_plan.md`); every reusable property the profile names — monotonicity trace, bound/reach derivation, partition alignment, determinism predicate, and the rest (`model_properties.md`); every physical mechanism the default plan drives — pushdown, DELETE+INSERT, the clamps, pinning (`model_transforms.md`); the time-dimension declaration `event_time_column`/`partition_column`/`granularity` (`timeseries.md`); the key-grain and `versioning: interval` shape profiles (`keyed_models.md`, `versioned_models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status that needs naming goes in §Known Divergences (behaviour + plan link) or §References → Plans (history). See the Timeless-oracle rule in `CLAUDE.md`.
>
> **Status: experimental.** The DuckDB DELETE+INSERT path is implemented and tested. MERGE on Spark/Databricks, schema-evolution, state tracking with gap detection, and per-column `data_latency` are planned and recorded under Known Divergences. The frontmatter surface described here (`refresh: incremental` + `grain: partition`, top-level `unique_key`/`safety_overrides`, `columns.<c>.contract`) is the live surface; `refresh: batched` is a hard error with a fix-it, while the `batched:` sub-block still parses as the profile's local options block and is refused without `refresh: incremental` + `grain: partition` (Known Divergences).

## Surface

### Composition

Per the composition contract (`maintenance_plan.md` §"The composition contract"), the partition-grain profile is composed as:

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: partition` — a complete table with a monotone `partition_column`, addressed by partition, not by key | `models.md` §"Refresh axis" |
| **Properties (required)** | event-time monotonicity trace; column nullability gate; unified bound/reach derivation; frame-reach taxonomy; injection-point / pushdown-depth; partition alignment (scoped); driving-fact / anchor resolution; determinism (run vs row) + nondeterminism predicate + taint; body-structure classifier; set-operation distribution; static-seed detection; window-independence / ordered-execution | `model_properties.md` |
| **World-facts (consumed)** | the timeseries clock (`event_time_column`/`partition_column`/`granularity`); source mutation profile and lateness margin (`sources.md`); the column-scoped equivalence contract (`columns.<c>.contract`) | `timeseries.md`, `sources.md`, `models.md` §"`columns:` — column metadata" |
| **Default plan (recompute corner)** | source-filter pushdown; partition DELETE+INSERT; output-window derivation (partition-column skew inversion); outer output-clamp; two-layer widened-scan + exact output clamp; compile-time pinning | `model_transforms.md` |
| **Admission** | every check below is one instance of `maintenance_plan.md` §"Per-cell admission" evaluated for the recompute-a-region corner over a partition-grain output (§"Per-cell admission mapping") | `maintenance_plan.md` |
| **Invariant upheld** | per-partition equivalence (the partition-grain specialisation of the framework's processed-input equivalence invariant, and of the plan's `S`-vector refinement) | `maintenance_plan.md` §"The equivalence invariant", `maintenance_plan.md` §"Per-cell admission" |

The normative content of this spec is that table plus the profile's **local** machinery defined below: the batch-safety roll-up, column-locality of the equivalence, event-time outer-visibility, backfill chunking, run/partition granularity alignment, and the partition-grain surface (`grain: partition`, `timeseries:` requirement, `safety_overrides`, per-source-clamp observability).

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
unique_key: [order_id]        # optional; backends with MERGE can use it; within-partition dedup aid only
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

`refresh: incremental` + `grain: partition` is the entire opt-in; it implies a stored `table`. `unique_key` and `safety_overrides` are top-level frontmatter keys (`models.md` §"YAML frontmatter keys") that apply only under `grain: partition`; `safety_overrides` on a key grain is a hard error (`models.md` §"Constraint violations").

A model with `grain: partition` must also declare `timeseries:` (`timeseries.md`). Missing the block produces a hard error at workspace load (`models.md` §"Constraint violations": "`grain: partition` without `timeseries`"). The declared `partition_column` must be **monotone** — validated by the event-time monotonicity trace (`model_properties.md` §"Event-time monotonicity trace"). Monotone admits either a timestamp *or* an ever-increasing integer (a sequence id / offset / watermark column): the trace recognises a constant shift over such a column (`batch_id + 5`, `batch_id - 5`) on the same footing as a constant `INTERVAL` shift over a timestamp column, while a non-monotone integer transform (`batch_id % n`, `batch_id * n`) is rejected fail-closed, naming the construct.

An output column's equivalence contract is the per-column `columns.<c>.contract` declaration (`models.md` §"`columns:` — column metadata", owned by `maintenance_plan.md`): `contract: plausible` exempts that column from the determinism requirement (audit stamps and surrogates the modeller accepts may vary) exactly where the pre-cut `nondeterministic_columns` list did. Listing `event_time_column`, `partition_column`, or a `unique_key` column as `plausible` is a configuration error (a skeleton position must be deterministic — `models.md` §"Constraint violations").

### `smelt.yml` (project-level overrides)

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

### CLI

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

- Both flags are required for any partition-grain execution. Format: ISO-8601 (`2026-03-20`, `2026-03-20T00:00:00Z`).
- The end bound is exclusive: `--event-time-end 2026-03-25` does not include `2026-03-25`.
- The supplied `[start, end)` range is the **run window**. It must be a positive integer multiple of `timeseries.granularity` aligned to granularity boundaries (`timeseries.md` §"Granularity arithmetic"). Run-window size may exceed partition granularity (Semantics §"Run window vs partition granularity").
- `backbuild` uses the model's classified batch safety (Semantics §"Batch safety classification") to expand or split the requested range.

### Granularity values

See `timeseries.md` §"Granularity values" for the closed enum. The profile consumes the granularity declared in the model's `timeseries:` block.

### Strategy enum (backend-internal)

Strategy is **not** declared on the model — it is derived per cell (`maintenance_plan.md` §"The plan matrix"). For the recompute corner the partition grain's default plan drives, backends pick a physical strategy from the model's config and their capabilities:

```rust
enum IncrementalStrategy {
    DeleteInsert,    // DELETE matching partitions + INSERT
    Append,          // insert-only; no dedup
    InsertOverwrite, // replace entire partitions atomically
}
```

DuckDB currently always uses `DeleteInsert`. UPSERT (`MERGE`) is **not** a partition-grain strategy — it is the keyed `merge_into` transform (`model_transforms.md`) the key-grain default plan drives, whose column families carry the end-state (not per-partition) equivalence contract (`keyed_models.md`).

## Semantics

### Execution model (DuckDB, current)

For a partition-grain run with run window `[start, end)`, the recompute corner drives four transforms from `model_transforms.md`:

1. **Partition DELETE** from the output table where `partition_column` falls in the **derived output window** — the run window pushed through the model's declared partition-column relation (`model_transforms.md` §"The output window is derived, never assumed"): identity when the `partition_column` tracks event time, so `output window = run window`; skew-inverted when the `partition_column` is *derived* and skews away from the driving date column, declared by a Form B relation. For such a **write-rebasing model** (e.g. a session keyed by `session_start_date` gaining events the next day, `before = after = 1 day`) the output window for run `[D, D+1)` is `[D−1, D+2)`, so the DELETE covers **every** partition the INSERT will write — including the prior-day partition the new data reaches. Deleting only the run window would strand the skew-reached partition stale forever: no later run's window contains it.
2. **Outer output-clamp** — inject `WHERE partition_column >= out_start AND partition_column < out_end` at the outermost SELECT, constraining the model's *output* to the same derived output window the DELETE covers. This step is **dropped for the transparent slice** (exactly one timeseries source, zero-margin bound `Bounded(_, 0, 0)`, no partition-column skew): the per-source pushdown filter already *is* the output clamp, so a second textually identical outer `WHERE` is redundant (Injection-point / pushdown-depth property; `model_transforms.md` §"Source-filter pushdown + the two clamps"). A genuine lookback margin, a partition-column skew, or more than one timeseries source keeps the outer clamp: scan window and output window are then distinct, load-bearing windows. Each written partition's **scan** is sized from the derived output window's reach, never the run window's — rewriting a skew-reached neighbour partition from a scan sized for the run window would under-read that partition's own reach.
3. **Source-filter pushdown** — inject a per-source `partition_column` filter on each `smelt.<path>` reference, derived from the model's SQL. Sources without a `timeseries:` declaration are lookups: no bound, read in full.
4. **INSERT** the resulting query's output into the output table.

DELETE range and output clamp are derived from **one** window so the contract stays idempotent for any write-window width. Re-running the same `[start, end)` under fixed input converges to the same final state (Constraint: idempotence). Per-partition equivalence holds (Semantics §"Per-partition equivalence").

The derived output window is a range to be **covered**, not a mandate for one statement. Backfill chunking (§"First-run and backfill") splits it into sequential DELETE+INSERT pairs the same way it splits a wide run window — the production pattern of running a multi-day update as several bounded sequential queries rather than one large one, each chunk's scan sized from that chunk's own reach (`model_transforms.md` §Design "Derived output window composes with chunking").

### Run window vs partition granularity

The CLI `[--event-time-start, --event-time-end)` declares a **run window**, not a per-partition invocation. It must be a positive integer multiple of `timeseries.granularity` aligned to granularity boundaries (`timeseries.md` §"Granularity arithmetic"); within that, run-window size and partition-granularity unit are independent. A daily-partitioned model run with a 30-day window is **one** engine query (sources filtered to the union of the run window and each source's pushdown bound; output clamped to the run window) and **one** partition-aligned DELETE over the 30 partitions followed by one INSERT. Backfilling 60 days is one `smelt run --event-time-start D --event-time-end D+60d`, not 60 daily invocations. Per-partition equivalence holds regardless of run-window size.

The declared `timeseries.granularity` (`g_run`, since it governs run-window alignment) must be at least as coarse as the granularity actually implied by the `partition_column` projection's truncation/grid transform (`g_part`) — derived independently from the model's SQL, not merely trusted from the declaration. A model whose `partition_column` is `DATE_TRUNC('day', event_time)` has `g_part = day`; declaring `granularity: hour` on that model is rejected, because an hourly run window does not correspond to the model's real (daily) partitions and would misalign the DELETE+INSERT contract. `g_run >= g_part` is checked under the closed enum's increasing-coarseness ordering (`hour < day < week < month < quarter < year`, `timeseries.md` §"Granularity values"); `g_run == g_part` or `g_run` coarser than `g_part` both pass. When `g_part` cannot be derived (an opaque projection the classifier has no rule for), this comparison is skipped — undecided, not a positive disproof — and only the declared-granularity alignment check applies. This is enforced with hard validation: a sub-`g_part` run window is rejected with a message naming the minimum window, never silently widened or coarsened to fit (Known Divergences).

### Batch safety classification

The optimizer rolls the per-source bound map (Property: *unified bound/reach derivation*, `BoundResult` per source) into a single **partition-grain-local** class per model. This roll-up is meaningful only inside the recompute-a-region execution shape and is owned here:

| Class               | Meaning                                                                 | Execution                                                |
|---------------------|-------------------------------------------------------------------------|----------------------------------------------------------|
| `FullyBatchSafe`    | All timeseries sources `Bounded(_, 0, 0)`; no temporal dependencies     | Single query for any run window                          |
| `BoundedSafe(n)`    | All timeseries sources `Bounded`, with `n = max(before + after)` > 0    | Auto-sized chunks (3× context, clamped 7–90 partitions)  |
| `PerPartitionOnly`  | One or more timeseries sources `Unbounded` (cumulative-across-history)  | One partition at a time, sequential                      |

`n` for `BoundedSafe` is rendered in the source's partition-column unit and is the same value the source-filter pushdown transform reads.

A model with **any** `NotDerivable` source is **refused at planning time**, not assigned a class — the optimizer cannot prove the partition-DELETE+INSERT contract is safe (`MaintenanceReachNotDerivable`, `maintenance_plan.md` §"Per-cell admission" obligation 4). The diagnostic names the offending construct and the source-map points at the original SQL. The author rewrites into a derivable form or removes the dependency. There is **no silent downgrade to full-refresh** (`maintenance_plan.md` §"Validator, not chooser").

**Wide single-batch builds.** When `FullyBatchSafe` causes a single-batch build spanning more than 30 partition periods, smelt warns and recommends `--per-partition` or `--batch-size <n>`. The warning is informational; both `--per-partition` and `--batch-size` suppress it (the user has opted into a safe batching shape).

### First-run and backfill

A first run (no output table) and a backfill (re-run of a written range) follow the same DELETE+INSERT contract — the DELETE is a no-op when the partition is absent. The planner picks a **backfill-chunking** shape (a partition-grain-local transform, `model_transforms.md` §"Transforms that stay in a mode spec") from the batch-safety class:

| Class                | Chunking                                                                                   |
|----------------------|--------------------------------------------------------------------------------------------|
| `FullyBatchSafe`     | A single DELETE+INSERT pair covers any `[start, end)`. No chunking.                        |
| `BoundedSafe(n)`     | Auto-sized sub-ranges (3× context, clamped 7–90 partitions). Each sub-range is one DELETE+INSERT pair, executed sequentially in temporal order. |
| `PerPartitionOnly`   | One partition per iteration, sequential, temporal order. Each partition is one DELETE+INSERT pair. |

**Per-partition batching is calendar-aligned for Month/Quarter/Year.** When per-partition execution is forced (or `smelt backbuild --per-partition` is requested), batches for `Month`/`Quarter`/`Year` advance by true calendar units, so every batch lands on a month/quarter/year boundary regardless of month length. `Day` and `Week` use fixed 1-day / 7-day steps.

**Output grain may be finer than partition grain.** A model whose `partition_column` holds monthly boundaries may emit daily/hourly rows within them; batch-splitting operates on the *partition* grain and writes/reads finer rows in their entirety within each partition batch.

**Per-chunk transaction boundary.** Each chunk's DELETE+INSERT is one backend transaction. INSERT failure rolls back the chunk's DELETE; earlier committed chunks do **not** roll back — partial progress is intentional since each chunk is idempotent.

**Failure mode.** A run halts at the first failed chunk and exits non-zero. Re-running the same `[start, end)` resumes correctly because every committed chunk is idempotent.

**Late-arriving data (interim guidance).** smelt does **not** auto-re-run partitions when data arrives late. Two interim mitigations: (1) trail `--event-time-end` behind real-time by the source's known latency; (2) run overlapping ranges (e.g. always re-process the last 7 days). A planned automated mechanism is per-column `data_latency:` (Known Divergences).

### Per-partition equivalence

For every partition `p` in the run window `[run_start, run_end)`:

```
partition_grain_run(model, [run_start, run_end)).where(partition_column = p)
  == full_refresh(model).where(partition_column = p)
```

This is the partition-grain specialisation of the framework's processed-input equivalence invariant (`maintenance_plan.md` §"The equivalence invariant"), and of the plan's `S`-vector refinement (`maintenance_plan.md` §"Per-cell admission"). It is independent of run-window size.

**Column-locality (partition-grain-local).** The equality holds for **local** columns — those whose value depends only on source rows visible within the model's source-filter ranges. A column depending on history outside those ranges (a cumulative aggregation such as connected-components or backward-fill) is **not equivalent**: its per-partition value reflects state at run time, not the final cumulative state. Such a column forces its source to `Unbounded` and the model to `PerPartitionOnly`; the run is correct as-of-the-run, just not equal to a full refresh that re-runs every partition with the final input.

**Equivalence is up to full-refresh non-determinism.** The equality is bit-identical on **deterministic** columns. A column with `contract: plausible` need only be a *plausible full-refresh value*. This never extends to a column that governs *which* rows exist, *where* they are partitioned, or *how* they are deduplicated (Semantics §"Safety checks").

### Safety checks (per-cell admission for the partition grain's recompute corner)

The optimizer rejects a partition-grain model whose SQL uses constructs that break the partition-DELETE-then-INSERT contract. Each check applies a shared `model_properties.md` proof to discharge one of `maintenance_plan.md` §"Per-cell admission"'s obligations for the recompute-a-region corner over this output shape; the table below names, for each check, the obligation it instantiates. Each check is individually disabled via `safety_overrides.allow_<check>: true` (opt-in, recorded).

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

### Event-time outer-visibility (partition-grain-local)

The outer output-clamp injects a `WHERE event_time_column >= start AND event_time_column < end` at the outermost SELECT. For that to bind correctly, `event_time_column` must be **accessible** there. A plain `UNION`/`INTERSECT`/`EXCEPT`, or a `UNION ALL` whose branches cannot be proven traceable, would bind the clamp to only the first branch and produce wrong results; a subquery FROM that does not project `event_time_column` references an inaccessible column. Either case is rejected with `EventTimeColumnNotVisibleAtOuterSelect` (Error) before execution.

A `UNION ALL` is **exempt** when every branch's projection of `event_time_column` traces `Traceable` (Property: *event-time monotonicity trace*; distributed by *set-operation distribution*) back to a real source's own partition column: per-source pushdown then narrows each branch's scan independently and the outer clamp's placement is immaterial. A `StaticSeed` branch is named and rejected; a `NotTraceable` branch conservatively keeps the whole-model outer clamp.

### Observing the per-source clamp (partition-grain-local surface)

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

### Functions inside partition-grain model bodies

A partition-grain model body may call transparent functions (`smelt.define`-resolved) and opaque calls (`smelt.extern`, canonical built-ins, source references). Function expansion (`expansion.md`) runs **before** every analysis stage here — bound derivation, source-filter pushdown, and most batch-safety sub-checks all see the expanded CST, so a `LAG()` inside a `smelt.define` body and one inlined at the call site are indistinguishable. The outer output-clamp is injected at the outermost expanded query and so sees columns produced inside expanded bodies; source-filter pushdown reaches `smelt.<path>` references that originated inside a `smelt.define` body via expansion. **Exception:** the `OVER`-clause admissibility sub-check scans the outer model SQL before expansion (Known Divergences).

**Opaque calls remain black boxes.** Bound derivation cannot read through `smelt.extern`/built-ins. A partition-grain model whose time-dependence is hidden behind an opaque call is `NotDerivable` and refused, unless a bound is provable from the surrounding SQL (a WHERE clause, an explicit RANGE-windowed projection). Cross-link: `planner_integration.md` §"Optimization boundary: transparent vs black-box".

### Window independence and self-referential models

Whether windows may be built **in parallel** or must be built **sequentially in temporal order** is the *window-independence / ordered-execution* property (`model_properties.md`), derived from the model's dependency graph, never declared. The partition grain's application:

- **Window-independent (the default).** Every window is a pure function of source rows in its own scan range (widened by the derived lookback). The entire safe slice the recompute corner admits is window-independent — the lookback reaches into *sources*, never the model's own earlier partitions — so a backfill of `[t₀, tₙ)` may split into sub-ranges built in any order, including in parallel.
- **Window-dependent → ordered.** A **self-referential** partition-grain model — one reading its own prior partitions via `smelt.<self>` (a running balance, a partition-by-partition state machine) — is **in scope** and still executes as partition DELETE+INSERT (it stays a partition-addressed table; it does **not** become key-grain), but the runtime must build its windows **sequentially in strict temporal order**, and its backfill may not be parallelised or reordered. A self-edge the planner cannot prove converges partition-by-partition (a self-reference reading *forward* or across all history) is refused at planning time, not silently mis-parallelised.

This is the same stateless/stateful spine that separates the partition grain from the key grain: a self-referential partition-grain model is *stateful-ordered* in execution yet keeps the partition-grain *output shape* (partitioned, per-partition-equivalent within each window's input).

### State ownership

smelt does not track watermarks, offsets, or run history for partition-grain models — the backend owns computational state (DuckDB: table state + transactions; future Delta/Spark: transaction log + MERGE; future Flink: checkpoints). Optional run-state tracking with gap detection is opt-in via the `state.mode: intervals` posture (`virtual_environments.md`); the on-disk layout is owned by `run_state.md`.

### `partition_column` validation

Partition-column projection is owned by `timeseries.md` §"Constraints & Invariants" rule 1: `partition_column` must appear in the model's output `SELECT` (and in the `GROUP BY` when grouping is present), else `MalformedTimeseries`. The partition-grain rule consumes that guarantee rather than re-checking.

## Design

This section captures the partition-grain-**specific** rationale; the rationale for each shared property/transform lives in its owning spec, and the rationale for deriving strategy while declaring grain lives in `models.md` §Design and `maintenance_plan.md` §Design.

**Logical SQL is pure; the framework injects the time filter.** A model body never contains `is_incremental()` or any conditional branching on full-vs-incremental. The same SQL is both descriptions; the framework injects the outer clamp and drives pushdown. *Jinja-style `is_incremental()` branching* (dbt) was rejected because it splits one model into two implicit ones that drift. The trade-off — partition-grain models must accept the framework's per-model filter shape — is policed by the batch-safety analysis.

**DELETE+INSERT over partition columns, not MERGE, for v1.** DuckDB's strategy is `DeleteInsert`. *MERGE* was rejected as the v1 default because it requires a `unique_key` (not every model has one) and carries cross-engine subtleties; it stays in the `IncrementalStrategy` enum for backends that opt in. DELETE+INSERT is idempotent under fixed input and aligns with the partition-column safety analysis.

**Three-class batch-safety taxonomy.** The `FullyBatchSafe` / `BoundedSafe(n)` / `PerPartitionOnly` roll-up (Semantics §"Batch safety classification") is partition-grain-local because it is meaningful only for this execution shape. *A binary safe/unsafe flag* was rejected — too many real workloads are bounded-safe and need auto-chunking. *A continuous safety score* was rejected — the user-facing decision is qualitative and maps directly to three backend-execution shapes.

**Derive lookback from the model's SQL, not from frontmatter.** The per-source bound is computed by the shared bound/reach derivation over the model's SQL (including inlined `smelt.define` bodies), not a `lookback_days:` YAML annotation, which would let declaration and logic drift (`feedback_derive_dont_declare`). The trade-off — a model with implicit time logic refuses partition-grain eligibility and must be rewritten into a derivable form — is arguably the right outcome. Deriving from SQL removes the artifact the author would read to confirm behaviour, so the derived clamp is made **observable** (Semantics §"Observing the per-source clamp") as the deliberate counterpart. Deeper rationale: `docs/research/20260521-incremental-as-planner-rule.md`.

**smelt does not own state — scoped to the partition grain.** Watermarks, run history, and offsets live in the backend; *owning a watermark store* was rejected as a v1 requirement because it duplicates engine state and opens a sync-correctness window. Optional run-state tracking is an opt-in extension. This doctrine is **specific to the partition grain**: `grain: key` maintains one deliberate exception, the transactional merge ledger (`keyed_models.md` §"The transactional merge ledger") — a small backend-resident table written in the *same transaction* as the window's merge, so it cannot drift from the state it records and does not reopen the sync-correctness window this doctrine guards against. A consequence of the ledger's correctness role: a backend may only select a physical strategy that preserves the declared shape's invariants, which is why the partition-grain `Append` strategy below is unreachable until it is gated on ledger-verified unwritten windows (`docs/research/20260705-keyed-collapse-application.md` D7) — an unguarded append-only write could not detect a re-run without the ledger's bookkeeping.

**Non-determinism is opted in per column, and confined by proof.** Whether a column is acceptable-to-vary is a value judgement only the author holds, so it is **declared** (`columns.<c>.contract: plausible`) — the one place the derive-don't-declare default correctly yields. *A whole-model `allow_nondeterministic` boolean* was rejected as the primary mechanism because it drops the guardrail keeping non-determinism out of the skeleton roles. The per-column opt-in keeps the guardrail and still proves, by the shared taint flow, that the tolerance did not leak into the deterministic skeleton. Derivation: `docs/research/20260703-model-updates.md` §9.2.

## Constraints & Invariants

1. **Logical model is pure SQL.** No `is_incremental()`, no macros, no conditional branches. The framework injects the time filter.
2. **`timeseries:` is required for `grain: partition`.** A model with `grain: partition` and no `timeseries:` block is a hard error at workspace load (`models.md` §"Constraint violations").
3. **Strategy is not on the model.** Frontmatter declares `unique_key`; the backend chooses `DeleteInsert`/`Merge`/etc. for the recompute corner's execution.
4. **smelt does not manage computational state — a partition-grain-scoped doctrine.** Watermarks, offsets, and run-history live in the backend. The one deliberate exception across the refresh axis is `grain: key`'s transactional merge ledger (`keyed_models.md`), which is backend-resident and transactional-with-the-merge rather than a separate synced store, so it does not reintroduce the sync-correctness window this constraint guards against. A backend may select only a physical strategy that preserves the declared shape's invariants; the partition grain's `Append` strategy (below) is unreachable until it is gated on ledger-verified unwritten windows.
5. **Output-filter injection is per-model; source-filter pushdown is per-reference.** The outer clamp is applied once at the outermost SELECT; pushdown filters are applied per `smelt.<path>` reference in the expanded body.
6. **Per-partition equivalence with full refresh, up to full-refresh non-determinism.** For every partition `p` in the run window, the partition-grain output `where(partition_column = p)` equals the full-refresh output for `p` on all local, deterministic columns; a `columns.<c>.contract: plausible` column need only be a plausible full-refresh value; globally-dependent columns are not equivalent (Semantics §"Per-partition equivalence").
7. **Idempotence under fixed input.** Re-running the same run window on unchanged sources converges to the same output table state.
8. **Granularity is closed under partition arithmetic.** A run window must align to whole granularity units; partial-unit ranges are rejected. The declared granularity must also be at least as coarse as the granularity independently derived from the `partition_column` projection's own truncation transform (`g_run >= g_part`); a declared granularity finer than the derived partition grid is rejected (Semantics §"Run window vs partition granularity").
9. **Safety-check overrides are explicit.** A `safety_overrides` entry names the specific check it bypasses; there is no global disable.
10. **No silent downgrade to full-refresh.** A model the safety classifier rejects, or whose bound derivation is `NotDerivable`, is refused at planning time with a diagnostic, never a silent fall back to full-table execution (`maintenance_plan.md` §"Validator, not chooser").
11. **`event_time_column` must be accessible at the outermost SELECT, unless every UNION ALL branch traces `Traceable`.** Otherwise `EventTimeColumnNotVisibleAtOuterSelect` (Error) fires at the diagnostic gate (Semantics §"Event-time outer-visibility").
12. **Non-determinism stays in the payload.** Non-deterministic SQL is admitted only when its value flows exclusively into a `columns.<c>.contract: plausible` column (except the run-nondeterministic class as a direct projection); it must never reach `event_time_column`, `partition_column`, a `unique_key` column, or any membership/grouping position. Declaring an excluded column `plausible` is a configuration error.

## Known Divergences / Open Questions

- **The mode value is cut; the sub-block remains.** `refresh: batched` is a hard error with a fix-it naming `refresh: incremental` + `grain: partition` (`crates/smelt-core/src/config.rs`); the `batched:` sub-block (`batched.unique_key`, `batched.nondeterministic_columns`, `batched.safety_overrides`) is still the live surface for those options and is refused without `refresh: incremental` + `grain: partition` (`crates/smelt-core/src/metadata.rs`). Top-level `unique_key`/`safety_overrides` do not yet parse; `columns.<c>.contract` does (`models.md` §Known Divergences). The `smelt migrate` assist does not exist. Delivered/tracked by `docs/plans/20260707-maintenance-plan-impl.md`.
- **`nondeterministic_columns` predates `columns.<c>.contract`.** The pre-cut `batched.nondeterministic_columns` list and the target `columns.<c>.contract: plausible` declaration are the same mechanism under two surfaces; the column-scoped `contract` key is owned by `models.md` §"`columns:` — column metadata" (semantics: `maintenance_plan.md`). The `columns.<c>.contract` key parses today; the pre-cut list form remains the live surface inside the `batched:` sub-block (previous divergence).
- **A self-referential model's very first partition cannot be created via `CREATE TABLE ... AS SELECT ...`.** DuckDB (and SQL generally) cannot resolve a table to itself mid-creation, so a self-referential partition-grain model's target table must already exist (e.g. seeded with an opening row before the run window) before the first backfill batch runs; there is no automatic bootstrap. Tracked in `docs/plans/20260704-model-updates-l4-batched.md`.
- **Rule module file still carries the old spelling.** `crates/smelt-logical/src/rules/incremental.rs` retains the file path; the diagnostic codes (`TimeseriesRequiredForBatched`, `CumulativeForbidsBatched`, `BatchedNotSafe`) and config types (`BatchedConfig`, `BatchedSafetyOverrides`) are renamed. A pure internal file rename is deferred.
- **One non-hot classification call site still reads the outer SQL body.** The bound-`NotDerivable` refusal gate (`derive_model_source_bounds`, pure planner) classifies on the outer `model.sql`; a lookback living only inside a function body with no outer Form B filter is the sole case that would behave differently, and none exists in the repo. Tracked in `docs/plans/20260530-thread-fn-registry-classification.md`.
- **Window-function batch-safety check runs on unexpanded outer SQL.** `find_inadmissible_over` scans the outer model SQL before function expansion, so an `OVER` clause inside a `smelt.define` body is invisible to it. Tracked in `docs/plans/20260530-thread-fn-registry-classification.md`.
- **Per-source clamp observability partly emitted.** `smelt explain --json` reports `source_partition_col` and `(before, after)` offsets but does not yet resolve the run-relative scan window even when a run window is supplied; the editor-hover readout is not yet implemented (LSP hover is type/column/ref oriented). Both are specified ahead of a plan.
- **Output-window derivation is unbuilt; the runtime pins the output window to the run window.** The two-layer widened-scan/exact-clamp split is built (the scan margin is read and never re-written), but both the DELETE range and the output clamp use the batch's run window verbatim — no skew inversion. A write-rebasing model (§"Execution model" item 1) therefore silently under-writes: the run receiving skew-reaching data computes the correct neighbour-partition row and clamps it away, and no later run's window contains that partition. Observable divergence: `examples/web_analytics` `silver.sessions` under single-day replay keeps a stale prior-day session row when a session crosses midnight (deterministic repro in `docs/plans/20260710-web-analytics-maintenance-demo.md` §"Deferred during implementation"). The Rust `per_partition_equivalence` harness's session assertion compares only `(session_id, utm_campaign)` — both invariant under this failure — so it must also assert `(session_end, event_count)` when this lands. Tracked in `docs/plans/20260711-derived-output-window.md`.
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

## References

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
- **Related specs**:
  - [`maintenance_plan.md`](maintenance_plan.md) — the equivalence invariant, algebraic ladder, and composition contract this profile composes; the plan matrix, per-cell admission, and the graph layer its admission instantiates
  - [`model_properties.md`](model_properties.md) — the properties this profile requires (monotonicity trace, bound/reach, partition alignment, determinism predicate, …)
  - [`model_transforms.md`](model_transforms.md) — the transforms the recompute corner drives (pushdown, DELETE+INSERT, the clamps, pinning)
  - [`models.md`](models.md) — the refresh axis, declared grain, three-state declaration law, input-consumption axis, litmus rule
  - [`timeseries.md`](timeseries.md) — declares `event_time_column`, `partition_column`, `granularity`
  - [`expansion.md`](expansion.md) — function expansion; runs before every analysis stage here
  - [`sources.md`](sources.md) — host of `timeseries:` and source-lateness/mutation-profile world-facts
  - [`keyed_models.md`](keyed_models.md), [`versioned_models.md`](versioned_models.md), [`materialized_view.md`](materialized_view.md) — the other shape profiles (key-grain and engine-owned)
  - [`multi_backend.md`](multi_backend.md) — backend capability flags a strategy checks
- **Legacy reference**: `docs/DESIGN.md` §"Incremental Table Builds" — superseded for current behavior; useful for design rationale
