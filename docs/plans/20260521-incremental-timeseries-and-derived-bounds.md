# Plan: timeseries: frontmatter + SQL-derived per-source bounds

**Date**: 2026-05-21
**Spec**: [`docs/specs/incremental_models.md`](../specs/incremental_models.md), [`docs/specs/timeseries.md`](../specs/timeseries.md)
**Spec diff**: `c7847e6a..81e9b3af` (commit `docs(specs): factor timeseries: out of incremental:, spec SQL-derived bounds`)
**Tracking PR / branch**: `worktree-web_analytics` → PR to be opened on plan adoption
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/incremental_models.md` and `docs/specs/timeseries.md` — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (e.g., `type_inference.rs` purity, project-isolation rule, workspace-loading-parity rule).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/<slug>.md` and `docs-site/docs/...` describe the feature as if it has always existed.

---

## Context

The spec diff factors the time-dimension declaration out of `incremental:` into a new `timeseries:` block (`timeseries.md` §Surface), introduces per-source bound derivation from SQL (`incremental_models.md` §Semantics → "Per-source bound derivation"), adds source-filter pushdown (§"Source-filter pushdown"), replaces silent downgrade with refused-incrementality (Constraint 10), upgrades the safety classifier to admit partition-aligned `OVER` (§"Safety checks (rejected by default)" + Known Divergences), decouples run-window size from partition granularity (§"Run window vs partition granularity"), and formalises per-partition equivalence with full refresh (§"Per-partition equivalence"). The motivating gaps are in `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md`; the design direction is in `docs/research/20260521-incremental-as-planner-rule.md`.

## Scope

### In scope (spec coverage)

- `timeseries.md` §Surface — frontmatter block on models and external sources, including `MalformedTimeseries` and `TimeseriesRequiredForIncremental` diagnostics.
- `incremental_models.md` §Surface — `incremental:` no longer carries `event_time_column` / `partition_column` / `granularity`.
- `incremental_models.md` §Semantics → "Per-source bound derivation" (Form A + Form B reading on the expanded CST).
- `incremental_models.md` §Semantics → "Batch safety classification" (admits partition-aligned `OVER`, refuses on `NotDerivable`).
- `incremental_models.md` §Semantics → "Source-filter pushdown".
- `incremental_models.md` §Semantics → "Run window vs partition granularity".
- `incremental_models.md` §Semantics → "Per-partition equivalence" — as a verifiable contract via a test harness.
- `incremental_models.md` Constraint 10 — no silent downgrade.

### Explicitly deferred

- **Projection catalog** (`AT TIME ZONE`, `DATE_TRUNC`, literal-`INTERVAL` arithmetic on projections). Reserved per `incremental_models.md` Known Divergences; authors needing tz-rebase use Form B explicit WHERE bounds.
- **MERGE / cumulative materialization strategy** (`incremental_models.md` Known Divergences). Sibling planner rule.
- **`--auto` gap-detection + affected-partitions analysis for orchestration**. Consumes the bound map shipped here; orchestration UX is its own scope.
- **Per-column `data_latency` annotation** (`incremental_models.md` Known Divergences). Independent line of work.
- **External source verification against the live database**.
- **LSP enrichment for `timeseries:` hover / goto-definition** (`timeseries.md` Known Divergences). Tracked alongside this plan's user-doc work but lands separately if it slips.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1 — `timeseries:` frontmatter migration                  | done     | c942f1e4 | 2026-05-21 |
| 2 — Refused-incrementality (no silent downgrade)         | done     |        | 2026-05-21 |
| 3 — Classifier admits partition-aligned `OVER`           | pending  |        |      |
| 4 — Per-source bound derivation (Form A + Form B)        | pending  |        |      |
| 5 — Source-filter pushdown on expanded CST               | pending  |        |      |
| 6 — Run-window-vs-partition decoupling                   | pending  |        |      |
| 7 — Per-partition equivalence harness                    | pending  |        |      |
| 8 — `examples/web_analytics/` simplification             | pending  |        |      |

---

### Phase 1: `timeseries:` frontmatter migration

**Goal.** Move `event_time_column`, `partition_column`, `granularity` (and `week_start`) out of `incremental:` into a sibling `timeseries:` frontmatter block; emit `TimeseriesRequiredForIncremental` when `incremental:` appears without `timeseries:`; emit `MalformedTimeseries` for structural violations.

