# Research: Incremental-model gaps surfaced by converting `examples/web_analytics/` to day-by-day

**Date**: 2026-05-20
**Topic**: Friction points and bugs in smelt's incremental-model surface exposed by reshaping a real example pipeline (identity stitching) to run incrementally day-by-day
**Branch**: worktree-web_analytics
**Commit**: 55d7e6b4

## Summary

`examples/web_analytics/` is now end-to-end incremental: every model with a natural time dimension is partitioned by day, identity-state is split into a per-day incremental edges table plus a cumulative rollup view, and `run_incremental.py` walks the 60-day datagen window invoking `smelt run` with a 2-day lookback per iteration. The work shipped (commits c343d2 → 55d7e6 on `worktree-web_analytics`) but the design had to step around seven distinct smelt-side limitations. Two of them were unambiguous bugs and got fixed inline. Four are design gaps that drove model-shape contortions; closing them would let the example collapse roughly 100 lines of workaround SQL. One is a fundamental property of incremental + global state, not a gap to be closed.

The design direction throughout is "derive from the model, don't restate in metadata." Time-window properties (lookback, batch safety, source filters) should be readable from the SQL or function definitions, not declared in YAML frontmatter where they can drift from the logic that creates the need.

Two of the items below (#1, #6) are bugs with shipped fixes — the writeup is here for the record. The others are design gaps still open.

This doc is a starting point for follow-up specs/plans. Each item is bounded enough to be one spec; together they describe a coherent next-iteration story for incremental in smelt.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-planner/src/rules/incremental.rs` | Batch-safety classifier (rejects outer-body `OVER`, `HAVING`, `LIMIT`, etc.) | analyze_safety_checks, analyze_select |
| `crates/smelt-cli/src/transformer.rs` | `inject_time_filter` — appends WHERE to outermost SELECT only | L56-97 |
| `crates/smelt-cli/src/commands/run.rs` | Run loop; logs planner errors as `warn!` and proceeds (silent downgrade) | L470 |
| `crates/smelt-cli/src/commands/run.rs` | Interval-recording lookup (db-name vs leaf-name mismatch) | L1139 |
| `crates/smelt-cli/src/executor.rs` | `execute_model_incremental` — sets `ExecutionResult.model_name = compiled.name` (db-form) | L54-99 |
| `crates/smelt-cli/src/compiler.rs` | `compiled.name = model.db_name_owned()` (address_segments.join("_")) | L584-590 |
| `crates/smelt-cli/src/logical_graph.rs` | `LogicalGraph::get_model` — keys by leaf bare name only | L474-478 |
| `crates/smelt-backend/src/lib.rs` | `execute_model_incremental` — DELETE+INSERT is the only incremental strategy wired | L196-225 |
| `docs/specs/incremental_models.md` | Normative spec — calls out several of these as "planned but not implemented" | §Known Divergences |
| `examples/web_analytics/functions/compute_session_start_date.sql` | Transparent function that exists solely to hide `FIRST_VALUE OVER` from the safety scan | full file |
| `examples/web_analytics/models/silver/device_user_edges.sql` + `device_user_edges_cumulative.sql` | Two-model split that exists solely because there's no MERGE/cumulative materialization | full files |
| `examples/web_analytics/run_incremental.py` | Driver loop with 2-day window per iteration to fake a per-model lookback that the planner should derive from the SQL | full file |
| `examples/web_analytics/README.md` § "Day-by-day is not equivalent to a full rebuild on the global identity columns" | Documents the as-of-day-D divergence the design can't avoid | — |

## Architecture & Data Flow

Today the surface looks like:

```
model frontmatter:
  incremental:
    enabled: true
    event_time_column: <SOURCE column>     ← used to filter the FROM
    partition_column: <OUTPUT column>      ← used to DELETE the existing partition
    granularity: day|hour|week|...

CLI: smelt run --event-time-start D --event-time-end D+1
       │
       ▼
planner.analyze_batch_safety  ← walks the OUTER body only; rejects OVER/HAVING/LIMIT/...
       │
       ▼  (if rejected: warn! and run as full-refresh)
transformer.inject_time_filter ← appends `AND (event_time_column >= D AND … < D+1)` to outermost WHERE
       │
       ▼
backend.execute_model_incremental ← DELETE WHERE partition_column ∈ [D, D+1)  THEN  INSERT
```

Several deliberate non-features fall out of this shape:
- The injected WHERE never reaches CTEs, subqueries, or `smelt.functions.*` bodies.
- Models cannot declare a lookback or look-forward window; the CLI window is the only knob.
- There is no MERGE / cumulative-state strategy; only DELETE+INSERT-per-partition.
- Function bodies are opaque to the batch-safety classifier — a `LAG` inside `smelt.functions.sessionize` is invisible to the scan.

The web_analytics conversion bumped into each of these.

## Current Behavior

### What works

- **DELETE+INSERT per-partition is robust and fast.** 60-day day-by-day replay of web_analytics at scale-factor 0.01 finishes in ~13s (~0.21s/day) on a laptop.
- **`event_time_column` ≠ `partition_column` is supported.** The `timeseries` example uses `event_time_column: event_timestamp` (source) + `partition_column: event_date` (output alias) without issue, and web_analytics uses both forms (matching for events_parsed/edges, distinct for sessions where the partition is a derived column).
- **Idempotency on overlapping windows is correct.** The 2-day driver window re-processes the prior day every iteration; the DELETE+INSERT semantics make this a no-op for unchanged inputs and a clean refresh when something changed.
- **`smelt explain --json` exposes `batch_safety` per incremental model.** This is the hook the new `web_analytics_incremental_models_classify_as_safe` regression test uses to gate against silent downgrades.

### Gaps

#### 1. Silent safety-check downgrade — **bug; fixed at example level, root cause remains**

The planner's batch-safety classifier rejects outer-body `OVER`, `HAVING`, subqueries, `LIMIT`, non-deterministic functions, and `DISTINCT`. When a model fails the check, `commands/run.rs:470` logs the rejection as `warn!` and the model runs as a full-refresh table. `silver/sessions` declared `incremental: enabled` for months but had `FIRST_VALUE OVER` in its outer body and was being silently downgraded the entire time; nobody noticed because the model still produced correct rows.

**Fix shipped:** moved the `FIRST_VALUE OVER` into a transparent function (`compute_session_start_date.sql`) so the outer body has no `OVER`, and added `crates/smelt-cli/tests/web_analytics_incremental_classification.rs` which asserts every model declaring incremental in the example classifies as `fully_batch_safe` via `smelt explain --json`.

**Underlying gap not fixed:** the CLI still degrades silently for any other repo that hits a safety rejection. Two follow-up options:
- (a) Promote the rejection from `warn!` to an error by default; add an explicit `--allow-downgrade` flag for the rare case where it's wanted.
- (b) Surface the classification on `smelt run` output (one line per incremental model) so a downgrade is visible without `--verbose` and without `smelt explain`.

The CI gate against the example only catches future regressions in this one workspace; option (a) or (b) is needed to gate all repos.

#### 2. Outer-body `OVER` rejection is too coarse

The safety check is a substring scan plus an AST check for `OVER` tokens in the outer body. It does not analyse the window's `PARTITION BY` keys. But `FIRST_VALUE OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS session_start_date` is *safe* for per-partition execution: every row of a session shares the same `session_start_date`, so the per-partition window is fully contained within the partition_column's grouping.

In the web_analytics example we wrap that exact expression in `compute_session_start_date.sql` purely to dodge the scan. The function body has zero algorithmic value beyond "hide the OVER from the analyser" — every callsite expands it identically. A smarter classifier could prove that `OVER (PARTITION BY <keys>)` is safe when `<keys>` is a superset of the partition_column's grouping, and admit it directly in the outer body.

This would let the example delete `compute_session_start_date.sql` and inline the projection back into `sessions.sql`.

#### 3. Lookback (and batch-safety) should be derived from the model, not declared in metadata

`gold/identity_forward_only` needs a 1-day lookback to catch the case "session started on D-1, got a late signin on D". `silver/sessions` needs the same for cross-midnight sessions. Today there is no way to express either at the model level; the only knob is the CLI window. The web_analytics driver works around this by always passing `[D-1, D+1)`, which is correct for the two models that need lookback and wasteful for the four others that don't.

The instinctive fix is a YAML annotation (`lookback_days: 1`), but that's the wrong shape — it puts the time-dependency declaration in metadata, separated from the SQL or function logic that creates the need. Declaration and logic can drift; an author can change a function's time window without remembering to update the YAML.

The right shape is to **derive lookback (and the related batch-safety property) from the model itself**. Three forms, in order of analyser difficulty:

**Form A — explicit time-bounded windows.** The SQL names its lookback directly via a `RANGE BETWEEN INTERVAL '...' PRECEDING` clause:

```sql
LAG(event_ts) OVER (
    PARTITION BY device_id ORDER BY event_ts
    RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW
) AS prev_ts
```

The planner reads the `INTERVAL '30 minutes'` directly. Lookback for daily granularity = ceil(30 minutes / 1 day) = 1 day. Straightforward implementation; this is the "lowest-friction" form.

**Form B — explicit time-bounded join filters.** A range join's lookback is whatever the date-filter says:

```sql
FROM smelt.silver.sessions s
JOIN smelt.silver.events_parsed e
    ON e.device_id = s.device_id
   AND e.event_ts BETWEEN s.session_start AND s.session_end
WHERE e.event_date
    BETWEEN s.session_start_date - INTERVAL '1 day'
        AND s.session_start_date + INTERVAL '1 day'
```

The planner reads the WHERE constant and derives the lookback. Also reasonably tractable.

**Form C — function-level `lookback` declaration that resolves at the callsite.** Transparent functions whose body has time-dependent logic expose it in their signature. `sessionize`'s `gap` parameter *is* its lookback:

```sql
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<Integer>,
    ts_col: Expr<Date>,
    platform_col: Expr<Text>,
    gap: Expr<BigInt> = 30 * 60 * 1000000
) -> TableExpr
    lookback = gap                       -- new: function declares its time-dependency
