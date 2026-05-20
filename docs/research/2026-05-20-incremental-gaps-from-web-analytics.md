# Research: Incremental-model gaps surfaced by converting `examples/web_analytics/` to day-by-day

**Date**: 2026-05-20
**Topic**: Friction points and bugs in smelt's incremental-model surface exposed by reshaping a real example pipeline (identity stitching) to run incrementally day-by-day
**Branch**: worktree-web_analytics
**Commit**: 55d7e6b4

## Summary

`examples/web_analytics/` is now end-to-end incremental: every model with a natural time dimension is partitioned by day, identity-state is split into a per-day incremental edges table plus a cumulative rollup view, and `run_incremental.py` walks the 60-day datagen window invoking `smelt run` with a 2-day lookback per iteration. The work shipped (commits c343d2 → 55d7e6 on `worktree-web_analytics`) but the design had to step around eight distinct smelt-side limitations. Two of them were unambiguous bugs and got fixed inline. Six are design gaps that drove model-shape contortions; closing them would let the example collapse roughly 100 lines of workaround SQL.

Two of the items below (#1, #7) are bugs with shipped fixes — the writeup is here for the record. The others are design gaps still open.

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
| `examples/web_analytics/run_incremental.py` | Driver loop with 2-day window per iteration to fake a per-model `lookback_days: 1` | full file |
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

#### 3. No model-level `lookback_days` annotation

`gold/identity_forward_only` needs a 1-day lookback to catch the case "session started on D-1, got a late signin on D". The session table itself needs the same lookback for cross-midnight sessions. Today there is no per-model annotation for this; the only way to express lookback is at the CLI invocation, by widening `--event-time-start`.

The web_analytics driver script always passes `[D-1, D+1)` per iteration. That's correct for the two models that need lookback but wasteful for the four others — `events_parsed`, `device_user_edges`, `eventstream_with_identity`, `daily_active_users_by_method` re-process the prior day's partition every iteration even though their inputs haven't changed. With 60 days that's 60 redundant DELETE+INSERTs per affected model; idempotent and cheap individually but the principle is wrong: the driver shouldn't be the source of truth for how far back each model needs to see.

Proposed surface:
```yaml
incremental:
  enabled: true
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
  lookback_days: 1     # new — automatically expand the run window for THIS model
```
The CLI window `[D, D+1)` would be expanded per model to `[D - lookback_days, D + 1)` for the WHERE injection and the partition DELETE. Models without `lookback_days` keep today's behaviour.

#### 4. No source-filter pushdown

Even with `lookback_days`, `silver/sessions` would still pay O(all events) compute per partition: smelt only injects WHERE on the outermost SELECT, never inside CTEs or function bodies. `sessionize` is a transparent function with `LAG OVER (PARTITION BY device ORDER BY ts)` in its body, and that LAG runs over the full `events_parsed` table every iteration before the outer WHERE filters down to today's session_start_dates.

This is called out in `docs/specs/incremental_models.md` § "Known Divergences" as planned but not wired. The shape we'd want:

```
sessions.sql declares lookback_days: 1
sessions.sql's FROM is smelt.silver.events_parsed
smelt analyses sessions and sees `event_time_column: session_start_date` is derived from `event_ts` via FIRST_VALUE
smelt cannot prove a tight equivalence, falls back to filtering `events_parsed` by `event_date` with the SAME ± lookback_days window
```

The minimal version doesn't need full algebraic equivalence — just a way to declare on the model "when filtering, also filter source `smelt.silver.events_parsed` on column `event_date` with the same window." A simple `source_filters:` annotation listing `(source_path, column, expand_by)` would do it:

```yaml
incremental:
  source_filters:
    - source: smelt.silver.events_parsed
      column: event_date
      lookback_days: 1
```

This is uglier than implicit pushdown but it's small and unambiguous; implicit pushdown can come later when the analyser is smarter.

#### 5. No batch-safety analysis through function bodies

This is the upstream cause of #2 and the "OVER inside sessionize is invisible to the analyser" property in #4. The planner's safety scan walks only the outer body. A transparent function call like `smelt.functions.sessionize(...)` is treated as opaque — its `LAG OVER` is silently invisible.

This is documented as "current state" in the spec (`docs/specs/functions.md` § "Batch-safety classification through call sites (current state)"). The asymmetry has two effects:
- (helpful) we can hide expensive but safe expressions in a function to bypass the outer-body check (this is how `compute_session_start_date.sql` works today).
- (harmful) we can hide expressions that genuinely break per-partition execution and the analyser won't catch it. Today's example doesn't do this, but future examples could.

The clean fix is to make the safety classifier recursively walk transparent function bodies — but that requires resolving function calls during planning, which couples the planner to the function resolver. Worth a spec to scope properly.

#### 6. No cumulative / MERGE materialization

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

#### 7. Nested-incremental model lookup mismatch — **bug; fixed**

`ExecutionResult.model_name` is set to `compiled.name = model.db_name_owned()`, which joins address segments with `_` (`silver_events_parsed`). But `LogicalGraph::get_model` keys by the leaf bare name only (`events_parsed`). The interval-recording lookup at `commands/run.rs:1139` does `graph.get_model(&result.model_name)?` and dies with "Model not found: silver_events_parsed" for any incremental model in a subdirectory.

This bug was hidden until this example because the only nested incremental model in the repo was `silver/sessions`, which was being silently downgraded to full-rebuild (gap #1) and so never hit the interval-recording path. The first incremental model in a subdir that actually classifies as fully_batch_safe surfaces the bug.

**Fix shipped:** added a db_name fallback at the call site. The proper fix is in `LogicalGraph::get_model` itself — either accept both forms, or expose a `get_model_by_db_name` API and use the right one at each call site. The current fallback works but is a wart at the wrong layer.

#### 8. As-of-day-D divergence with global identity — **design property, not a bug**

`backward_fill` and `connected_components` are *global* identity algorithms — their per-device output depends on the cumulative `(device, user)` edge set across all dates. The day-by-day pipeline writes `gold/eventstream_with_identity` for day D using the cumulative edges visible at the time of day D's run. A subsequent day D+1 may add edges that would have changed day D's mapping, but the day D rows are not retroactively rewritten under DELETE+INSERT-per-partition.

A full-window single-shot rebuild writes everything at once using the final cumulative identity. So the two pipelines necessarily diverge on `dau_backward_fill`, `dau_connected_components`, and the corresponding `identified_events_*` columns. The local columns (`raw`, `forward_only`) agree exactly.

This is *correct* incremental semantics — the same as-of-day-D property real streaming pipelines have. But it's a fundamental property the user needs to know about, because the intuitive expectation is "the marts should match a full rebuild." For 60 days at scale-factor 0.01, 45/60–48/60 days differ on the global columns; only the days where no later edge changes any device's election agree exactly.

There is no fix-by-feature here. The example's README now calls this out explicitly with a per-column equality table, and `verify_incremental_equivalence.py` asserts the local-column equality and prints the global-column divergence summary as a sanity check. The framework-level improvement worth considering: a built-in `as_of_partition_date` annotation that documents this expectation on a model, so a reader of the SQL knows up front that the column reflects state at the time of the partition's run rather than current cumulative state. Optional, low priority.

## Related Patterns

- **dbt's `is_incremental()` macro + `--full-refresh` flag** addresses #3 and #4 by giving the model access to a `this` table and run-window variables, letting authors write incremental logic explicitly. smelt's design choice (pure logical SQL, no template vars) is deliberate per `docs/specs/incremental_models.md` § Design; the trade-off is that the framework has to be smarter (lookback annotations, source-filter pushdown) to compensate.

- **dbt incremental "merge" strategy** is the upstream of #6. dbt's `unique_key + merge` model is widely used and the prior art is well-trodden; the path to wiring smelt's `IncrementalStrategy::Merge` end-to-end is mostly mechanical.

- **The `--auto` gap-detection mode** mentioned in the spec (`--auto`: process only gaps since last run) intersects with #3 — it requires the framework to know each model's "what range is fresh enough to be considered done," which is naturally expressed as `lookback_days`.

## Test Coverage

- **`crates/smelt-cli/tests/web_analytics_incremental_classification.rs`** — new regression gate for #1; asserts every incremental model in the example classifies as `fully_batch_safe`.
- **`examples/web_analytics/tests/device_user_edges_per_day_invariants.test.sql`** — new inline test for the per-day edges shape (#6 workaround correctness).
- **`examples/web_analytics/verify_incremental_equivalence.py`** — runs both pipelines, asserts local-column equivalence, reports global-column divergence (#8 documentation).
- **`crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs`** — pre-existing snapshot tests; still pass against the refactored shape.
- **Smelt's existing incremental tests** (`crates/smelt-cli/tests/incremental/`) — pass; the run.rs interval-lookup fix from #7 keeps them green.

## Open Questions

1. **Priorities.** Items #1, #3, #4, #6 all return real value. Closing #6 (MERGE) removes the most lines from the example. Closing #1 prevents the next "silently shipped as full-refresh" bug. Which lands first depends on how much the next user-facing feature push needs MERGE vs. how much we trust the safety-check warning is good enough.

2. **Does the safety classifier need to be smarter (#2/#5) or do we just hide more inside transparent functions?** The latter is easy and works; the former is the architecturally clean answer. A middle ground: keep the rule that transparent-function bodies are opaque, but introduce a `safety_overrides:` block at the function-definition level so authors can declare "this function's body is safe for per-partition execution" once, instead of every call site.

3. **Is `lookback_days` (#3) the right knob, or do we want `lookback_partitions`?** Granularity-day is the common case, but week/quarter granularities exist; the latter generalises.

4. **For #4, does the inline `source_filters:` declaration scale?** With one source per model it's fine; with many sources it gets repetitive. Worth checking dbt's `+pre_hook` pattern as prior art.

5. **For #8, is there value in a `as_of_partition_date` decorative annotation, or does the README explanation cover it?** The annotation only adds value if multiple examples end up needing it.