**Pre-conditions.** None — entry point.

**TDD tests to write first.**
- `crates/smelt-core/src/metadata.rs::tests::test_timeseries_block_parses` — frontmatter with a `timeseries:` block parses to a `TimeseriesConfig` carrying the four fields.
- `crates/smelt-core/src/metadata.rs::tests::test_incremental_without_timeseries_errors` — a `.sql` file declaring `incremental:` with no `timeseries:` produces `TimeseriesRequiredForIncremental` at metadata extraction.
- `crates/smelt-core/src/metadata.rs::tests::test_legacy_nested_form_errors` — a `.sql` file declaring `event_time_column` *inside* `incremental:` produces `MalformedTimeseries` with a message pointing at the new location.
- `crates/smelt-core/src/metadata.rs::tests::test_timeseries_on_ephemeral_errors` — `materialization: ephemeral` + `timeseries:` is `MalformedTimeseries` (per `timeseries.md` §Semantics).
- `crates/smelt-core/src/metadata.rs::tests::test_timeseries_partition_column_must_project` — `partition_column` absent from the model's output is `MalformedTimeseries`.
- `crates/smelt-cli/tests/example_diagnostics.rs::test_examples_load_clean` (existing test) — must still pass after the example workspaces are migrated to the new shape.

**Implementation shape.**
- Add `TimeseriesConfig { event_time_column, partition_column, granularity, week_start }` in `smelt-core/src/config.rs` alongside the slimmed `IncrementalConfig { enabled, unique_key, safety_overrides }`. `Granularity` and `Weekday` already exist; reuse.
- Update `ModelMetadata` (`smelt-core/src/metadata.rs`) to parse `timeseries:` at the top level of the frontmatter; emit `TimeseriesRequiredForIncremental` and `MalformedTimeseries` diagnostics through the existing diagnostic channels.
- Rewrite every example workspace's frontmatter to the new shape in the same commit (`examples/timeseries/`, `examples/retail_analytics/`, `examples/web_analytics/`).
- Update `crates/smelt-core/tests/example_workspaces.rs` and the metadata snapshot tests for the new shape.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs` — `TimeseriesConfig`, slimmed `IncrementalConfig`.
- `crates/smelt-core/src/metadata.rs` — frontmatter extraction, new diagnostics.
- `examples/timeseries/`, `examples/retail_analytics/`, `examples/web_analytics/` — frontmatter rewrites.
- `crates/smelt-cli/src/main.rs` — if it consumes the nested fields, redirect to the new struct shape.

**Docs touched.**
- `docs/specs/timeseries.md` — already authoritative; no further surface changes.
- `docs/specs/incremental_models.md` — already authoritative.
- `docs-site/docs/guide/incremental-models.md` — update YAML examples to the new shape; describe `timeseries:` as the time-dimension home.
- `docs-site/docs/reference/timeseries.md` — new reference page mirroring `timeseries.md` §Surface for users.

**Review checklist.**
- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `TimeseriesRequiredForIncremental` and `MalformedTimeseries` codes are owned by `timeseries.md` and surface through the same diagnostic channel as other frontmatter errors.
- [ ] Every example workspace migrated; no `event_time_column:` keys remain nested inside `incremental:`.
- [ ] User docs read as feature description, not changelog.
- [ ] Spec + docs-site edits are timeless — no `Phase 1` headings, no `(Phase 1)` labels.

**Commit.** `feat(core): timeseries: frontmatter block, factored out of incremental:`

---

### Phase 2: Refused-incrementality (no silent downgrade)

**Goal.** Promote the safety-check rejection from `warn!` to a hard error. A model whose SQL fails the safety classifier or whose bound derivation produces `NotDerivable` (when bound derivation lands in Phase 4) is refused at planning time.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/incremental_refusal.rs::test_outer_over_refused` — a real-fixture model with `OVER` in the outer body (no `PARTITION BY` admissibility) errors out at `smelt run`; the diagnostic names the construct and points at the SQL.
- `crates/smelt-cli/tests/incremental_refusal.rs::test_outer_having_refused` — same for `HAVING`.
- `crates/smelt-cli/tests/incremental_refusal.rs::test_refusal_exit_nonzero` — `smelt run` exits non-zero on refusal.
- `crates/smelt-cli/tests/incremental_refusal.rs::test_no_full_refresh_fallback` — the output table is *not* populated when a refusal fires (the previous silent downgrade would have inserted full-table rows).
- `crates/smelt-cli/tests/web_analytics_incremental_classification.rs::test_all_incremental_models_classify` (existing) — still passes; no model in the example regresses to refused.

