# Plan: cumulative_aggregate materialization

**Date**: 2026-05-23
**Spec**: [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md)
**Spec diff**: introduction of `docs/specs/cumulative_aggregate.md`; updates to `docs/specs/models.md` (new materialization mode, new constraint rows) and `docs/specs/incremental_models.md` (drop `IncrementalStrategy::Merge` from the strategy enum, dead-code Known Divergence)
**Tracking branch**: `worktree-web_analytics`
**Docs**: code+docs (user-facing surface — materializations guide gains the new mode)
**Motivating example**: `examples/web_analytics/models/silver/device_user_edges{,_cumulative}.sql` — the two-model workaround the new materialization deletes.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/cumulative_aggregate.md` (the correctness oracle) and the relevant sections of `docs/specs/models.md` and `docs/specs/incremental_models.md`.
2. Read `docs/research/20260522-cumulative-as-its-own-rule.md` for design rationale — do not re-open settled decisions.
3. Confirm you are on branch `worktree-web_analytics`.
4. Find the next `pending` phase in Progress tracking.

**For each phase:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**Conventions every phase:**
- Real-fixture tests in `examples/` alongside any AST units.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits using the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Honour `CLAUDE.md` invariants (`type_inference.rs` purity, project-isolation rule, workspace-loading-parity rule).
- **Timeless-oracle rule.** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/cumulative_aggregate.md`, `docs/specs/models.md`, and `docs-site/docs/...` describe the feature as if it has always existed.

---

## Context

`cumulative_aggregate.md` introduces a new materialization mode whose output is one row per `GROUP BY` key, where each row reflects state across every processed source partition. The materialization is selected via `materialization: cumulative_aggregate` and carries no other frontmatter — the unique key and per-column combiners are derived from the SELECT. The rule reads the driving partition shape from a single `timeseries:`-tagged source in the FROM clause and uses the existing `Backend::merge_into` primitive for per-partition UPSERT. The plan also drops the `IncrementalStrategy::Merge` enum variant, which was a placeholder for the cumulative-as-strategy shape the spec rejects (`incremental_models.md` Known Divergences).

The motivating concrete simplification is `examples/web_analytics/`: today's two-model split (a per-day incremental table + a cumulative view) collapses to a single cumulative model. The plan also closes Gap #5 from `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md`.

## Scope

### In scope (spec coverage)

- `cumulative_aggregate.md` §Surface — `materialization: cumulative_aggregate` selector; forbid-`timeseries:` and forbid-`incremental:` validation; diagnostic codes.
- `cumulative_aggregate.md` §Semantics → "Classifier checks" — GROUP BY required; allowlist aggregators on non-key projections; no window functions; no non-determinism; GROUP BY must not contain the driving source's partition column.
- `cumulative_aggregate.md` §Semantics → "Driving source" — exactly one timeseries-tagged source in the FROM clause.
- `cumulative_aggregate.md` §Semantics → "Execution model" — per-partition step loop calling `Backend::merge_into` with the derived combiner-aware `source_sql`.
- `cumulative_aggregate.md` §Semantics → "Source-filter pushdown" — per-partition pushdown on the driving source.
- `cumulative_aggregate.md` §Semantics → "Reprocessing semantics" — refuse v1; `--full-refresh` is the mitigation.
- `cumulative_aggregate.md` §Semantics → "Cross-partition equivalence" — verifiable via a test harness.
- Dropping `IncrementalStrategy::Merge` and removing its dispatcher branch in `Backend::execute_model_incremental`.
- `examples/web_analytics/` migration (delete cumulative view, shrink the model, fix downstream references).
- User docs at `docs-site/docs/guide/materializations.md`.

### Explicitly deferred

