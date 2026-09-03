# Phase 30 summary — backbuild family joins the `statement_parity` gate

**Shipped:**
- Three new tests in `crates/smelt-runtime/tests/statement_parity.rs` covering the backbuild
  emitter family (`crates/smelt-logical/src/backbuild/emit.rs`), driven directly through
  `smelt_runtime::definition_delta::{derive_plan, apply_migration}`:
  - `backbuild_in_place_backfill_statements_come_from_the_emitter` — B1 (`SelfDerivedColumnAdd`):
    `ALTER TABLE ADD` + in-place `UPDATE`.
  - `backbuild_full_refresh_statement_comes_from_the_emitter` — a grain change yields
    `MigrationVerdict::SkeletonChange`; the hand-built full-refresh plan's statement is
    byte-identical to `emit_full_refresh`.
  - `backbuild_upstream_backfill_statements_come_from_the_emitter` — B3 (`UpstreamPullthrough`):
    `ALTER TABLE ADD` + `UPDATE ... FROM` against a declared-`unique_key` upstream.
  - All three carry both legs: byte-identical executed-vs-direct-emitter-call, and result
    multiset-equal to a full refresh (same shape as the maintenance-family tests).
- A shared `stage_and_migrate` helper: stages a project, deploys v1 via a real `execute_project`
  run, rewrites the target model to v2, re-derives the plan via `derive_plan`.
- Widened `no_maintenance_statement_authoring_outside_the_emitter`'s forbidden-shape list with
  `"ALTER TABLE "`, `"CREATE OR REPLACE TABLE "`, and `"__backbuild_diff"` (the E2/E4
  difference-insert alias marker) — each justified inline for why it has no legitimate
  production match in the scanned crates.
- Removed the narrowed `architecture.md` Known Divergences bullet ("Backbuild's
  executed-statement parity leg is proven end to end but not yet...") and extended item 12's
  Standing CI gate sentence to name the backbuild families and the new forbidden shapes.

**Decisions:**
- B3's fixture needs the upstream's key column pull-through present as its own SELECT-list
  column (e.g. `customers.customer_id AS customers_customer_id`) — the grain-link proof binds a
  key column to a 1:1 *stored* representative under the exact same FROM-tree alias, not merely
  to the join predicate.
- `SELECT *` on a VALUES-derived model defeats `infer_deployed_columns` (returns zero columns,
  so schema tracking silently skips saving that model's deployed schema at all) — fixtures for
  any test relying on an upstream's declared/inferred NOT NULL facts must use an explicit column
  list, never `SELECT *`.
- Test 2 constructs a hand-built `MigrationPlan` (statements = `full_refresh.statements`) rather
  than applying `derived.plan` directly, since a `SkeletonChange` verdict's `plan.statements` is
  empty (`assemble` never offers a partial script) — falling back to full refresh is the
  caller's decision, mirrored from `apply_migration_executes_plan_statements_in_order`
  (`definition_delta.rs`)'s own hand-built-plan pattern.

**For the next planner:**
- Confirmed real gap for phase 30b (not investigated further here, per the plan's explicit
  scope note): `smelt-state/src/ddl_duckdb.rs` builds its own `ALTER TABLE ... ADD/DROP COLUMN`
  text for schema-evolution DDL, a second author beside `backbuild::emit`'s
  `emit_alter_add_column`/`emit_alter_drop_column`. Not scanned by this phase's gate
  (`smelt-state` deliberately excluded from the crate list) and not touched.
- D2/B5/B6/E1/E2/E4/F1/F2/C1 techniques remain unexercised by `statement_parity` — only B1/B3/
  FullRefresh got real-fixture parity tests this phase, matching the plan's exact test list. If
  a future phase wants broader backbuild statement-parity coverage, those techniques are the gap.

**Gates:**
- `cargo test -p smelt-runtime --test statement_parity` — 32 passed (29 pre-existing + 3 new).
- `cargo test -p smelt-cli --test migrate_apply` — 9 passed, unaffected.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
- `rg -n "statement_parity structural gate itself|not yet by the .statement_parity" docs/specs` — no hits.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  `cargo test`, `example_diagnostics`).