**Implementation shape.**
- Locate the `warn!` path in `crates/smelt-cli/src/commands/run.rs` (around the existing safety-check rejection logging). Replace with returning an error diagnostic via the existing diagnostic plumbing.
- Surface the diagnostic in `smelt run` output (one line per refused model, with span and reason).
- Add an `--allow-downgrade` CLI flag as an explicit opt-in for the rare case where full-refresh is wanted; documented in `cli.md` as escape hatch.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/commands/run.rs` — warn → error promotion, `--allow-downgrade` flag.
- `crates/smelt-cli/src/main.rs` — flag plumbing.
- `crates/smelt-planner/src/rules/incremental.rs` — diagnostic structure if needed.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences entry for "Refused-incrementality replaces silent downgrade" closes; Constraint 10 is now reality.
- `docs/specs/cli.md` — `--allow-downgrade` flag.
- `docs-site/docs/guide/incremental-models.md` — author section on what to do when refused; mention `--allow-downgrade` as the escape hatch.

**Review checklist.**
- [ ] TDD tests listed above exist and pass.
- [ ] No `warn!` path remains for safety rejections — only hard errors with diagnostics.
- [ ] `--allow-downgrade` is opt-in; default behaviour is refusal.
- [ ] Spec Known Divergences entry removed; Constraint 10 reflects implementation.
- [ ] User docs explain refusal and the escape hatch.

**Commit.** `feat(cli): refuse incremental models on safety rejection, no silent downgrade`

---

### Phase 3: Classifier admits partition-aligned `OVER`

**Goal.** Upgrade the batch-safety classifier to admit `OVER (PARTITION BY <keys>)` in the outer (expanded) body when `<keys>` is a superset of the model's partition grouping. Removes the need to hide `FIRST_VALUE OVER (PARTITION BY device_id, session_seq)` inside a transparent function purely to dodge the scan.

**Pre-conditions.** Phase 1 done. Independent of Phase 2; can land in either order, but Phase 2 first gives a sharper test signal.

**TDD tests to write first.**
- `crates/smelt-planner/src/rules/incremental.rs::tests::test_admissible_over_partition_by_superset` — a CST with `OVER (PARTITION BY device_id, session_seq ORDER BY event_ts)` on a model whose partition grouping is `(device_id, session_seq)` classifies as `FullyBatchSafe` (no `OVER` rejection).
- `crates/smelt-planner/src/rules/incremental.rs::tests::test_inadmissible_over_partition_by_disjoint` — a CST with `OVER (PARTITION BY user_id ORDER BY event_ts)` on a model whose partition grouping is `(device_id,)` is refused.
- `crates/smelt-planner/src/rules/incremental.rs::tests::test_admissible_over_partition_by_equals` — equality (not strict superset) is also admitted.
- `crates/smelt-cli/tests/web_analytics_incremental_classification.rs` (existing) — assert `silver/sessions` classifies without needing the `compute_session_start_date.sql` workaround.

**Implementation shape.**
- Extend the classifier's `OVER` rule in `crates/smelt-planner/src/rules/incremental.rs` to inspect the window's `PARTITION BY` keys and compare against the model's declared partition grouping (sourced from `timeseries.partition_column` after Phase 1).
- "Partition grouping" for the model is the column set the partition is expressed over — for daily-by-`event_date` models, the singleton `{event_date}`; for finer partitioning (post-Phase 1 multi-column support, if applicable), the declared set.
- Reuse the existing AST traversal; add a key-comparison helper.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-planner/src/rules/incremental.rs` — classifier rule, helper.
- Tests as listed.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences entry for "Batch-safety classifier admits OVER (PARTITION BY <keys>)" closes.
- `docs-site/docs/guide/incremental-models.md` — author guidance: partition-aligned window functions are safe directly in the outer body.