- **Sibling rules** (`scd2`, `latest_value`, `accumulating_snapshot`). Each is its own future spec + plan; the rule-API surface stays stable across rules per the predecessor research.
- **`AVG` rewrite.** Refused by the classifier in v1; future plan may rewrite to `SUM/COUNT`.
- **`driven_by:` for multi-source cumulative.** Refused in v1; future plan may add the disambiguation field.
- **Self-referential cumulative.** Refused in v1.
- **Delta-history side table** for reprocessing reversible aggregators. Refused-reprocessing is the v1 policy; `--full-refresh` is the mitigation.
- **`--auto` staleness fidelity** beyond the conservative "any partition ≥ earliest stale" behaviour.
- **Schema evolution** for cumulative tables (adding a new aggregate column).
- **LSP enrichment** for the new diagnostic codes — code spans surface via the standard diagnostic channel; LSP-specific enrichment is independent.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1 — `Materialization::CumulativeAggregate` variant + frontmatter validation | done | 32f6a0d8 | 2026-05-23 |
| 2 — Cumulative classifier (pure function, allowlist + GROUP BY + driving source) | done | 16c526b2 | 2026-05-23 |
| 3 — Per-partition execution loop + source-filter pushdown + combiner-aware source SQL | done | ab78810a | 2026-05-23 |
| 4 — Drop `IncrementalStrategy::Merge` and its dispatcher branch | done |  | 2026-05-23 |
| 5 — Cross-partition equivalence harness (real DuckDB fixture in `examples/web_analytics`) | pending |  |  |
| 6 — `examples/web_analytics/` migration to `cumulative_aggregate` | done | f087673a | 2026-05-23 |
| 7 — User docs: `docs-site/docs/guide/materializations.md` + cross-link from incremental | pending |  |  |

---

### Phase 1: `Materialization::CumulativeAggregate` variant + frontmatter validation

**Goal.** Extend `Materialization` with `CumulativeAggregate`. Parse `materialization: cumulative_aggregate` from frontmatter and `smelt.yml`. Emit `CumulativeForbidsTimeseries` and `CumulativeForbidsIncremental` when those blocks appear alongside it. Wire the variant through the metadata + config paths so downstream code can branch on it but does not yet execute it.

**Pre-conditions.** None — entry point.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs::tests::test_materialization_cumulative_aggregate_parses` — `Materialization::deserialize` of `"cumulative_aggregate"` produces the new variant.
- `crates/smelt-core/src/metadata.rs::tests::test_cumulative_aggregate_frontmatter_parses` — a `.sql` file with `materialization: cumulative_aggregate` and no other rule-specific keys parses cleanly.
- `crates/smelt-core/src/metadata.rs::tests::test_cumulative_aggregate_forbids_timeseries` — a `.sql` file with `materialization: cumulative_aggregate` + a `timeseries:` block emits `CumulativeForbidsTimeseries`.
- `crates/smelt-core/src/metadata.rs::tests::test_cumulative_aggregate_forbids_incremental` — same with an `incremental:` block emits `CumulativeForbidsIncremental`.
- `crates/smelt-core/src/config.rs::tests::test_smelt_yml_cumulative_aggregate` — `models.<name>.materialization: cumulative_aggregate` in `smelt.yml` round-trips.
- `crates/smelt-cli/tests/example_diagnostics.rs::test_examples_load_clean` (existing) — still passes; no example workspace regresses.

**Implementation shape.**
- Add `CumulativeAggregate` to the `Materialization` enum in `crates/smelt-core/src/config.rs`; extend `Deserialize`/`Serialize` to recognise `"cumulative_aggregate"`.
- Add `CumulativeForbidsTimeseries` and `CumulativeForbidsIncremental` to the diagnostic surface in `crates/smelt-core/src/metadata.rs` (alongside the existing `TimeseriesRequiredForIncremental` plumbing).
- Update `crates/smelt-core/src/config.rs::validate_model_configs` to enforce the forbid-combination rules. Mirror the structure of the existing `ephemeral + timeseries` / `test + incremental` validation.
- Touch the executor/run/backbuild dispatch paths only to add a `Materialization::CumulativeAggregate => unreachable!("handled in Phase 3")` arm so the build stays green; full execution lands in Phase 3.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs`
- `crates/smelt-core/src/metadata.rs`
- `crates/smelt-backend/src/lib.rs` — `Materialization` mirror (cross-crate type) and its match arms.
- `crates/smelt-cli/src/executor.rs` — `unreachable!` placeholder arms.
- `crates/smelt-cli/src/config.rs` — if the CLI re-declares `Materialization`, extend it consistently.

**Docs touched.**
- `docs/specs/cumulative_aggregate.md` — already authoritative.
- `docs/specs/models.md` — already updated alongside the spec.

