# Plan: Maintenance-Statement Emission Unification

**Date**: 2026-07-10
**Spec**: [`docs/specs/maintenance_plan.md`](../specs/maintenance_plan.md) §"Statement emission (single owner)", [`docs/specs/cli.md`](../specs/cli.md) §"`smelt explain <model>` maintenance-plan report" (`--show-sql`), [`docs/specs/architecture.md`](../specs/architecture.md) §"Constraints & Invariants" item 12
**Spec diff**: uncommitted working tree (2026-07-10 review session; committed together with this plan)
**Tracking PR / branch**: `worktree-incremental`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/maintenance_plan.md` §"Statement emission (single owner)" and `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-incremental`. If not, ask the user before continuing.
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
- Verification gate is `bash .claude/scripts/verify-phase.sh` (needs `DUCKDB_LIB_DIR`, see root `CLAUDE.md`).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (Run pipeline parity, Maintenance-plan purity, Fail-loud discipline).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*.

---

## Context

The 2026-07-10 MP-series review found the maintenance-SQL layer has two owners: the pure emitters in `crates/smelt-logical/src/maintenance/emit.rs` are called only from tests, while production runs author their statements in the backends (`delete_and_insert_transactional`, `merge_into`) and `smelt-runtime` (`build_cumulative_merge_sql`, the driver's `CREATE TABLE … AS`). The conformance suite therefore proves the wrong copy equivalent to full refresh, and no CLI surface can print the SQL a run would execute. The spec now makes single ownership normative (`maintenance_plan.md` §"Statement emission (single owner)") and specifies the observation surface (`cli.md` `--show-sql`). This plan drives code to match. Investigation notes (per-family deltas between emitter and production text) are in `docs/plans/20260710-web-analytics-maintenance-demo.md` §6 and the doc comments this plan touches.

## Scope

### In scope (spec coverage)
- `maintenance_plan.md` §"Statement emission (single owner)": statement data model, per-family unification (region DELETE+INSERT, keyed fold MERGE + first-run CREATE, column-scoped MERGE), backends as executors.
- `maintenance_plan.md` §Constraints "Maintenance statements have one author" + `architecture.md` item 12: the statement-parity standing gate (conformance HOLDS legs diff against `execute_project`).
- `cli.md` §"`smelt explain <model>` maintenance-plan report": `--show-sql`, `--period`, and the `--json --show-sql` statements array.

### Explicitly deferred
- Ledger DDL/DML stays in `smelt-state` (spec-sanctioned exclusion — bookkeeping, not a maintenance statement).
- `emit_in_place_update`: no live plan cell lowers to it; it stays a spec'd emitter with no production consumer (doc-comment classified). The schema-evolution backfill `UPDATE … FROM` (`smelt-runtime/src/backfill.rs`) is a separate surface, untouched.
- MP17 `KNOWN_GAPS` matrix cells — bookkeeping only; no new cells are claimed here beyond what statement parity itself grounds.
- Spark statement parity is asserted at the emitter level (dialect-keyed unit tests); the live Spark leg runs only under the gated `spark-parity` CI job, not `verify-phase.sh`.
- `smelt run --dry-run` printing maintenance SQL (the spec chose `smelt explain --show-sql`; extending `--dry-run` can be revisited after the demo consumes `--show-sql`).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |

---

### Phase 1: Statement data model + region-recompute family

**Goal.** Introduce `MaintenanceStatement`/`StatementGroup` (ordered statements + `transactional: bool`) and `MaintenanceDialect` in `smelt-logical::maintenance::emit`; make the region DELETE+INSERT pair single-owner: `emit_delete_insert` produces byte-identical text to what production executes, and the DuckDB/Spark backends execute the emitted group instead of authoring their own strings.

**Pre-conditions.** None (first phase).

**TDD tests to write first.**
- `crates/smelt-logical/tests/emit_statements.rs::delete_insert_group_is_transactional_and_matches_production_shape` — emitter output for a region on a quoted-literal boundary: two statements, `transactional == true`, DELETE text `DELETE FROM <t> WHERE <col> >= '<s>' AND <col> < '<e>'` with `''`-escaped literals, INSERT text `INSERT INTO <t> <body>` (no redundant outer `WHERE` — the body already carries the output clamp, `model_transforms.md` §"the two clamps").
- `crates/smelt-runtime/tests/statement_parity.rs::region_recompute_statements_come_from_the_emitter` — run `examples/timeseries` (or the harness fixture) through `execute_project` with a recording backend/reporter; every executed DELETE/INSERT for an incremental batch is byte-identical to `emit_delete_insert` called with that batch's plan-cell data.
- Existing `crates/smelt-backend-duckdb` rollback test (`a failed INSERT must roll back the paired DELETE`) stays green against the new execution path.

**Implementation shape.** `emit.rs`: `pub struct MaintenanceStatement { pub sql: String }`, `pub struct StatementGroup { pub statements: Vec<MaintenanceStatement>, pub transactional: bool }`, `pub enum MaintenanceDialect { DuckDb, Spark }`; rewrite `emit_delete_insert(table, partition_col, region, body, dialect) -> StatementGroup` mirroring production text (escaping included). `smelt-backend`: add `execute_statement_group(&self, group) -> Result<…>` (generic: one transaction when `transactional`, else sequential); `delete_and_insert_transactional`'s internal string-building is deleted and its callers (trait-default `execute_model_incremental` DeleteInsert arm) route the emitted group through. Runtime (`execute.rs` batch loop) builds the group via the emitter and passes it down. `RunReporter` gains a default-no-op `maintenance_statements(&run_id, model, &StatementGroup)` hook so tests (and later `--show-sql` docs generation) can observe.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/emit.rs` — data model + region emitter rewrite; module doc drops "v0 tracer bullet"
- `crates/smelt-backend/src/lib.rs` — `execute_statement_group`, DeleteInsert arm routing
- `crates/smelt-backend-duckdb/src/lib.rs`, `crates/smelt-backend-spark/src/{lib,sql}.rs` — execute, don't author
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-runtime/src/reporter.rs` — emit + report
- `crates/smelt-logical/tests/`, `crates/smelt-runtime/tests/` — new tests; tracer tests updated to the new signature

**Docs touched.**
- `docs/specs/maintenance_plan.md` — §Known Divergences "Statement emission is not yet single-owner": narrow to the families still un-unified

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Emitted text byte-matches pre-phase production text (no silent SQL behavior change) — verified by the parity test, not by eye
- [ ] Transactionality preserved (rollback test)
- [ ] No scope creep into keyed/column-scoped families
- [ ] Spec Known Divergences updated timelessly

**Commit.** `refactor(maintenance): region DELETE+INSERT statements come from the smelt-logical emitter; backends execute, never author`

### Phase 2: Keyed fold family (combiner-aware MERGE + first-run CREATE)

**Goal.** `emit_keyed_fold` becomes the single author of the keyed MERGE — combiner-aware (rendered combine expressions passed as plain data), `INSERT *` form — plus a new `emit_create_table_as` for the driver's first-run statement; `build_cumulative_merge_sql` and the driver's inline `format!` are deleted or reduced to thin emitter calls.

**Pre-conditions.** Phase 1 (`StatementGroup`, reporter hook, parity-test harness).

**TDD tests to write first.**
- `crates/smelt-logical/tests/emit_statements.rs::keyed_fold_renders_combiners_and_insert_star` — emitter output matches the current production shape (`MERGE INTO <schema>.<table> AS target USING (<delta>) AS delta ON … WHEN MATCHED THEN UPDATE SET c = target.c + delta.c / LEAST(...) / GREATEST(...) WHEN NOT MATCHED THEN INSERT *`), with combiner expressions supplied pre-rendered.
- `crates/smelt-runtime/tests/statement_parity.rs::keyed_fold_statements_come_from_the_emitter` — `examples/web_analytics` `device_user_edges` (grain: key) through `execute_project` with the recording reporter: first-run `CREATE TABLE … AS` and every step's MERGE are byte-identical to the emitters' output.
- Existing `cumulative.rs::test_build_cumulative_merge_sql` migrates to assert through the emitter.

**Implementation shape.** `emit_keyed_fold(schema_table, key, folds: &[(col, rendered_combine_expr)], delta_select, dialect) -> StatementGroup`; `WindowedKeyedRule::merge_sql` impls call it (callers render `CrossPartitionCombiner` to expression strings first — `smelt-logical` stays below `smelt-planner`); `run_windowed_keyed_maintenance`'s CREATE arm calls `emit_create_table_as`. Ledger interleaving (`fold_ledger_delta`) unchanged — it wraps the emitted action statement exactly as today.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/emit.rs`
- `crates/smelt-runtime/src/cumulative.rs`, `crates/smelt-runtime/src/maintenance_driver.rs`
- test files as above