AS ( ... )
```

At each callsite, the planner resolves `gap` to its bound expression. `sessions.sql` calling `sessionize(gap => 30 minutes)` yields "this model needs ≥30 minutes of source lookback" → 1 day at daily granularity. The declaration lives with the function definition, where the time-dependent logic lives, so it can't drift.

**This subsumes today's "no batch-safety analysis through function bodies" gap.** The same machinery — the planner reading bounded-time declarations from function definitions — gives it the information to classify `sessionize`-using models as bounded-safe rather than opaque. Form A and Form B's analysis are immediate; Form C just generalises the same reading to function signatures.

**Composition.** Each layer's lookback propagates up. `sessions` calls `sessionize` (declares 30min). `identity_forward_only` joins to `sessions` (declares the session-window range join). `eventstream_with_identity` joins to both. The planner walks the dependency graph and accumulates the maximum lookback per model, then expands each model's run window automatically.

**Limitation.** Models with implicit time logic and no syntactic anchor — a bare `LAG` with no RANGE clause, a join on a computed expression with no date filter — can't be analysed. The planner refuses incrementality on those models, and the author must rewrite using one of the three forms above. Arguably the right outcome: a model the planner can't analyse for lookback is also a model the planner can't reason about for correctness.

**Migration path for the web_analytics example.** `sessionize` gets the `lookback = gap` declaration (Form C). `identity_forward_only`'s session-window join is rewritten to add the `WHERE event_date BETWEEN session_start_date - INTERVAL '1 day' AND session_start_date + INTERVAL '1 day'` (Form B). After both changes, the driver script can drop the 2-day window and just pass `[D, D+1)` — the planner expands each model's window per its derived lookback.

#### 4. No source-filter pushdown

Once the planner can derive a model's lookback (gap #3), the next step is to push the inferred filter onto the model's sources. Today smelt only injects WHERE on the outermost SELECT, never inside CTEs or function bodies — so `silver/sessions` reads the full `events_parsed` table every partition, even though it only needs `[D - 1 day, D + 1 day)`.

With gap #3 closed, the planner knows the lookback. The mechanical step that remains is to push the corresponding range filter onto each source FROM clause:

```
sessions.sql derived lookback: 1 day
sessions.sql's FROM is smelt.silver.events_parsed (partitioned by event_date)
                              ↓