**Review checklist.**
- [ ] TDD tests listed above exist and assert what the spec requires.
- [ ] `Materialization::CumulativeAggregate` reachable from every place that branches on the enum (the compiler enforces this once the variant is added).
- [ ] Forbid-combination diagnostics surface through the same channel as `TimeseriesRequiredForIncremental`.
- [ ] No `Materialization::CumulativeAggregate` execution path lands yet — the placeholder `unreachable!` is acceptable for one phase.
- [ ] Spec / docs edits stay timeless (no phase vocabulary in spec or guide).

**Commit.** `feat(core): Materialization::CumulativeAggregate + forbid-block diagnostics`

---

### Phase 2: Cumulative classifier (pure function)

**Goal.** Implement the classifier from `cumulative_aggregate.md` §"Classifier checks" as a pure function. Input: the inlined (post-expansion) outer SELECT plus the source-resolution context. Output: a `CumulativeClassification` carrying the derived `unique_key`, the per-column `(per_partition_agg, cross_partition_combiner)` map, the driving source reference, and any classifier diagnostics.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_classify_simple` — the motivating `device_user_edges` SELECT (COUNT/MIN/MAX with GROUP BY device_id, user_id and a single timeseries source) classifies with unique_key `[device_id, user_id]`, three aggregator columns mapped to `(COUNT,SUM)`/`(MIN,MIN)`/`(MAX,MAX)`, and the driving source resolved.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_no_group_by_refused` — a SELECT with no GROUP BY produces `CumulativeRequiresGroupBy`.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_unknown_aggregator_refused` — `STRING_AGG(name, ',')` on a non-key projection produces `CumulativeUnknownAggregator` naming the function and the projection.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_composite_aggregate_expression_refused` — `SUM(x) + 1` as a projection produces `CumulativeUnknownAggregator` (composite expressions are not direct calls).
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_group_by_contains_partition_column_refused` — a SELECT with GROUP BY `device_id, user_id, event_date` and a driving source partitioned by `event_date` produces `CumulativeGroupByContainsPartitionColumn`.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_window_function_refused` — an `OVER (...)` projection produces `CumulativeForbidsWindowFunctions`.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_nondeterministic_refused` — `NOW()` in a projection produces `CumulativeForbidsNondeterministic`.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_zero_driving_sources_refused` — a SELECT from a source with no `timeseries:` declaration produces `CumulativeNoDrivingSource`.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_multiple_driving_sources_refused` — two timeseries-tagged sources in the FROM clause produce `CumulativeMultipleDrivingSources` listing both.
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_lookup_source_admitted` — a SELECT from one timeseries-tagged source and one non-timeseries lookup classifies cleanly (the lookup does not count toward the driving-source cardinality).
- `crates/smelt-planner/src/rules/cumulative.rs::tests::test_function_expansion_aggregator` — a SELECT whose projection calls a `smelt.define`-resolved aggregator function classifies on the expanded SQL.

**Implementation shape.**
- New module `crates/smelt-planner/src/rules/cumulative.rs` with:
  - `pub struct CumulativeClassification { unique_key, aggregator_columns, driving_source, diagnostics }`.
  - `pub fn classify_cumulative(expanded_sql: &str, ctx: &ClassifierContext) -> CumulativeClassification` — pure function.
  - `pub struct AggregatorColumn { output_name, per_partition_agg, cross_partition_combiner }`.
  - The fixed allowlist + combiner lookup table as a `const &[...]`.
