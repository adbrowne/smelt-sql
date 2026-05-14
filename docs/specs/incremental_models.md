---
feature: incremental_models
status: experimental
last_reviewed: 2026-05-05
owners: [andrew]
---

# Incremental Models

> **Scope.** Incremental materialization for time-partitioned models: configuration surface, the partition-based DELETE+INSERT execution strategy on DuckDB, safety checks the optimizer enforces, and the rules around what may be expressed in a logical incremental model.
>
> **Status: experimental.** The DuckDB DELETE+INSERT path is implemented and tested. MERGE on Spark/Databricks, schema-evolution, state tracking with gap detection, and per-column `data_latency` are planned (see `docs/plans/20260322-incremental-model-support.md`) and recorded under Known Divergences below.

## Surface

### YAML frontmatter (in `.sql` files)

```sql
---
materialization: table
incremental:
  enabled: true
  event_time_column: order_date
  partition_column: order_date
  granularity: day            # hour | day | week | month | quarter | year
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

### `smelt.yml` (project-level overrides)

Frontmatter wins over `smelt.yml` when both set the same field.

```yaml
models:
  daily_revenue:
    materialization: table
    incremental:
      enabled: true
      event_time_column: order_date
      partition_column: order_date
      granularity: day
```

### CLI

```
smelt run --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]
```

- Both flags are required for any incremental execution. Format: ISO-8601 (`2026-03-20`, `2026-03-20T00:00:00Z`).
- The end bound is exclusive: `--event-time-end 2026-03-25` does not include `2026-03-25`.
- `backbuild` uses the model's classified batch safety (see Semantics) to expand or split the requested range.

### Granularity values

`hour`, `day`, `week`, `month`, `quarter`, `year`.

- `week` accepts a configurable start day (e.g., Monday, Sunday). Default is Monday.
- Custom granularity via plugin is allowed but no plugins ship today.

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

For an incremental run with `[start, end)`:

1. **DELETE** from the output table where `partition_column >= start AND partition_column < end`.
2. **Inject** an AST-level `WHERE` filter on the model's logical SQL: `partition_column >= start AND partition_column < end`. The injection is per-model (whole-query), not per-`smelt.<path>` reference inside the body.
3. **INSERT** the filtered query's result into the output table.

This is idempotent: re-running the same `[start, end)` range produces the same final state.

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

### Batch safety classification

The optimizer analyzes the model's SQL via the CST and classifies each incremental model:

| Class               | Meaning                                                | Execution                                                |
|---------------------|--------------------------------------------------------|----------------------------------------------------------|
| `FullyBatchSafe`    | No temporal dependencies                                | Single query for any range                               |
| `BoundedSafe(n)`    | Bounded lookback/lookahead of `n` partitions            | Auto-sized chunks (3× context, clamped 7–90 partitions)   |
| `PerPartitionOnly`  | Unbounded temporal dependencies                         | Must process one partition at a time                     |

Detection sources: window-frame `ROWS BETWEEN`/`RANGE`, `LAG`/`LEAD`, JOIN with interval offsets, `WHERE` with interval patterns. (See `docs/specs/architecture.md` for the planner's role.)

### Safety checks (rejected by default)

The optimizer rejects an incremental model if its SQL uses constructs that break the partition-DELETE-then-INSERT contract or produce non-deterministic output:

- Window functions (`OVER (...)`)
- `HAVING`
- `LIMIT`
- Subqueries (`SELECT ... FROM (SELECT ...)`)
- Non-deterministic functions (`RANDOM()`, `NOW()` outside of stable contexts, etc.)
- `DISTINCT`

Each check can be individually disabled via `incremental.safety_overrides.allow_<check>: true`. Disabling is opt-in and recorded.

### Functions inside incremental bodies

An incremental model body may call transparent functions (`smelt.define`-resolved) and opaque calls (`smelt.extern` declarations, canonical built-ins, source references). Two interactions matter:

1. **Per-model WHERE injection happens at the outer query, not at call sites.** The framework's injected `WHERE partition_column >= start AND partition_column < end` clause is applied once at the outermost SELECT (Execution model step 2). It is **not** pushed into transparent function bodies or opaque call arguments. This is Constraint 4. Source-level filtering (pushing a predicate into a `smelt.<path>` reference inside the body) depends on temporal-dependency analysis and is planned, not implemented; in-body transparent-function expansion happens via the L1 `ExpandTransparentFunctionCalls` rule (`planner_integration.md`), not via WHERE pushdown.

2. **Transparent expansion happens before WHERE injection.** Conceptually, a transparent function call site is replaced with its body during planning, and only then is the per-model WHERE injected. The injected filter therefore applies to the columns visible *after* expansion — including any columns produced inside the transparent body — without the planner needing a separate pushdown rule.

**Batch-safety classification through call sites (current state).** Today, `analyze_batch_safety` and `analyze_temporal_dependencies` (`crates/smelt-planner/src/rules/incremental.rs`, `crates/smelt-planner/src/analysis/temporal.rs`) operate on the outermost CST after frontmatter strip — they do not walk transparent function bodies, and they do not have a special opaque-call branch. Consequences:

- A window-frame, `LAG`/`LEAD`, or temporal join lifted out of a transparent function and inlined at the call site is detected; one buried inside a `smelt.define` body is not yet detected. Aligning the classifier with `ExpandTransparentFunctionCalls` so it walks expanded bodies is open work — see Known Divergences.
- Opaque calls (`smelt.extern`, built-ins) are not flagged as `PerPartitionOnly` automatically. The classifier sees them as ordinary function calls and classifies based only on the constructs it does recognise. Authors of opaque calls with hidden temporal dependencies must encode the constraint via the model's frontmatter (e.g., declaring a partition-aligned shape) — there is no compiler-enforced check today.

The intent — once the classifier walks expanded transparent bodies, transparent calls inherit the body's class; opaque calls are treated conservatively (likely forcing `PerPartitionOnly`) — is captured here so future plans don't have to re-derive it. Cross-link: `planner_integration.md` §"Optimization boundary: transparent vs black-box".

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

## Constraints & Invariants

1. **Logical model is pure SQL.** No `is_incremental()`, no `{{ ... }}` macros, no conditional branches. The same SQL describes both the full-build and the incremental-build behavior; the framework injects the time filter.
2. **Strategy is not on the model.** Frontmatter declares `unique_key` and `partition_column`; the backend chooses `DeleteInsert` / `Merge` / etc. Model files do not name a strategy.
3. **smelt does not manage computational state.** Watermarks, offsets, and run-history live in the backend. The framework only generates SQL artifacts.
4. **Time filter injection is per-model.** The injected `WHERE` is applied to the outer model query once, not pushed into each `smelt.<path>` reference in the body. Source-level filtering depends on temporal-dependency analysis (planned); function-body filtering happens via the L1 expand-transparent-function rule (`planner_integration.md`), not via pushdown.
5. **Idempotence under fixed input.** For a given backend and unchanged source data, running the same `[start, end)` range repeatedly converges to the same output table state.
6. **Granularity is closed under partition arithmetic.** A `[start, end)` range must align to whole granularity units; partial-unit ranges are rejected.
7. **Safety check overrides are explicit.** A safety override must name the specific check it bypasses. There is no global "disable all safety checks" switch.

## Known Divergences / Open Questions

- **`-- @materialize: incremental` annotation never implemented.** `docs/DESIGN.md` § "Configuration Syntax" still shows the annotation form as an alternative; YAML frontmatter is the only implemented surface. The DESIGN.md prose marks this as a future option but the surface section in this spec is authoritative.
- **Lookback / temporal analysis is partial.** Batch safety classification exists in the optimizer; full inference of `lookback_days` from window frames and join intervals is not yet wired into the per-source filter injection.
- **Per-column `data_latency` not implemented.** Plan calls for declaring `data_latency` on upstream sources for late-arriving data; not yet available.
- **MERGE strategy is DuckDB-only-future.** `BackendCapabilities.supports_merge` is `true` for DuckDB / Spark / PostgreSQL, but the planner emits `DeleteInsert` for all three today. A Spark MERGE pathway is in the plan but unbuilt.
- **Three execution paths in `crates/smelt-cli/src/main.rs`.** Legacy, optimizer+incremental, incremental-only — Phase 1 of the incremental plan unified `IncrementalConfig` but the CLI dispatch is still tri-modal. Should converge.
- **Granularity conversion boilerplate.** A duplicate `Granularity` enum existed in `smelt-planner/src/types.rs` and was reconciled with `smelt-core`; check for residual conversion code in `main.rs` (lines around 669–683 in the plan reference) when next touching this area.
- **No interval / run-state tracking.** Skipped runs currently produce silent gaps (same failure mode as dbt). Tracking is planned but opt-in; see `run_state.md` anchor in `architecture.md` §"Specs not yet authored".
- **Schema evolution is unspecified.** A `partition_column` rename or an output schema change has no defined handling today.
- **`smelt.metric()` interaction.** The interaction between metric expansion and time-filter injection is not fully spelled out for incremental models that consume metrics.
- **Diagnostic codes pre-`diagnostics.md`.** Codes listed in this spec are owned here until a `diagnostics.md` spec lands. `diagnostics.md` will define ownership rules, severity tiers, stability tiers, and suppression. Code names may be renamed under that spec. (See `architecture.md` §"Specs not yet authored".)
- **Generator-emitted incremental models.** A `ModelDef` value emitted by a generator file (per `meta_language.md` §"Multi-model production") may carry `materialization: 'incremental'`, and the emitted model is subject to every rule in this spec on equal terms with a hand-authored incremental model — batch-safety classification, per-source filter injection, DELETE+INSERT execution, the first-run-and-backfill path. The `incremental:` block of an emitted model is inherited from the generator file's file-wide frontmatter; per-`ModelDef` overrides of `incremental.partition_column`, `incremental.granularity`, etc. are not part of the closed `ModelDef` field set in v1 (a future spec edit may add them). Until then, a single generator emits a single `incremental:` configuration shared by every emitted incremental model; users who need divergent per-emission incremental settings split the generator into multiple files. Tracked in `docs/plans/20260509-meta-language-overall.md`.

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
- **Related specs**: [`architecture.md`](architecture.md) for planner role; future `materializations.md` for the broader materialization surface
- **Legacy reference**: `docs/DESIGN.md` § "Incremental Table Builds" — superseded by this spec for current behavior; useful for design rationale