planner injects: AND event_date BETWEEN partition_date - 1 AND partition_date + 1
                 on the events_parsed reference, BEFORE sessionize sees it
```

This requires resolving "partition_date" symbolically to the run-window — which the planner already does for the outer WHERE injection. The new mechanism is just pushing the same expression onto inner FROM references.

This is called out in `docs/specs/incremental_models.md` § "Known Divergences" as planned-but-not-wired. The minimal version doesn't need full algebraic equivalence — it needs the lookback from #3 and a way to recognise time-partition columns on source tables (which is already present in their incremental frontmatter).

#### 5. No cumulative / MERGE materialization

`silver/device_user_edges` stores `(device_id, user_id, event_date, daily_event_count, daily_first_seen, daily_last_seen)`. To produce the cumulative `(device_id, user_id, event_count, first_seen, last_seen)` shape the identity algorithms need, we added `silver/device_user_edges_cumulative.sql` as a view that aggregates across all dates.

The two-model split is a workaround. If smelt had a per-key incremental strategy — MERGE on `unique_key: [device_id, user_id]`, with the SQL emitting one row per pair containing cumulative counts — `device_user_edges` could be a single cumulative incremental table:

```yaml
incremental:
  enabled: true
  event_time_column: event_date
  partition_column: event_date
  granularity: day
  strategy: merge
  unique_key: [device_id, user_id]
  merge_columns:
    event_count: { aggregate: sum }
    first_seen:  { aggregate: min }
    last_seen:   { aggregate: max }