**Docs touched.**
- `docs/specs/maintenance_plan.md` — Known Divergences narrowed further

**Review checklist** (material findings only):
- [ ] TDD tests exist and assert byte-parity
- [ ] Layering holds: `smelt-logical` gains no `smelt-planner`/`smelt-backend` dependency (`cargo tree` spot-check)
- [ ] Never-fold-twice wiring untouched (driver ledger tests green)
- [ ] No scope creep into column-scoped family

**Commit.** `refactor(maintenance): keyed fold MERGE + first-run CREATE emitted by smelt-logical; retire build_cumulative_merge_sql`

### Phase 3: Column-scoped MERGE family

**Goal.** The column-scoped MERGE text (today authored per-backend in `merge_into`) is emitted by `emit_column_scoped_merge`, rewritten to the real production shape as dialect-keyed variants (DuckDB: `UPDATE SET *` requiring full-row source projection; Spark per its `merge_into`); backends execute the emitted statement.

**Pre-conditions.** Phases 1–2.

**TDD tests to write first.**
- `crates/smelt-logical/tests/emit_statements.rs::column_scoped_merge_duckdb_uses_set_star_full_row_projection` — DuckDB-dialect output matches production `MERGE INTO <t> AS target USING (<src>) AS source ON <key eq> WHEN MATCHED THEN UPDATE SET * WHEN NOT MATCHED THEN INSERT *`; Spark-dialect variant asserted from `smelt-backend-spark`'s current text.
- `crates/smelt-runtime/tests/statement_parity.rs::column_scoped_merge_statements_come_from_the_emitter` — the existing `technique_lowering.rs::column_scoped_merge_e2e` fixture (`examples/timeseries/models/daily_events_enriched.sql`) re-run with the recording reporter; the executed MERGE byte-matches the emitter.