**Review checklist.**
- [ ] TDD tests listed above exist and assert the superset relation precisely.
- [ ] Existing `OVER` rejections (non-aligned cases) still fire.
- [ ] Spec Known Divergences entry removed; Semantics § "Safety checks" updated by spec edit if needed (this entry already exists in the spec via the upgrade clause).
- [ ] No false negatives — a literal mistake like `OVER (PARTITION BY foo)` for the wrong column is still refused.

**Commit.** `feat(planner): admit partition-aligned OVER in incremental safety classifier`

---

### Phase 4: Per-source bound derivation (Form A + Form B)

**Goal.** Walk the expanded model body and produce a per-source-reference bound map: `{ source_ref → Bounded(col, before, after) | Unbounded | NotDerivable }`. Read Form A (`RANGE BETWEEN INTERVAL '…' PRECEDING/FOLLOWING` on window frames) and Form B (literal-`INTERVAL` time filters on JOIN/WHERE). Expose the map through `smelt explain --json`.

**Pre-conditions.** Phase 1 done (frontmatter knows source's `timeseries.partition_column`). Phase 3 is preferred so the classifier doesn't need workarounds while testing.

**TDD tests to write first.**
- `crates/smelt-planner/src/analysis/source_bounds.rs::tests::test_range_between_interval_preceding` — Form A: `LAG(x) OVER (PARTITION BY id ORDER BY ts RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW)` over a source partitioned by `event_date` derives `Bounded(event_date, 30min, 0)`.
- `crates/smelt-planner/src/analysis/source_bounds.rs::tests::test_explicit_between_filter` — Form B: `WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1 day' AND m.partition_date` derives `Bounded(event_date, 1d, 0)`.
- `crates/smelt-planner/src/analysis/source_bounds.rs::tests::test_cross_column_tz_rebase` — Form B with cross-column rebase: `WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '1 day' AND m.event_date_local + INTERVAL '1 day'` derives `Bounded(event_ts_utc, 1d, 1d)`.
- `crates/smelt-planner/src/analysis/source_bounds.rs::tests::test_aggregation_max` — same source referenced twice with different ranges takes the union (max before, max after).
- `crates/smelt-planner/src/analysis/source_bounds.rs::tests::test_lookup_source_no_bound` — a source without `timeseries:` produces no bound entry.
- `crates/smelt-planner/src/analysis/source_bounds.rs::tests::test_bare_lag_not_derivable` — `LAG(x) OVER (PARTITION BY id ORDER BY ts)` (no RANGE) derives `NotDerivable`.
- `crates/smelt-planner/src/analysis/source_bounds.rs::tests::test_function_body_traversal` — a model calling `smelt.functions.sessionize(...)` whose body has a `RANGE BETWEEN INTERVAL '30 minutes' PRECEDING` derives the bound from the expanded CST.
- `crates/smelt-cli/tests/web_analytics_source_bounds.rs::test_explain_json_exposes_bounds` — `smelt explain --json` for `silver/sessions` exposes a `source_bounds` field with the derived `{events_parsed: Bounded(event_date, 30min, 0)}`-equivalent map.

**Implementation shape.**
- New module `crates/smelt-planner/src/analysis/source_bounds.rs` containing pure functions: `derive_bound_for_reference(expr, ctx) -> BoundResult` and `derive_model_bounds(expanded_cst, ctx) -> HashMap<SourceRef, BoundResult>`.
- Run after function expansion (per `expansion.md`); take the expanded CST as input.
- Reuse the existing temporal-analysis helpers from `crates/smelt-planner/src/analysis/temporal.rs` (which today knows about `RANGE` / `LAG` patterns) — extend to read the interval and resolve which source backs the column.
- `smelt explain --json` writes the map alongside the existing batch-safety classification.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-planner/src/analysis/source_bounds.rs` — new.
- `crates/smelt-planner/src/analysis/temporal.rs` — extension for interval extraction.
- `crates/smelt-planner/src/rules/incremental.rs` — call the bound derivation; surface `NotDerivable` as a refusal (combining with Phase 2's refusal mechanism).
- `crates/smelt-cli/src/commands/explain.rs` — `--json` field.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences entry for "Per-source bound derivation and source-filter pushdown not yet wired" partially closes (derivation half done; pushdown lands in Phase 5).
- `docs-site/docs/guide/incremental-models.md` — author section on Form A / Form B patterns; what the planner reads.
- `docs-site/docs/reference/smelt-explain.md` — `source_bounds` field in JSON output.

**Review checklist.**
- [ ] TDD tests listed above exist and pass against real-fixture models in `examples/web_analytics/`.
- [ ] Bound derivation runs on the *expanded* CST (function bodies visible).
- [ ] `NotDerivable` produces a refusal diagnostic (Phase 2 mechanism).
- [ ] `smelt explain --json` exposes the bound map.
- [ ] Bound derivation is a pure function — no Salsa imports (per `CLAUDE.md` pure-function rule).

**Commit.** `feat(planner): derive per-source (before, after) bounds from expanded SQL`

---

### Phase 5: Source-filter pushdown on expanded CST

**Goal.** Consume Phase 4's bound map and inject per-reference `WHERE source.partition_col >= run_start − before AND < run_end + after` on each FROM. Outer-WHERE injection (model's own partition column) continues unchanged.

**Pre-conditions.** Phase 4 done.

**TDD tests to write first.**
- `crates/smelt-cli/src/transformer.rs::tests::test_pushdown_emits_per_reference` — given a model with a derived bound `{events_parsed: Bounded(event_date, 1d, 0)}`, the compiled SQL contains `WHERE event_date >= <start>-1d AND event_date < <end>` on the `events_parsed` FROM, *in addition* to the outer model WHERE.
- `crates/smelt-cli/src/transformer.rs::tests::test_pushdown_skips_lookups` — a lookup source (no `timeseries:`) gets no pushdown WHERE.
- `crates/smelt-cli/src/transformer.rs::tests::test_pushdown_inside_function_body` — a model calling `smelt.functions.sessionize(...)` reads `events_parsed` inside the function body; pushdown lands inside the expanded body's FROM.
- `crates/smelt-cli/src/transformer.rs::tests::test_pushdown_same_source_twice` — a self-join takes the union bound and emits the same widened filter on both references.
- `crates/smelt-cli/tests/web_analytics_pushdown.rs::test_pushdown_reduces_scan` — assertion via `smelt explain` SQL output: source FROMs are filtered to the bound range, not the full table.
- `crates/smelt-cli/tests/web_analytics_pushdown.rs::test_full_run_equivalent_with_pushdown` — running `silver/sessions` for `[D, D+1)` with pushdown produces the same rows as before pushdown (Phase 4 + 5 are not supposed to change semantics, only scan size).

**Implementation shape.**
- Extend `crates/smelt-cli/src/transformer.rs` (`inject_time_filter` / new `inject_source_filters`) to take the bound map from Phase 4 and rewrite each `FROM smelt.<path>` reference on the expanded CST.
- Resolve the source's `timeseries.partition_column` from its frontmatter (already loaded in Phase 1).
- The injection is per-reference; the outer-WHERE injection remains a separate rewrite pass on the outermost SELECT.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/transformer.rs` — new injection logic.
- `crates/smelt-cli/src/compiler.rs` — wire the bound map into the transformer.
- Tests as listed.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences entry for "Per-source bound derivation and source-filter pushdown not yet wired" fully closes.
- `docs-site/docs/guide/incremental-models.md` — section on what gets pushed where; example of the rewrite.

**Review checklist.**
- [ ] TDD tests listed above exist and pass.
- [ ] Pushdown is per-reference, never duplicated on the outer query.
- [ ] Outer WHERE on the model's own partition column still applies (untouched).
- [ ] Lookups are not pushdown candidates.
- [ ] Spec Known Divergences entry removed; Semantics § "Source-filter pushdown" describes reality.

**Commit.** `feat(cli): source-filter pushdown using derived per-source bounds`

---

### Phase 6: Run-window-vs-partition decoupling

**Goal.** Accept multi-partition run windows on the CLI. Run a single engine query for the whole window; DELETE+INSERT covers all partitions in the window in one transaction.

**Pre-conditions.** Phases 1 + 5 done (timeseries frontmatter + pushdown that widens correctly with the run window).

**TDD tests to write first.**
- `crates/smelt-cli/tests/incremental_run_window.rs::test_seven_day_window_one_query` — `smelt run --event-time-start D --event-time-end D+7d` on a daily-partitioned model emits *one* engine query covering 7 days; output rows have 7 distinct partition values.
- `crates/smelt-cli/tests/incremental_run_window.rs::test_window_equivalent_to_daily_runs` — running `[D, D+7d)` as one window produces the same per-partition output as 7 successive `[D+i, D+i+1)` runs.
- `crates/smelt-cli/tests/incremental_run_window.rs::test_misaligned_window_rejected` — a run window that isn't an integer multiple of granularity is rejected at planning time.
- `crates/smelt-cli/tests/incremental_run_window.rs::test_delete_covers_full_window` — the DELETE before INSERT removes all 7 partitions in `[D, D+7d)`, not just one.
- `crates/smelt-cli/tests/web_analytics_backfill.rs::test_60_day_backfill_one_call` — the web_analytics example runs a 60-day backfill in one `smelt run` invocation, with one engine query.

**Implementation shape.**
- Update CLI argument handling in `crates/smelt-cli/src/main.rs` to accept multi-partition windows without complaint (already accepts arbitrary `[start, end)`; the validation against granularity alignment is the change).
- Update `crates/smelt-cli/src/executor.rs::execute_model_incremental` so the DELETE step uses the full window range, not the partition-by-partition loop that exists today for `BoundedSafe(n)` chunking.
- Chunking semantics: `FullyBatchSafe` runs the whole window in one query; `BoundedSafe(n)` still chunks per `n`, but each chunk can be multi-partition; `PerPartitionOnly` still one partition at a time.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/main.rs` — alignment validation.
- `crates/smelt-cli/src/executor.rs` — DELETE+INSERT per window.
- `crates/smelt-backend/src/lib.rs` and `crates/smelt-backend-duckdb/src/lib.rs` — `delete_partitions()` accepts a `[start, end)` range, not a list-of-partitions.
- Tests as listed.

**Docs touched.**
- `docs/specs/incremental_models.md` — Semantics § "Run window vs partition granularity" describes reality (no Known Divergence entry needed; this is greenfield surface for the spec).
- `docs-site/docs/guide/incremental-models.md` — backfill section: one call for the whole range, not a loop.

**Review checklist.**
- [ ] TDD tests listed above exist and assert one-query-per-window for `FullyBatchSafe`.
- [ ] Per-partition equivalence held (window-as-one == window-as-loop).
- [ ] DELETE covers full window; INSERT inserts the windowed result.
- [ ] `BoundedSafe(n)` chunking still works (each chunk is multi-partition up to `n`).
- [ ] `PerPartitionOnly` semantics preserved.

**Commit.** `feat(cli): one engine query per run window; decouple from partition granularity`

---

### Phase 7: Per-partition equivalence harness

**Goal.** Promote `examples/web_analytics/verify_incremental_equivalence.py` into a first-class test that runs as part of `cargo test`, asserting per-partition equivalence (`incremental[p] == full_refresh().where(partition = p)`) for every model in the example with a local-column shape. Documents the global-column divergence per `incremental_models.md` §"Per-partition equivalence".

**Pre-conditions.** Phases 1–6 done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/per_partition_equivalence.rs::test_local_columns_equivalent` — runs both pipelines on the web_analytics example, asserts per-partition equality on `raw` and `forward_only` columns for every partition.
- `crates/smelt-cli/tests/per_partition_equivalence.rs::test_global_columns_documented_divergence` — asserts that `dau_connected_components` and `dau_backward_fill` *may* diverge per the spec; the test verifies *which* partitions diverge and that the divergence pattern matches the as-of-day-D property described in the README.
- `crates/smelt-cli/tests/per_partition_equivalence.rs::test_runs_under_test_harness` — the harness runs without a manual Python invocation; `cargo test -p smelt-cli` exercises it.

**Implementation shape.**
- Port `examples/web_analytics/verify_incremental_equivalence.py` to Rust as a test fixture under `crates/smelt-cli/tests/`. Reuse the existing example workspace; drive `smelt run --refresh` and `smelt run --event-time-start D --event-time-end D+1d` from inside the test.
- Use the existing `assert_eq!` machinery on DuckDB query results; capture per-column-pair divergences for global columns and assert the divergence count is bounded (the README's 45/60–48/60-day figure).
- Wire into CI via the existing test target.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/per_partition_equivalence.rs` — new.
- `examples/web_analytics/verify_incremental_equivalence.py` — kept for human convenience but flagged as superseded by the Rust harness.

**Docs touched.**
- `docs/specs/incremental_models.md` — no change (Semantics § "Per-partition equivalence" already describes this).
- `docs-site/docs/concepts/incremental-equivalence.md` — new short page describing the equivalence property and the as-of-day-D divergence, with a link to the example.

**Review checklist.**
- [ ] TDD tests listed above exist and pass.
- [ ] Equivalence asserted per-partition, not per-table.
- [ ] Global-column divergence is documented, not asserted away.
- [ ] Test runs under `cargo test -p smelt-cli` without manual setup.

**Commit.** `test(cli): per-partition equivalence harness against examples/web_analytics`

---

### Phase 8: `examples/web_analytics/` simplification

**Goal.** Remove the workarounds that exist purely because the planner couldn't read Form A / Form B or admit partition-aligned `OVER`. Specifically: delete `functions/compute_session_start_date.sql`, inline the `FIRST_VALUE OVER` back into `silver/sessions.sql`, drop the 2-day driver window in `run_incremental.py`, and rewrite `gold/identity_forward_only` to use the explicit Form B date filter.

**Pre-conditions.** Phases 1–7 done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs` (existing) — must still pass after the example rewrite; snapshot output unchanged.
- `crates/smelt-cli/tests/web_analytics_incremental_classification.rs` (existing) — `silver/sessions` classifies without the workaround function.
- `examples/web_analytics/run_incremental.py` updated to pass `[D, D+1)` per iteration; the test runner confirms the per-partition output matches the prior 2-day-window output.

**Implementation shape.**
- Delete `examples/web_analytics/functions/compute_session_start_date.sql`.
- Inline `FIRST_VALUE(event_date) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts)` into `silver/sessions.sql`.
- Edit `gold/identity_forward_only.sql` to add an explicit Form B WHERE date filter on the sessions join (per the migration path called out in the predecessor research doc).
- Simplify `run_incremental.py` to pass a single-day window; rely on the planner to widen the source reads.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/functions/compute_session_start_date.sql` — deleted.
- `examples/web_analytics/models/silver/sessions.sql` — inline the `FIRST_VALUE OVER`.
- `examples/web_analytics/models/gold/identity_forward_only.sql` — Form B WHERE.
- `examples/web_analytics/run_incremental.py` — single-day window.
- `examples/web_analytics/README.md` — update the "why this is shaped this way" notes; remove the workaround callouts.

**Docs touched.**
- `examples/web_analytics/README.md` — the example's own README; reflects the post-cleanup shape.
- `docs-site/docs/examples/web-analytics.md` — if exists, updated; otherwise skipped.

**Review checklist.**
- [ ] No model in `examples/web_analytics/` exists solely as a workaround.
- [ ] Existing snapshot tests pass; behaviour is unchanged.
- [ ] Driver passes per-partition windows; planner widens automatically.
- [ ] README explains the SQL shapes the planner reads (Form A + Form B) rather than the workarounds.

**Commit.** `refactor(examples): simplify web_analytics now that planner reads Form A/B`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:

- `cargo test --quiet 2>&1 | tail -40` — workspace tests green.
- `cargo test -p smelt-cli --test example_diagnostics` — every example workspace loads clean.
- `cargo test -p smelt-lsp --test example_workspaces` — LSP-side discovery clean (catches asymmetric discovery bugs).
- `cargo test -p smelt-cli --test per_partition_equivalence` (Phase 7) — equivalence harness passes.
- `cargo test -p smelt-cli --test web_analytics_refactor_snapshot` — example snapshot unchanged.
- `cargo test -p smelt-cli --test web_analytics_incremental_classification` — every incremental model in the example classifies as `fully_batch_safe`.
- `smelt explain --json` on `silver/sessions` exposes a `source_bounds` field with the expected per-source map.
- `/smelt:validate incremental_models` reports zero drift.
- `/smelt:validate timeseries` reports zero drift.
- The `examples/web_analytics/functions/compute_session_start_date.sql` file no longer exists.
- The `incremental_models.md` Known Divergences for "Migration to `timeseries:` block pending", "Per-source bound derivation and source-filter pushdown not yet wired", "Refused-incrementality replaces silent downgrade", and "Batch-safety classifier admits `OVER (PARTITION BY <keys>)`" entries are all removed.