- `ClassifierContext` carries the source-resolution map (so the driving-source walk knows which targets declare `timeseries:`). It is passed in (per the `smelt-db` pure-function rule); the Salsa-side wrapper produces it.
- The classifier shares the CST-walking framework with `crates/smelt-planner/src/rules/incremental.rs` (analysis helpers in `crates/smelt-planner/src/analysis/`); add helpers as needed but keep them shared.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-planner/src/rules/cumulative.rs` (new).
- `crates/smelt-planner/src/rules/mod.rs` — register the module.
- `crates/smelt-planner/src/analysis/` — add the projection-reading helper if not already present.
- `crates/smelt-planner/src/lib.rs` — re-export `CumulativeClassification` at the crate boundary.

**Docs touched.** None in this phase — spec is authoritative.

**Review checklist.**
- [ ] Every diagnostic code from `cumulative_aggregate.md` §"Diagnostic codes" that the classifier owns has at least one TDD test that fires it.
- [ ] Allowlist is a single source of truth — there is no second place to update if a new aggregator is added.
- [ ] `classify_cumulative` is a pure function — no Salsa imports, no database access (per the smelt-db pure-function rule, even though this code lives in `smelt-planner`).
- [ ] Function-expansion-aware: classifier reads the expanded SQL, not the raw outer body.
- [ ] No code path executes a cumulative model yet.

**Commit.** `feat(planner): cumulative_aggregate classifier + diagnostics`

---

### Phase 3: Per-partition execution loop + source-filter pushdown + combiner-aware source SQL

**Goal.** Wire `Materialization::CumulativeAggregate` into the CLI's execution path. For a run window `[run_start, run_end)`:

1. Classify the model (Phase 2).
2. Step over the driving source's partitions in temporal order.
3. For each partition `D`: inject the source-filter pushdown WHERE clause on the driving source; either create the output table from the delta SELECT (first run) or call `Backend::merge_into` with a `source_sql` that wraps the delta SELECT in the combiner-aware MERGE shape.

Refuse reprocessing at planning time when the run window includes already-merged partitions (consult the backend's table existence + a simple "earliest partition not yet merged" check; conservative — refuse on any overlap with existing data unless `--full-refresh`).

**Pre-conditions.** Phases 1 + 2 done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/cumulative_aggregate/first_run.rs::test_first_run_creates_table` — a fresh-state cumulative model run over a 1-day window creates the target table with the correct schema and one row per key.
- `crates/smelt-cli/tests/cumulative_aggregate/multi_partition.rs::test_three_partitions_combine` — three sequential 1-day runs over partitions D1/D2/D3 produce per-key combined values (sum of counts, min of first_seen, max of last_seen) equal to a full-refresh over the same input.
- `crates/smelt-cli/tests/cumulative_aggregate/run_window_multi_partition.rs::test_single_run_three_partitions` — one run with `[D1, D4)` produces the same final state as three sequential `[D1, D2)`/`[D2, D3)`/`[D3, D4)` runs.
- `crates/smelt-cli/tests/cumulative_aggregate/source_filter_pushdown.rs::test_pushdown_per_partition` — the driving source receives a WHERE filter scoped to one partition at a time (inspect via `smelt explain --json` or a captured execution trace).
- `crates/smelt-cli/tests/cumulative_aggregate/reprocess_refused.rs::test_reprocess_overlaps_refused` — a run window that overlaps an already-merged partition exits non-zero with a diagnostic pointing at `--full-refresh`.
- `crates/smelt-cli/tests/cumulative_aggregate/lookup_source_full_read.rs::test_non_timeseries_lookup_read_in_full` — a cumulative model joining a lookup table reads the lookup in full on every partition step.
- `crates/smelt-backend-duckdb/src/lib.rs::tests::test_merge_into_with_combiner_source_sql` — the `merge_into` primitive accepts a `source_sql` that performs the combine projection (verifies the backend handshake — no changes to the backend itself).

**Implementation shape.**
- New module `crates/smelt-cli/src/cumulative.rs` with the per-partition step loop. The loop:
  1. Calls `classify_cumulative(...)` to derive unique_key, aggregator_columns, and driving_source.
  2. Computes the partition list `[D₁, …, Dₙ]` from `(run_start, run_end, driving_source.granularity)` using the existing partition-arithmetic helpers (the same ones incremental uses).
  3. For each `Dᵢ`: rewrites the inlined SQL to inject `<driving_source>.<partition_column> >= Dᵢ AND <driving_source>.<partition_column> < Dᵢ + granularity` on the driving source's reference; computes the `source_sql` for `merge_into` by wrapping the delta SELECT in a CTE and projecting the combiner-aware columns (e.g., `target.event_count + delta.event_count`); calls `Backend::merge_into(schema, table, source_sql, &unique_key)`.
  4. For the first partition when the target table does not exist, calls `Backend::create_table_as(schema, table, delta_sql)` instead of `merge_into`.
