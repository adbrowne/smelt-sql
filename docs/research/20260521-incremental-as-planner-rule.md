# Research: Incremental as a planner rule, with SQL-derived per-source bounds

**Date**: 2026-05-21
**Topic**: Design direction for the next iteration of smelt's incremental story — refactoring incremental into a planner-rule extension, lifting `timeseries:` metadata into core, and deriving per-source filter bounds from the SQL rather than declaring them in YAML
**Branch**: worktree-web_analytics
**Predecessor**: `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md`

## Summary

The predecessor doc catalogued seven gaps surfaced by converting `examples/web_analytics/` to day-by-day. This doc proposes a design direction that closes gaps #1 (silent safety downgrade), #2 (over-coarse `OVER` rejection), #3 (lookback declared in metadata rather than derived), and #4 (no source-filter pushdown) under a single architectural shape. Gap #5 (MERGE / cumulative materialization) is a sibling strategy and out of scope here. Gap #7 (as-of-day-D divergence with global identity) falls out as the limit case of the same analysis: a source with unbounded lookback can't be incremental on that column.

Six architectural moves carry the design:

1. **Equivalence is the formal property.** Per-partition: `incremental_run[D] == full_refresh().where(partition_col = D)`. Cumulative-across-history equivalence on global aggregations isn't achievable without rewriting prior partitions; the design exposes this as "unbounded source → refuse incrementality" rather than masking it.

2. **Derive from SQL, not from YAML.** The SQL is the canonical statement of a model's time-dependency. Window-frame `RANGE BETWEEN INTERVAL '…' PRECEDING` (Form A) and explicit date filters on JOIN/WHERE clauses (Form B) are read directly by the planner. Models that can't express their time-dependency in standard SQL constructs refuse incrementality; the author rewrites.

3. **Function expansion as a logical pass in core.** Function bodies stop being opaque. A logical pass substitutes `smelt.functions.foo(args)` calls with the function body, with arguments bound, producing an inlined CST. All downstream analysis (safety classifier, lookback derivation, pushdown) runs on the inlined form. Source-maps thread back to original locations for diagnostics. Physical execution remains free to compile functions as CTEs or otherwise — the inlining is for analysis.

4. **Per-source bound abstraction.** Generalises "lookback" to `(source_partition_col, before, after)` per source-reference. The same machinery handles same-column lookback, timezone rebasing (different column, ±24h), range-joins, and unbounded cumulatives. Different sources on the same model get independent bounds.

5. **Incremental is a planner rule, not core.** Core smelt owns parse, type-check, dependency graph, workspace loading, the function-expansion pass, the `timeseries:` metadata, and the planner-rule API. The incremental rule consumes those and owns the batch-safety classifier, lookback derivation, source-filter pushdown, and the DELETE+INSERT physical strategy. Other rules (append-only, snapshot, MERGE, CDC) are siblings.

6. **Partition size ≠ run-window size.** `timeseries:` declares partition granularity. The CLI / driver passes a run window that is any positive integer multiple of the partition granularity. One engine query covers the whole run window with the appropriately-widened source filters; DELETE+INSERT writes are still aligned to partitions. Backfill amortises engine startup and source scans across many partitions in one call.