**Implementation shape.** `emit_column_scoped_merge(table, unique_key, source_select, dialect) -> StatementGroup` (the old column-list `SET` form is deleted — it never matched production; `dimension_horizon_merge` keeps building the clamped source SELECT and feeds it in). `Backend::merge_into` implementations stop formatting SQL; either the trait method takes the emitted statement or the runtime calls `execute_statement_group` directly and `merge_into` is removed — pick whichever keeps `Backend` smallest.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/emit.rs`
- `crates/smelt-backend/src/lib.rs`, `crates/smelt-backend-duckdb/src/lib.rs`, `crates/smelt-backend-spark/src/lib.rs`
- `crates/smelt-runtime/src/maintenance_driver.rs`
- test files as above

**Docs touched.**
- `docs/specs/maintenance_plan.md` — Known Divergences narrowed

**Review checklist** (material findings only):
- [ ] TDD tests exist; DuckDB e2e fixture still green
- [ ] Full-row-projection contract documented on the emitter (moved from the backend doc comment)
- [ ] Spark variant matches `smelt-backend-spark`'s pre-phase text (unit-level; live leg deferred to gated CI)

**Commit.** `refactor(maintenance): column-scoped MERGE emitted by smelt-logical as dialect-keyed variants`

### Phase 4: Conformance gate upgrade + structural gate

**Goal.** The standing conformance gate proves *production* ≡ full refresh: the HOLDS legs in `maintenance_plan_conformance.rs` drive `execute_project` (recording reporter) instead of calling emitters directly; the stale header (lines 11–42, "execute_project does not yet consult derive_maintenance_plan") is rewritten; a structural test asserts no maintenance-statement authoring survives outside `emit.rs`.

**Pre-conditions.** Phases 1–3 (all families emitted).

**TDD tests to write first.**
- Upgrade `crates/smelt-logical/tests/maintenance_plan_conformance.rs::described_technique_matches_execution_*` (3 legs) — each leg's executed statements now come from a real `execute_project` run over its fixture and are asserted (a) byte-equal to the emitters' output for the derived cell and (b) result-equivalent to full refresh (the existing oracle). Note: this may need the legs to move to (or be driven from) `crates/smelt-runtime/tests/` since `smelt-logical` cannot depend on `smelt-runtime`; the matrix bookkeeping meta-test stays where it is.
- `crates/smelt-runtime/tests/statement_parity.rs::no_maintenance_statement_authoring_outside_the_emitter` — structural: `rg`-over-sources assertion (same style as `hardening_budget`) that `DELETE FROM`/`MERGE INTO`/`CREATE TABLE {}.{} AS`-shaped `format!` construction is absent from `smelt-backend*/src` and `smelt-runtime/src` production code (allowlist: `emit.rs`, `smelt-state` ledger/schema-evolution DDL, `backfill.rs`).

**Implementation shape.** Recording `RunReporter` test util (shared via `smelt-maintenance-testkit` if that's where the harness lives); conformance header rewrite; `CLAIMED` entries for the cells the parity legs now actually ground (additive-only change to the matrix).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/tests/maintenance_plan_conformance.rs`, `crates/smelt-maintenance-testkit/`
- `crates/smelt-runtime/tests/statement_parity.rs`
- `docs/specs/architecture.md` — Known Divergences: statement-emission half of the purity gate flips to "gated"

