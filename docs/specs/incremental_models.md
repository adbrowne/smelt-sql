---
feature: incremental_models
status: experimental
last_reviewed: 2026-04-27
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
FROM smelt.models.orders
GROUP BY order_date, customer_id
```

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

### `partition_column` validation

The optimizer requires `partition_column` to appear in both the `SELECT` list and the `GROUP BY` (when grouping is present). A model whose SELECT does not project `partition_column` is rejected before execution.

### State ownership

smelt does not track watermarks, offsets, or run history for incremental models. The backend owns computational state:

- DuckDB: table state and transaction history.
- Future Delta/Spark: transaction log; MERGE strategy.
- Future Flink: checkpoints.

Optional run-state tracking with gap detection is planned (Phase 5 of the incremental plan) and is opt-in.

## Constraints & Invariants

1. **Logical model is pure SQL.** No `is_incremental()`, no `{{ ... }}` macros, no conditional branches. The same SQL describes both the full-build and the incremental-build behavior; the framework injects the time filter.
2. **Strategy is not on the model.** Frontmatter declares `unique_key` and `partition_column`; the backend chooses `DeleteInsert` / `Merge` / etc. Model files do not name a strategy.
3. **smelt does not manage computational state.** Watermarks, offsets, and run-history live in the backend. The framework only generates SQL artifacts.
4. **Time filter injection is per-model.** The injected `WHERE` is applied to the outer model query once, not pushed into each `smelt.<path>` reference in the body. Source-level filtering depends on temporal-dependency analysis (planned).
5. **Idempotence under fixed input.** For a given backend and unchanged source data, running the same `[start, end)` range repeatedly converges to the same output table state.
6. **Granularity is closed under partition arithmetic.** A `[start, end)` range must align to whole granularity units; partial-unit ranges are rejected.
7. **Safety check overrides are explicit.** A safety override must name the specific check it bypasses. There is no global "disable all safety checks" switch.

## Known Divergences / Open Questions

- **`-- @materialize: incremental` annotation never implemented.** `docs/DESIGN.md` § "Configuration Syntax" still shows the annotation form as an alternative; YAML frontmatter is the only implemented surface. The DESIGN.md prose marks this as a future option but the surface section in this spec is authoritative.
- **Lookback / temporal analysis is partial.** Batch safety classification exists in the optimizer; full inference of `lookback_days` from window frames and join intervals is not yet wired into the per-source filter injection.
- **Per-column `data_latency` not implemented.** Plan calls for declaring `data_latency` on upstream sources for late-arriving data; not yet available.
- **MERGE strategy is DuckDB-only-future.** `BackendCapabilities.supports_merge` is `true` for DuckDB / Spark / PostgreSQL, but the planner emits `DeleteInsert` for all three today. A Spark MERGE pathway is in the plan but unbuilt.
- **Three execution paths in `crates/smelt-cli/src/main.rs`.** Legacy, optimizer+incremental, incremental-only — Phase 1 of the incremental plan unified `IncrementalConfig` but the CLI dispatch is still tri-modal. Should converge.
- **Granularity conversion boilerplate.** A duplicate `Granularity` enum existed in `smelt-optimizer/src/types.rs` and was reconciled with `smelt-core`; check for residual conversion code in `main.rs` (lines around 669–683 in the plan reference) when next touching this area.
- **No interval / run-state tracking.** Skipped runs currently produce silent gaps (same failure mode as dbt). Tracking is planned but opt-in.
- **Schema evolution is unspecified.** A `partition_column` rename or an output schema change has no defined handling today.
- **`smelt.metric()` interaction.** The interaction between metric expansion and time-filter injection is not fully spelled out for incremental models that consume metrics.

## References

- **Code**:
  - `crates/smelt-core/src/config.rs` — `IncrementalConfig`, `Granularity`, `Weekday`
  - `crates/smelt-core/src/metadata.rs` — frontmatter extraction, `ModelMetadata`
  - `crates/smelt-core/src/sources.rs` — `SourceColumnDef` (future home of `data_latency`)
  - `crates/smelt-optimizer/src/rules/incremental.rs` — detection + safety checks
  - `crates/smelt-optimizer/src/types.rs` — safety-override types
  - `crates/smelt-cli/src/transformer.rs` — `inject_time_filter()`
  - `crates/smelt-cli/src/executor.rs` — `execute_model_incremental()`, `execute_plan_incremental()`
  - `crates/smelt-cli/src/main.rs` — CLI dispatch (incremental paths around the `run` / `backbuild` subcommands)
  - `crates/smelt-backend/src/lib.rs` — `Backend::delete_partitions()`, `Backend::insert_into_from_query()`
  - `crates/smelt-backend-duckdb/src/lib.rs` — DuckDB `DeleteInsert` impl
  - `crates/smelt-dialect/src/dialect.rs` — `BackendCapabilities::supports_merge`
- **Tests**: 17 optimizer unit tests in `crates/smelt-optimizer/src/rules/incremental.rs`; CLI integration tests in `crates/smelt-cli/tests/incremental_*.rs`; 7 optimizer integration tests; 13 metadata tests
- **User docs**: [`docs-site/docs/guide/incremental-models.md`](../../docs-site/docs/guide/incremental-models.md), [`docs-site/docs/guide/materializations.md`](../../docs-site/docs/guide/materializations.md)
- **Plans (history)**:
  - [`docs/plans/20260322-incremental-model-support.md`](../plans/20260322-incremental-model-support.md) — comprehensive plan; many phases still open
  - [`docs/plans/20260325-materialization-types.md`](../plans/20260325-materialization-types.md)
- **Related specs**: [`architecture.md`](architecture.md) for planner role; future `materializations.md` for the broader materialization surface
- **Legacy reference**: `docs/DESIGN.md` § "Incremental Table Builds" — superseded by this spec for current behavior; useful for design rationale