## Key Files (current state, before this work)

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-planner/src/rules/incremental.rs` | Batch-safety classifier (rejects outer-body `OVER`, `HAVING`, `LIMIT`, etc.) | `analyze_safety_checks` |
| `crates/smelt-cli/src/transformer.rs` | `inject_time_filter` — appends WHERE to outermost SELECT only | L56-97 |
| `crates/smelt-cli/src/commands/run.rs` | Run loop; logs planner errors as `warn!` and proceeds (silent downgrade) | L470 |
| `crates/smelt-backend/src/lib.rs` | `execute_model_incremental` — DELETE+INSERT is the only incremental strategy wired | L196-225 |
| `docs/specs/incremental_models.md` | Current normative spec for incremental — owns most of what becomes the rule spec | full file |
| `docs/specs/architecture.md` | Records architectural invariants like the project-isolation rule and workspace-loading-parity rule | full file |
| `docs/planner_rule_api_design.md` | Future planner API doc — partial, will be the basis for the rule-API spec | full file |

## Design

### Core vs extension boundary

```
                                    ┌─────────────────────────────┐
                                    │   Core smelt                │
                                    │                             │
   model SQL  ───parse───▶   CST    │   • parser, types, graph   │
                                    │   • function expansion pass │
                                    │   • timeseries: metadata    │
                                    │   • planner rule API        │
                                    │                             │
                                    │   inlined CST + timeseries  │
                                    │     info handed to rules    │
                                    └───────────────┬─────────────┘
                                                    │
                ┌───────────────────────────────────┼────────────────────────────┐
                │                                   │                            │
                ▼                                   ▼                            ▼
   ┌─────────────────────────┐     ┌───────────────────────────┐     ┌──────────────────────┐
   │  Incremental rule       │     │  (future) MERGE rule       │     │  (future) snapshot…  │
   │                         │     │                            │     │                       │
   │ • batch-safety upgrade  │     │ • upsert physical strategy │     │ • period-over-period │
   │ • per-source bound      │     │ • unique-key reasoning     │     │   diff strategy       │
   │   derivation            │     │                            │     │                       │
   │ • source-filter         │     │                            │     │                       │
   │   pushdown              │     │                            │     │                       │
   │ • DELETE+INSERT         │     │                            │     │                       │
   │   physical strategy     │     │                            │     │                       │
   └─────────────────────────┘     └───────────────────────────┘     └──────────────────────┘
```

The rule API surface is provisional — `docs/planner_rule_api_design.md` is the starting point. A rule receives the inlined CST, the workspace's dependency graph, and `timeseries:` info for each source it references; produces either a refusal (with a diagnostic span pointing at the original SQL location) or a transformed physical plan plus the metadata other planner stages may need (e.g., per-source bounds for `--auto`'s affected-partitions analysis).

### `timeseries:` metadata (core)

The fields `event_time_column`, `partition_column`, and `granularity` are time-dimension declarations — useful for downstream consumers regardless of whether *this* model runs incrementally. They move from inside `incremental:` to a sibling `timeseries:` block. A view, a non-incremental rollup, or an external source can declare `timeseries:` and be eligible as a pushdown target without itself running incrementally.

**Model frontmatter:**

```yaml
timeseries:
  event_time_column: event_ts      # source-of-truth time column on this output
  partition_column: event_date     # column the engine prunes on (may differ)
  granularity: day                 # day | hour | week | …

incremental:                       # optional; presence opts the model into the incremental rule
  enabled: true
```

**Sources (sources.yml):**

```yaml
sources:
  - name: external_events
    schema: bronze
    timeseries:
      event_time_column: event_ts_utc
      partition_column: event_date_utc
      granularity: day
```

A source without `timeseries:` is a lookup. Pushdown rules skip it; it is read in full each partition.

Migration: existing examples re-shape from the nested `incremental: { event_time_column, partition_column, granularity, enabled }` to `timeseries: {...} + incremental: { enabled: true }`. Per project status (no backward-compat constraints), this is a one-shot rewrite — examples, docs, and tests change together in the same PR. No transitional dual-form support.

### Function expansion (core)

Pass that runs between parse/typecheck and any rule analysis. Walks the CST, replaces every `smelt.functions.foo(args)` call with the function body, with arguments substituted into the body's bindings. Produces an inlined CST.

- Recursive: a function body that itself calls another function is expanded fully.
- Source-maps: every inlined node carries a back-reference to its origin (the function's definition location and the call site). Diagnostics use this — error spans point at the call site primarily and the function definition secondarily, never at the synthetic inlined location.
- Pure: deterministic, no side effects, cacheable. Salsa already handles this kind of incremental computation; the pass plugs into the existing query graph.

Function expansion is core because expansion isn't incremental-specific. Type-checking already benefits from seeing through function bodies for diagnostics; future rules (MERGE, snapshot, audit-trail) will want the same view. The incremental rule is the first rule to consume it for analysis purposes beyond type-checking.

Note: function expansion is a *logical* pass. Physical SQL generation remains free to emit function bodies as CTEs or subqueries — that decision is the planner's. Inlining for analysis ≠ inlining for execution.

### Per-source bound abstraction (rule)

For each `(model, source_reference)` pair, the incremental rule's derivation produces:

```
BoundResult =
  | Bounded { source_partition_col: ColumnRef,
              before: Duration,
              after:  Duration }
  | Unbounded                    -- analyzable but ∞ (e.g., cumulative aggregation)
  | NotDerivable                 -- analyzer can't read this pattern