- The combiner-aware `source_sql` construction is a pure function `build_merge_source_sql(delta_sql, &unique_key, &aggregator_columns) -> String`; testable in isolation.
- Refuse reprocessing: before the loop, check whether the target table exists and whether `Dᵢ` is already represented (a simple "table exists and the run window starts ≤ current max merged partition" heuristic; refuse with `--full-refresh` mitigation in the diagnostic). The reprocessing check is conservative: when no metadata is tracked, refuse if the table exists and the run window's start is ≤ the smallest partition implied by existing rows; document the heuristic.
- The CLI dispatch in `crates/smelt-cli/src/commands/run.rs` and `crates/smelt-cli/src/commands/backbuild.rs` branches on `Materialization::CumulativeAggregate` and calls into `crates/smelt-cli/src/cumulative.rs`. Replace the Phase 1 `unreachable!` arms.
- The classifier diagnostics (Phase 2) are surfaced as CLI errors via the existing diagnostic plumbing — the same channel that surfaces `TimeseriesRequiredForIncremental`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/cumulative.rs` (new).
- `crates/smelt-cli/src/commands/run.rs` — dispatch arm.
- `crates/smelt-cli/src/commands/backbuild.rs` — dispatch arm.
- `crates/smelt-cli/src/executor.rs` — replace `unreachable!` placeholder.
- `crates/smelt-planner/src/analysis/` — any helpers needed for source-filter pushdown on a single source reference (likely already present from incremental's pushdown work; reuse).

**Docs touched.** None in this phase — spec is authoritative; user docs land in Phase 7.

**Review checklist.**
- [ ] TDD tests listed above exist and exercise real DuckDB.
- [ ] Cross-partition equivalence holds for every aggregator in the allowlist (the multi-partition test covers COUNT/MIN/MAX; add coverage for SUM/BOOL_OR/BIT_XOR if not already exercised — at least one test per aggregator family).
- [ ] First-run path is `create_table_as`, not `merge_into` into a non-existent table.
- [ ] Source-filter pushdown WHEREs are correctly bounded per partition (test verifies the captured SQL).
- [ ] Reprocessing is refused with a clear diagnostic; `--full-refresh` works.
- [ ] No regressions in `cargo test -p smelt-cli --test example_diagnostics` or `cargo test -p smelt-lsp --test example_workspaces`.

**Commit.** `feat(cli): cumulative_aggregate per-partition merge loop`

---

### Phase 4: Drop `IncrementalStrategy::Merge` and its dispatcher branch

**Goal.** Remove `IncrementalStrategy::Merge` from the `IncrementalStrategy` enum, drop its arm in `Backend::execute_model_incremental`'s dispatcher, drop the `resolve_strategy` branch that selected it, and update any tests that referenced the variant. `Backend::merge_into` itself stays — it is the cumulative rule's physical primitive (`cumulative_aggregate.md` §"Execution model").

**Pre-conditions.** Phase 3 done (cumulative rule now owns the `merge_into` caller relationship; removing `IncrementalStrategy::Merge` leaves no caller-orphan).

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs::tests::test_incremental_strategy_no_merge_variant` — `IncrementalStrategy::deserialize("merge")` returns an error (the value is no longer accepted).
- `crates/smelt-backend-duckdb/src/lib.rs::tests::test_merge_into_upsert` (existing) — still passes; the primitive is unchanged.
- Existing CLI / executor tests pass without modification (the variant was unreachable from frontmatter, so no test should have been driving it from end-to-end).