**Review checklist** (material findings only):
- [ ] HOLDS legs demonstrably execute through `execute_project` (not emitters directly)
- [ ] Structural gate fails when a `format!("MERGE INTO …")` is reintroduced in a backend (verified red once)
- [ ] Matrix stays additive-only (`coverage_matrix_is_inhabited` green)

**Commit.** `test(maintenance): conformance HOLDS legs diff execute_project statements; structural no-authoring gate`

### Phase 5: `smelt explain <model> --show-sql`

**Goal.** Implement the observation surface per `cli.md`: `--show-sql` (with `--period` literals or `{{window_start}}`/`{{window_end}}` placeholders, never connecting to a backend) and `--json` (honored with `--show-sql`) with per-cell `statements`.

**Pre-conditions.** Phases 1–3 (emitters are the single author, so printing them is truthful).

**TDD tests to write first.**
- `crates/smelt-cli/tests/explain_show_sql.rs::show_sql_prints_emitted_statements_per_cell` — against `examples/timeseries`: each cell's block contains the emitter-produced statements; transactional groups bracketed `BEGIN`/`COMMIT`; no backend connection attempted (runs without a database).
- `crates/smelt-cli/tests/explain_show_sql.rs::period_substitutes_real_literals_placeholders_otherwise` — with `--period 2024-01-01..2024-01-03` literals appear; without, `{{window_start}}`.
- `crates/smelt-cli/tests/explain_show_sql.rs::json_format_carries_statements_array` — `--json --show-sql` output parses; `statements[].sql` byte-matches text mode's statements; `transactional_group` indices correct.
- `crates/smelt-cli/tests/explain_show_sql.rs::web_analytics_keyed_model_shows_merge` — real-fixture: `examples/web_analytics` `device_user_edges` shows the keyed fold MERGE.

**Implementation shape.** `explain.rs::build_maintenance_plan_report` grows a statements section fed by the same emitters, with a `RegionLiterals::Period(a, b) | Placeholders` input; CLI flags `--show-sql`, `--period` on the explain subcommand (reusing the existing `--json` flag). SELECT bodies compile through the existing `compile_with_sql_and_ephemerals` path (Run-pipeline-parity: no new compile helper in `smelt-cli`).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/commands/explain.rs`, `crates/smelt-cli/src/explain.rs`, `crates/smelt-cli/src/main.rs`
- `crates/smelt-cli/tests/explain_show_sql.rs`

**Docs touched.**
- `docs/specs/cli.md` — Known Divergences: `--show-sql` entry removed (landed)
- `docs-site/docs/reference/cli.md` — `--show-sql`/`--period`/`--json` on the explain section, with a sample
- `docs-site/docs/reference/smelt-explain.md` — per-model section mentions `--show-sql`

**Review checklist** (material findings only):
- [ ] Printed SQL is the emitters' output (asserted, not re-formatted)
- [ ] No backend connection in `--show-sql` path
- [ ] Run-pipeline-parity honored (no compile logic added to `smelt-cli`)
- [ ] User docs updated; timeless

**Commit.** `feat(cli): smelt explain <model> --show-sql — print the emitted maintenance statements per cell`

### Phase 6: Divergence cleanup + validate sweep

**Goal.** Flip the remaining Known-Divergences entries this plan resolved (`maintenance_plan.md` "Statement emission is not yet single-owner" reduced to whatever genuinely remains, e.g. `emit_in_place_update`'s consumer-less status; `architecture.md`; `cli.md`), run `/smelt:validate maintenance_plan` and `/smelt:validate cli`, and fix any drift they report.

**Pre-conditions.** Phases 1–5.

**TDD tests to write first.** None (docs-only phase); the gate is the validate sweep + `verify-phase.sh`.

**Critical files (allowed to touch in this phase).**
- `docs/specs/maintenance_plan.md`, `docs/specs/architecture.md`, `docs/specs/cli.md` — Known Divergences only

**Review checklist** (material findings only):
- [ ] Every divergence claim matches HEAD behavior (spot-check against code, not memory)
- [ ] Timeless-oracle rule holds

**Commit.** `docs(specs): flip statement-emission divergences resolved by the emit unification`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-runtime --test statement_parity` — every family's executed statements byte-match the emitters
- `cargo test -p smelt-logical --test maintenance_plan_conformance` — HOLDS legs prove production ≡ full refresh
- `smelt explain daily_events_enriched --show-sql` in `examples/timeseries` prints the column-scoped MERGE a run executes
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate maintenance_plan` and `/smelt:validate cli` report zero drift