```

`source_partition_col` is the column on the source's table that the filter will be pushed onto. It must equal the source's `timeseries.partition_column`, or a column the source declares as partition-aligned.

`before` and `after` are durations in the source's partition-column unit.

| Case | Tuple |
|---|---|
| Same column, look back for windowed state | `(event_date, 1d, 0)` |
| Same column, no lookback | `(event_date, 0, 0)` |
| Bronze → user-local, tz rebase | `(event_ts_utc, 24h, 24h)` |
| Range-join across a session window | `(event_date, session_max, session_max)` |
| Cumulative-history aggregation | `(event_date, ∞, 0)` → `Unbounded` |

**Aggregation across multiple references to the same source.** Take the union: `before = max(before_i)`, `after = max(after_i)`. If any reference is `Unbounded`, the union is `Unbounded`. If any is `NotDerivable`, the union is `NotDerivable`. The pushdown filter has to be wide enough for the worst use.

**Aggregation across distinct sources.** Each source independent — no cross-source mixing.

**Lookup tables.** Sources without `timeseries:` are lookups. No bound derived; no filter pushed; read in full.

### Derivation rules (rule, v1)

V1 ships with two forms, both reading existing SQL constructs. A projection-catalog (recognising `AT TIME ZONE`, `DATE_TRUNC`, etc. as bounded operations on the source column) is deferred — see Open Questions.

**Form A — Window-frame `RANGE BETWEEN INTERVAL '…' PRECEDING/FOLLOWING`.**

```sql
LAG(event_ts) OVER (
    PARTITION BY device_id ORDER BY event_ts
    RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW
) AS prev_ts
```

The planner reads `INTERVAL '30 minutes'` from the frame. The source backing the projected column (resolved via the FROM clause) gets `before += 30 minutes`. `BETWEEN INTERVAL '…' FOLLOWING` similarly adds to `after`.

**Form B — Explicit JOIN/WHERE date filters with literal `INTERVAL` offsets.**

```sql
FROM bronze.events b
JOIN users u ON b.user_id = u.user_id
WHERE b.event_ts_utc BETWEEN m.event_date_user_local - INTERVAL '1 day'
                         AND m.event_date_user_local + INTERVAL '1 day'
```

The planner reads literal `INTERVAL` on each side of the BETWEEN and derives `(event_ts_utc, 1d, 1d)` on `bronze.events`. The same pattern handles per-source lookback (same column on both sides) and cross-column rebasing (different columns).

`BETWEEN` and pairs of `>=` / `<` with literal offsets are equivalent forms — both are read.

**What v1 doesn't read.**

- Projections through `AT TIME ZONE`, `DATE_TRUNC`, etc. (the projection catalog). Authors needing the tz-rebase case write an explicit WHERE bound (Form B); the catalog comes later.
- Bare `LAG` / `LEAD` without a RANGE clause. Time-dependent but no syntactic anchor → `NotDerivable`.
- Function-level `lookback = ...` declarations on function definitions (Form C). The function-expansion pass means Form A and Form B patterns inside function bodies are already read.

### Source-filter pushdown (rule)

For each source reference with `Bounded` result, the planner injects:

```
WHERE <source_partition_col> >= run_start - before
  AND <source_partition_col> <  run_end + after