```

The backend would translate this to an upsert: for each new (device_id, user_id, event_date) row from the daily-aggregated source, update the existing cumulative row's counters or insert a new row.

The current `IncrementalStrategy::Merge` enum variant exists (`crates/smelt-backend/src/lib.rs:214`) and the `MergeInto` backend method has a signature, but only DELETE+INSERT is fully wired. Closing this gap removes the cumulative-view pattern from the example.

#### 6. Nested-incremental model lookup mismatch — **bug; fixed**

`ExecutionResult.model_name` is set to `compiled.name = model.db_name_owned()`, which joins address segments with `_` (`silver_events_parsed`). But `LogicalGraph::get_model` keys by the leaf bare name only (`events_parsed`). The interval-recording lookup at `commands/run.rs:1139` does `graph.get_model(&result.model_name)?` and dies with "Model not found: silver_events_parsed" for any incremental model in a subdirectory.

This bug was hidden until this example because the only nested incremental model in the repo was `silver/sessions`, which was being silently downgraded to full-rebuild (gap #1) and so never hit the interval-recording path. The first incremental model in a subdir that actually classifies as fully_batch_safe surfaces the bug.

**Fix shipped:** added a db_name fallback at the call site. The proper fix is in `LogicalGraph::get_model` itself — either accept both forms, or expose a `get_model_by_db_name` API and use the right one at each call site. The current fallback works but is a wart at the wrong layer.

#### 7. As-of-day-D divergence with global identity — **design property, not a bug**

`backward_fill` and `connected_components` are *global* identity algorithms — their per-device output depends on the cumulative `(device, user)` edge set across all dates. The day-by-day pipeline writes `gold/eventstream_with_identity` for day D using the cumulative edges visible at the time of day D's run. A subsequent day D+1 may add edges that would have changed day D's mapping, but the day D rows are not retroactively rewritten under DELETE+INSERT-per-partition.

A full-window single-shot rebuild writes everything at once using the final cumulative identity. So the two pipelines necessarily diverge on `dau_backward_fill`, `dau_connected_components`, and the corresponding `identified_events_*` columns. The local columns (`raw`, `forward_only`) agree exactly.

This is *correct* incremental semantics — the same as-of-day-D property real streaming pipelines have. But it's a fundamental property the user needs to know about, because the intuitive expectation is "the marts should match a full rebuild." For 60 days at scale-factor 0.01, 45/60–48/60 days differ on the global columns; only the days where no later edge changes any device's election agree exactly.

There is no fix-by-feature here. The example's README now calls this out explicitly with a per-column equality table, and `verify_incremental_equivalence.py` asserts the local-column equality and prints the global-column divergence summary as a sanity check. The framework-level improvement worth considering: a built-in `as_of_partition_date` annotation that documents this expectation on a model, so a reader of the SQL knows up front that the column reflects state at the time of the partition's run rather than current cumulative state. Optional, low priority.

## Related Patterns

- **dbt's `is_incremental()` macro + `--full-refresh` flag** addresses lookback and source-filter pushdown by giving the model access to a `this` table and run-window variables, letting authors write incremental logic explicitly with template variables. smelt's design choice (pure logical SQL, no template vars) is deliberate per `docs/specs/incremental_models.md` § Design; the trade-off is that the framework has to be smarter (derive lookback from SQL/function bodies) to compensate. Gap #3 above is the alternative path that keeps the logical-SQL constraint.

- **dbt incremental "merge" strategy** is the upstream of #5. dbt's `unique_key + merge` model is widely used and the prior art is well-trodden; the path to wiring smelt's `IncrementalStrategy::Merge` end-to-end is mostly mechanical.

- **The `--auto` gap-detection mode** mentioned in the spec (`--auto`: process only gaps since last run) intersects with #3 — it requires the framework to know each model's "what range is fresh enough to be considered done," which falls naturally out of derived-lookback analysis.

- **SQL:2003 windowed-frame syntax** (`RANGE BETWEEN INTERVAL '...' PRECEDING`) is exactly the surface gap #3 Form A leverages. It's a standard SQL feature and DuckDB / Spark / Postgres all support it; smelt's planner just needs to read it.

## Test Coverage

- **`crates/smelt-cli/tests/web_analytics_incremental_classification.rs`** — new regression gate for #1; asserts every incremental model in the example classifies as `fully_batch_safe`.
- **`examples/web_analytics/tests/device_user_edges_per_day_invariants.test.sql`** — new inline test for the per-day edges shape (#5 workaround correctness).
- **`examples/web_analytics/verify_incremental_equivalence.py`** — runs both pipelines, asserts local-column equivalence, reports global-column divergence (#7 documentation).
- **`crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs`** — pre-existing snapshot tests; still pass against the refactored shape.
- **Smelt's existing incremental tests** (`crates/smelt-cli/tests/incremental/`) — pass; the run.rs interval-lookup fix from #6 keeps them green.

## Open Questions

1. **Priorities.** Items #1, #3, #4, #5 all return real value. Closing #3 (derive lookback from the model) is the biggest single architectural lift but it also unlocks #4 (source-filter pushdown) for free and subsumes the old "function-body batch-safety analysis" gap. Closing #5 (MERGE) removes the most lines from the example with smaller scope. Closing #1 prevents the next "silently shipped as full-refresh" bug. Likely landing order: #1 (trivial), then #5 (mechanical), then #3 (the architectural one) with #4 as its natural follow-on, then #2 (independent and small).

2. **For #3, which form lands first — A, B, or C?** Form A (RANGE BETWEEN INTERVAL clauses) is the lowest-friction and the most standard SQL. Form C (function-level `lookback` declaration) is required for the sessionize pattern, which is exactly the pattern that exposed the gap. Form B (range joins with date filters) is in between. Suggested order: A first (smallest planner change), then C (extends the same analysis to function signatures), then B.

3. **For #3 Form C, what's the surface for declaring lookback in a function?** `lookback = gap` as a post-signature clause looks reasonable but isn't strongly typed today — `gap` is `Expr<BigInt>` (microseconds) so the planner would need to recognise the unit. Alternatives: require the lookback parameter to be typed as `Duration` or `Interval`; or make it a separate annotation `@lookback duration_us = gap`.

4. **For #2, does the safety classifier need to be smarter or do we just hide more inside transparent functions?** With #3 Form C in place, function bodies become analysable. At that point #2 (admitting safe `OVER (PARTITION BY ...)` directly in the outer body) is less urgent — authors can put the OVER inside a function with a declared safety property. The middle ground: keep the rule that the outer body is conservative, but make the function-level declarations the place where safety/lookback is established once.

5. **For #4, does pushdown need to know the source's partition column?** Yes — and the source's own incremental frontmatter already declares it. The planner reads `event_time_column` / `partition_column` from each source model's frontmatter to know which column to filter on. No new metadata required.

6. **For #7, is there value in a `as_of_partition_date` decorative annotation, or does the README explanation cover it?** The annotation only adds value if multiple examples end up needing it.