**Implementation shape.**
- Edit `crates/smelt-core/src/config.rs`: remove the `Merge` variant from `IncrementalStrategy`.
- Edit `crates/smelt-backend/src/lib.rs`: remove the `IncrementalStrategy::Merge => self.merge_into(...)` arm from `Backend::execute_model_incremental`'s dispatch (around line 216). The `merge_into` trait method definition and the DuckDB implementation stay.
- Edit `crates/smelt-backend/src/lib.rs::resolve_strategy`: remove the `supports_merge` + `unique_key` branch that selected `IncrementalStrategy::Merge`. `resolve_strategy` now always returns `IncrementalStrategy::DeleteInsert` (incremental's v1 default).
- Edit `crates/smelt-cli/tests/incremental/strategies.rs` and `crates/smelt-cli/src/helpers.rs` to remove any references to `IncrementalStrategy::Merge`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs`
- `crates/smelt-backend/src/lib.rs`
- `crates/smelt-cli/src/executor.rs` (if it pattern-matches on the variant)
- `crates/smelt-cli/src/helpers.rs`
- `crates/smelt-cli/tests/incremental/strategies.rs`
- `crates/smelt-cli/tests/incremental/lookback.rs`
- `crates/smelt-backend-duckdb/src/lib.rs` (only if the variant is referenced — `merge_into` itself and its tests stay).

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergence entry for "`IncrementalStrategy::Merge` variant is dead code" is removed (or marked closed); §Surface "Strategy enum" already reflects the new enum shape.

**Review checklist.**
- [ ] `rg 'IncrementalStrategy::Merge' crates/` returns no results.
- [ ] `Backend::merge_into` trait method definition and its DuckDB implementation are unchanged.
- [ ] `Backend::merge_into` unit tests (`test_merge_into_upsert`, `test_merge_into_insert_only`) still pass.
- [ ] `cargo clippy --all-targets` is warning-free.
- [ ] Known Divergence entry in `docs/specs/incremental_models.md` is closed.

**Commit.** `refactor(core): drop IncrementalStrategy::Merge variant (cumulative_aggregate owns merge_into)`

---

### Phase 5: Cross-partition equivalence harness

**Goal.** A real-DuckDB test fixture that asserts `cumulative_aggregate.md` §"Cross-partition equivalence" — for any set of source partitions `S` and any ordering π over `S`, processing π(S) through the cumulative rule produces the same final state as a full-refresh over `source.where(partition ∈ S)`.

This is the load-bearing property and deserves a dedicated harness, mirroring `crates/smelt-cli/tests/per_partition_equivalence/` from the incremental work (referenced in the prior incremental plan).

**Pre-conditions.** Phase 3 done. Phase 4 is independent; either order works.

**TDD tests to write first.**
- `crates/smelt-cli/tests/cumulative_equivalence/forward.rs::test_forward_ordering_equivalent` — partitions D1, D2, D3 processed in temporal order produce the same table as a full refresh.
- `crates/smelt-cli/tests/cumulative_equivalence/reverse.rs::test_reverse_ordering_equivalent` — partitions D3, D2, D1 processed in reverse order produce the same table as the forward run (cross-partition equivalence under reordering).
- `crates/smelt-cli/tests/cumulative_equivalence/shuffled.rs::test_shuffled_ordering_equivalent` — a fixed-shuffle order (e.g., D2, D1, D3) produces the same table.
- `crates/smelt-cli/tests/cumulative_equivalence/aggregator_coverage.rs::test_all_allowlist_aggregators_equivalent` — one model per aggregator family (COUNT, SUM, MIN, MAX, BOOL_AND/OR, BIT_AND/OR/XOR) confirms each upholds the contract.
- Re-runs of identical input converge (idempotence under fixed input check, separate from reordering).

**Implementation shape.**
- New test module `crates/smelt-cli/tests/cumulative_equivalence/` mirroring the structure of incremental's per-partition equivalence harness.
- Shared fixture: a fresh DuckDB target table, a small synthetic timeseries source (e.g., `(event_date, key, value)` with 3 partitions × ~10 rows), a cumulative model SELECT that exercises the aggregator under test.
- Helper: `run_cumulative_in_order(partitions: &[Date], model: &str) -> TableState` and `full_refresh(model: &str, partitions: &[Date]) -> TableState`; assert `TableState` equality.
- `TableState` is `Vec<Row>` sorted by `unique_key` — direct equality, no fuzzy matching.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/cumulative_equivalence/` (new tree).
- Possibly `crates/smelt-cli/tests/common/` — if a shared fixture helper is added, follow the existing test-helpers conventions.

**Docs touched.** None — the harness is internal test coverage; it does not change user-facing surface.

**Review checklist.**
- [ ] Every aggregator family in the v1 allowlist is exercised in the harness.
- [ ] Forward, reverse, and shuffled orderings are all tested.
- [ ] `TableState` equality is bit-exact (sorted rows, direct equality).
- [ ] The harness fails informatively when run against a (deliberately) bad combiner — verify by temporarily swapping a combiner in a test branch and confirming the assert fires.
- [ ] CI time impact is reasonable (≤ 30 seconds added; cumulative tests use small synthetic data).

**Commit.** `test(cli): cumulative_aggregate cross-partition equivalence harness`

---

### Phase 6: `examples/web_analytics/` migration

**Goal.** Apply the migration described in `docs/research/20260522-cumulative-as-its-own-rule.md` §"Migration impact on `examples/web_analytics/`":

1. Delete `examples/web_analytics/models/silver/device_user_edges_cumulative.sql`.
2. Shrink `examples/web_analytics/models/silver/device_user_edges.sql` to the `cumulative_aggregate` shape: frontmatter becomes `materialization: cumulative_aggregate` (drop `timeseries:` and `incremental:` blocks); SELECT drops `event_date` from the projection and GROUP BY; per-day column names lose their `daily_` prefix.
3. Update downstream FROM references in `gold/identity_backward_fill.sql` and `gold/identity_connected_components.sql` (and any others) from `smelt.silver.device_user_edges_cumulative` to `smelt.silver.device_user_edges`. Column references should not need to change — the cumulative output's column names (`event_count`, `first_seen`, `last_seen`) match the old cumulative view's projection.
4. Update `examples/web_analytics/README.md` to remove the "two-model split because of smelt limitation" caveat.
5. Update any tests that hardcode the old shape — particularly `tests/device_user_edges_per_day_invariants.test.sql`. The per-day invariant test does not apply to the cumulative shape; either delete it or replace it with a cumulative equivalence test (which is generic in Phase 5, so probably delete).

**Pre-conditions.** Phases 1–4 done. Phase 5 is recommended but not strictly required.

**TDD tests to write first.**
- The migration is itself a verification — the existing `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` must pass after the file changes.
- `examples/web_analytics/` running end-to-end via `smelt run` produces the same output as before the migration — capture via a one-shot before/after row count + per-key spot check.
- The `examples/web_analytics/tests/` test files that reference `device_user_edges_cumulative` must still pass after the rename (or be removed if the underlying invariant no longer applies).

**Implementation shape.**
- File deletion: `examples/web_analytics/models/silver/device_user_edges_cumulative.sql`.
- File rewrite: `examples/web_analytics/models/silver/device_user_edges.sql` to:
  ```sql
  ---
  materialization: cumulative_aggregate
  ---
  SELECT
      device_id,
      user_id,
      COUNT(*) AS event_count,
      MIN(event_ts) AS first_seen,
      MAX(event_ts) AS last_seen
  FROM smelt.silver.events_parsed
  WHERE user_id IS NOT NULL
  GROUP BY device_id, user_id
  ```
- Search-and-replace of `smelt.silver.device_user_edges_cumulative` → `smelt.silver.device_user_edges` across `examples/web_analytics/`.
- README update: drop the workaround caveat; mention `cumulative_aggregate` once as the materialization the model uses.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/device_user_edges.sql`
- `examples/web_analytics/models/silver/device_user_edges_cumulative.sql` (delete)
- `examples/web_analytics/models/gold/identity_backward_fill.sql`
- `examples/web_analytics/models/gold/identity_connected_components.sql`
- `examples/web_analytics/models/marts/identity_method_comparison.sql` (if it references the cumulative view)
- `examples/web_analytics/models/marts/daily_active_users_by_method.sql` (if it references the cumulative view)
- `examples/web_analytics/tests/*.test.sql` (audit; rewrite or delete tests that hardcode the per-day shape)
- `examples/web_analytics/README.md`

**Docs touched.**
- `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md` — mark Gap #5 as closed (link this plan).

**Review checklist.**
- [ ] `silver/device_user_edges_cumulative.sql` is deleted.
- [ ] `silver/device_user_edges.sql` frontmatter is two lines.
- [ ] All downstream `device_user_edges_cumulative` references are updated.
- [ ] `examples/web_analytics/README.md` no longer mentions the workaround.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` passes.
- [ ] `cargo test -p smelt-lsp --test example_workspaces` passes.
- [ ] `smelt run` end-to-end on `examples/web_analytics/` produces the expected output (sanity check, document the row counts in the commit message).
- [ ] Gap #5 in the research gap catalogue is marked closed with a link to this plan.

**Commit.** `refactor(examples/web_analytics): collapse device_user_edges to cumulative_aggregate`

---

### Phase 7: User docs

**Goal.** Document `cumulative_aggregate` in `docs-site/docs/guide/materializations.md` as a peer of the existing modes. Cross-link from `docs-site/docs/guide/incremental-models.md`'s "when to use incremental vs ..." discussion. The spec is the normative source of truth; user docs explain the concept and surface in idiomatic prose.

**Pre-conditions.** Phase 3 done (the feature works end-to-end).

**TDD tests to write first.**
- N/A — docs phase. Smoke check: the user-facing examples in the new doc parse cleanly when copied into a test workspace (covered by `example_diagnostics` if a new example workspace is added; otherwise a manual `smelt run` check).

**Implementation shape.**
- New section in `docs-site/docs/guide/materializations.md` describing `cumulative_aggregate`: what it is, when to reach for it (vs `incremental:`, vs `view`), the frontmatter shape (one line), the SELECT shape (GROUP BY + allowlisted aggregators), the allowlist table, what the cross-partition combiner is (one paragraph; cite the spec for the proof).
- A short "Two patterns for time-aware tables" section either in this guide or in `docs-site/docs/guide/incremental-models.md`: incremental keeps a per-partition shape; cumulative collapses to one row per key. Cross-link both ways.
- One worked example mirroring the new `device_user_edges` model.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/guide/materializations.md`
- `docs-site/docs/guide/incremental-models.md` (cross-link)

**Docs touched.** As above. Spec stays authoritative; do not duplicate normative rules in the user guide.

**Review checklist.**
- [ ] The user guide reads as feature description, not changelog.
- [ ] Timeless: no phase vocabulary, no "now supports", no "as of …".
- [ ] The allowlist table appears in user docs and matches the spec verbatim.
- [ ] Cross-links between materializations.md and incremental-models.md flow naturally.
- [ ] The worked example matches the post-migration `examples/web_analytics/models/silver/device_user_edges.sql`.

**Commit.** `docs(site): materializations guide — cumulative_aggregate`

---

## Post-implementation verification

After Phase 7:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets` — warning-free.
- `cargo test` — full workspace clean.
- `cargo test -p smelt-cli --test example_diagnostics` — green.
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo test -p smelt-cli --test cumulative_equivalence` (new; from Phase 5) — green.
- Manual smoke: `cd examples/web_analytics && cargo run -p smelt-cli -- run --event-time-start 2024-01-01 --event-time-end 2024-01-08` builds the project end-to-end (or whatever the example's canonical run range is — confirm against `examples/web_analytics/README.md`).

## Open questions during execution

These are spec-resolved but worth flagging if real implementation surprises:

- **Where exactly does source-filter pushdown live for cumulative?** The incremental rule's pushdown happens in `crates/smelt-cli/src/transformer.rs::inject_time_filter`. Cumulative's pushdown is similar but per-partition (different range every iteration). Either generalise the existing helper to accept arbitrary `(start, end)` and call it from the cumulative loop, or write a dedicated cumulative pushdown helper. Phase 3 will pick the cleanest cut.
- **Reprocessing detection without a watermark store.** The spec says refuse reprocessing; the implementation needs a heuristic. The conservative shape: refuse if the target table exists and any partition in the run window has already been merged, which can be inferred from `SELECT EXISTS (SELECT 1 FROM target WHERE <unique_key tuple> IS NOT NULL)` — but that's not precise enough. A precise check needs a per-model "max processed partition" hint; until a state-tracking spec lands (`incremental_models.md` Known Divergences), the heuristic is "refuse if target table exists and run window's start is ≤ now() - some-cutoff" or simply "always refuse if target exists, force `--full-refresh` opt-in". The Phase 3 commit message should call out which heuristic was picked.
- **Multi-partition first run.** When the target table does not exist and the run window covers multiple partitions, does the first partition `CREATE TABLE AS` and the rest `MERGE INTO`, or does the whole run create the table from the union? Phase 3 picks first-partition-creates; the rest merge in.

## References

- **Spec**: [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) — normative oracle.
- **Adjacent specs**: [`docs/specs/incremental_models.md`](../specs/incremental_models.md), [`docs/specs/models.md`](../specs/models.md), [`docs/specs/timeseries.md`](../specs/timeseries.md).
- **Research**:
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — rule-shape rationale.
  - [`docs/research/20260521-incremental-as-planner-rule.md`](../research/20260521-incremental-as-planner-rule.md) — sibling research; "derive from SQL" principle.
  - [`docs/research/2026-05-20-incremental-gaps-from-web-analytics.md`](../research/2026-05-20-incremental-gaps-from-web-analytics.md) — Gap #5 closes here.
- **Predecessor plan**: [`docs/plans/20260521-incremental-timeseries-and-derived-bounds.md`](20260521-incremental-timeseries-and-derived-bounds.md) — lookback-from-SQL precedent.