```

onto that source's FROM clause. The outer-WHERE injection that exists today for the model's own partition column continues unchanged on the outermost SELECT — that ensures the *output* is filtered to `[run_start, run_end)` regardless of what the sources read.

Pushdown happens after function expansion so the FROMs in question include those that came from inlined function bodies. The same is true for the WHERE injection — it lands inside the inlined CST.

### Batch-safety classifier upgrade (rule)

The classifier still rejects outer-body `OVER`, `HAVING`, `LIMIT`, subqueries, `DISTINCT`, non-deterministic functions. Two changes:

- **"Outer body" is the inlined outer body.** An `OVER` that used to be hidden inside `sessionize` is now visible and classified. Closes the function-body opacity in gap #1's underlying analyzer.

- **`OVER (PARTITION BY <keys>)` admissible when `<keys>` ⊇ partition grouping.** A `FIRST_VALUE OVER (PARTITION BY device_id, session_seq ORDER BY event_ts)` is safe per-partition: every row of a session shares the same value, and the per-partition window is contained within the partition_column's grouping. The classifier now admits this directly. Closes gap #2 and removes the `compute_session_start_date.sql` workaround.

A model that fails the safety check (post-upgrade) is **refused** at planning time — no silent downgrade. The error message names the specific SQL construct and the source-map points at the original location. This closes gap #1's underlying issue. A future `--allow-downgrade` CLI flag is the escape hatch.

### Partition size ≠ run-window size (rule)

`timeseries.granularity` declares the partition granularity. The CLI passes a `--window-start D --window-end D+k·G` for any positive integer `k`. The incremental rule:

1. Validates `window-end - window-start` is a positive integer multiple of `granularity`.
2. Derives per-source bounds as described.
3. Emits one engine query for the whole window:
   - source FROMs filtered: `source.partition_col ∈ [D - before, D + k·G + after)`
   - outermost SELECT filtered: `partition_col ∈ [D, D + k·G)`
4. Backend writes: `DELETE WHERE partition_col ∈ [D, D + k·G)` followed by INSERT of the engine's result.

Per-partition equivalence still holds:
```
∀ partition p ∈ [D, D + k·G):  result_p == full_refresh().where(partition_col = p)
```
regardless of whether `p` was produced inside a 1-partition run or a `k`-partition run.

For backfill, the driver collapses contiguous ranges into one call: instead of 60× `smelt run --window 1d`, one `smelt run --window 60d` with the same source-filter widening logic. Engine scans the source once over the widened range, computes everything in one pass, the backend slices into 60 partition writes.

### Equivalence as the formal property

The contract any rule using this machinery upholds:

```
For all D such that the rule accepts the model:
  incremental_run(model, [D, D + k·G))
    .where(partition_col = p)                  ∀ p ∈ [D, D + k·G)
  ==
  full_refresh(model).where(partition_col = p)
```

Held for the **local** columns of the output — columns whose value depends only on rows visible within the model's source-filter ranges. Not held for **global** columns — those whose value depends on history beyond the source-filter ranges (gap #7's `connected_components`, `backward_fill`, etc.). Those columns force the source's lookback to `Unbounded` → the model refuses incrementality at planning time.

The local/global distinction is structural: it falls out of whether the derivation rules can prove a bound, not from a separate column-level annotation.

## Open Questions

1. **Projection catalog (deferred).** Recognise `DATE_TRUNC`, `AT TIME ZONE`, literal-`INTERVAL` arithmetic as known-bounded projections on the source column, and infer the bound from the composition. Lets the author skip an explicit WHERE bound when the projection alone implies the relationship. Worth doing once Form A + Form B are landed and the catalog can extend incrementally. Doesn't block v1.

2. **Function-level lookback declarations (deferred).** Form C in earlier discussion (`smelt.define foo(...) lookback = gap AS (...)`). With function expansion in core, Form A + Form B inside function bodies are already read, which subsumes most of what Form C provided. The remaining motivation is performance — declared lookbacks are O(1) to read, derived ones require walking the inlined CST. Re-visit if walking is measurably expensive.

3. **Affected-partitions analysis for orchestration.** The same per-source bound map fuels a forward-flow analysis: when upstream partition `S[p]` changes, downstream model `M`'s partitions `[p, p + bound.after]` are stale. This is what `--auto` and dependency-aware re-runs need. The analysis is its own scope of work; this design produces the bound map it'd consume.

4. **MERGE / cumulative materialization (gap #5).** Sibling planner rule. The shared inputs (timeseries metadata, function-expanded CST, dependency graph) are designed in this doc; the rule itself is separate work. Likely a follow-on spec/plan.

5. **Sources.yml schema for `timeseries:`.** Needs detail on validation, what happens when an external source's declared partition column doesn't exist on the underlying table, and how this composes with existing source declarations. Tractable but unaddressed here.

6. **Diagnostic UX through inlining.** Source-maps are described in principle but the LSP integration story isn't fleshed out. Where does a refused-incrementality squiggle land in VSCode when the offending `OVER` is two function-call levels deep? Probably the call chain — primary span at the model's call, secondary spans at each function boundary back to the offending node. Needs prototyping.

7. **Planner rule API surface.** `docs/planner_rule_api_design.md` is a starting point but doesn't yet specify the rule lifecycle, the metadata-handoff between rules and the orchestrator, or the diagnostic-collection mechanism. The incremental rule is the first real implementation; its API needs and the abstract API design have to be co-evolved.

## Migration / Impact

**`examples/web_analytics/`**

- Frontmatter rewrites: `incremental: { event_time_column, partition_column, granularity, enabled: true }` → `timeseries: {...} + incremental: { enabled: true }`.
- `functions/compute_session_start_date.sql` can be deleted once the safety classifier admits `OVER (PARTITION BY device_id, session_seq)`. The `FIRST_VALUE OVER` inlines back into `sessions.sql`.
- `run_incremental.py`'s 2-day-window workaround for the absent per-model lookback declaration goes away. Driver passes `[D, D+1)` or a backfill window of any size; the planner derives per-source filters.
- `silver/sessions` declares its lookback by using `RANGE BETWEEN INTERVAL '30 minutes' PRECEDING …` or an explicit WHERE bound on `events_parsed`.
- `gold/identity_forward_only` rewrites the session-window join with explicit date filters (Form B). Derived bound is `(event_date, 1d, 0)` on sessions; lookup-style on cumulative edges.

**`docs/specs/incremental_models.md`**

Refactor into "the incremental rule" spec. The pieces that aren't rule-specific (`timeseries:` declarations, function expansion behaviour) move out — likely to a new `docs/specs/timeseries.md` and `docs/specs/architecture.md` (function expansion as an architectural invariant).

**`docs/specs/architecture.md`**

New invariant: "Function expansion is a logical pass that runs before any rule analysis. Rules see the inlined CST." Sits alongside the project-isolation rule and the workspace-loading-parity rule.

**Planner rule API**

`docs/planner_rule_api_design.md` graduates from a research-flavoured design doc to a real spec, driven by the incremental rule's needs.

## References

- **Predecessor:** `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md` — the gap catalogue this design responds to.
- **Existing spec:** `docs/specs/incremental_models.md` — what the incremental rule's spec evolves from.
- **Existing spec:** `docs/specs/architecture.md` — host for the function-expansion invariant.
- **Existing research:** `docs/planner_rule_api_design.md` — basis for the rule API.
- **Patterns referenced:**
  - dbt's `is_incremental()` + `this` table — the design path smelt explicitly didn't take (pure logical SQL, no template vars). Gap #3's derived-lookback story is the alternative that keeps logical SQL.
  - SQL:2003 windowed-frame syntax (`RANGE BETWEEN INTERVAL '…' PRECEDING`) — the surface Form A reads. Standard across DuckDB / Spark / Postgres.
  - dbt incremental "merge" strategy — sibling-rule prior art for gap #5.
