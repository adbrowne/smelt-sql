# Plan: Type Inference, Parser & Ref Resolution Fixes

**Date**: 2026-04-09
**Source**: smelt_shop validation report (`~/smelt_shop/smelt_report.md`)
**Branch**: `smelt-shop-fixes`

## Context

A real-world 19-model ecommerce analytics pipeline (smelt_shop) was built against smelt-sql 0.2.0, exposing 6 critical/major bugs and 5 minor issues. The models had to use verbose workarounds (explicit CASTs on every column, avoiding CASE, splitting CTEs into separate models, using `EPOCH()` instead of `EXTRACT(EPOCH FROM ...)`). These issues block real-world adoption.

The bugs also revealed testing strategy gaps: our example workspaces are too simple, we only check LSP diagnostics (not whether compiled SQL actually executes), and property tests only cover expressions (not full models).

This plan fixes all issues and closes the testing gaps in 9 phases. Each phase produces exactly one commit on the `smelt-shop-fixes` branch.

## How to Execute This Plan

Each phase should be implemented by a **sub-agent** (using the Agent tool). Phases are sequential — each depends on prior phases. The orchestrator should:

1. Create the branch and PR before starting Phase 1.
2. For each phase, spawn a sub-agent with the phase instructions below plus the execution rules.
3. After each sub-agent completes, verify the commit landed and tests pass before proceeding.
4. If a phase is marked `[!]` (blocked), stop and ask the user.

### Sub-Agent Prompt Template

Each sub-agent should receive:

```
You are implementing Phase N of the smelt-shop fixes plan.

PLAN: docs/plans/20260409-smelt-shop-fixes.md
REPORT: ~/smelt_shop/smelt_report.md  
EXAMPLE MODELS: ~/smelt_shop/models/

Read CLAUDE.md for project conventions before starting any work.

<phase-specific instructions from the plan>
```

### Execution Rules (include in every sub-agent prompt)

**Red-green testing:**
- FIRST write the failing tests described in the phase.
- Run the tests to confirm they fail (red) — they should fail for the RIGHT reason (assertion failure or missing feature, NOT a compile error).
- THEN implement the code to make them pass.
- Run the tests to confirm they pass (green).
- If any test still fails, debug and fix until green.

**Quality gates (run before committing):**
- `cargo fmt --all` — format all code
- `cargo clippy --all-targets` — no warnings
- `cargo test` — all tests pass (if too slow, at minimum run the relevant package tests)
- `cargo test -p smelt-cli --test example_diagnostics` — verify examples are clean

**Architectural invariant — pure function rule:**
- Analysis logic must be pure functions in smelt-db. Salsa queries are thin wrappers.
- Do NOT call `db.some_query()` inside analysis logic — pass data in as parameters.
- See CLAUDE.md "Pure Function Rule" section for details.

**Plan updates (REQUIRED in every commit):**
- Check off completed work items with `[x]` in the plan file.
- Update the phase status: `[~]` while in progress, `[x]` when complete, `[!]` if blocked.
- Add a session log entry at the bottom of the plan: date, phase, summary of work, any decisions.
- Document design decisions in the Decisions Log section WITH reasoning (not just what — explain WHY).
- If the phase changes user-facing behavior, update `docs/ROADMAP.md` too.

**Commit format:**
```
Phase N: <short description>

<2-3 sentence summary of changes>

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
```

Produce exactly ONE commit per phase. Include all changes (code + plan + roadmap + docs) in a single commit. Push to `origin/smelt-shop-fixes`.

## Status Key

- `[ ]` — Not started
- `[~]` — In progress
- `[x]` — Complete
- `[!]` — Blocked or needs review

---

## Phase 1: Complex Example Workspace + Regression Scaffold `[x]`

**Priority**: Highest — gives us a failing test suite to drive all subsequent fixes (red-green).

**Goal**: Create `examples/ecommerce/` workspace with models that exercise every broken pattern (JOINs on sources, CASE, CTEs, EXTRACT(EPOCH FROM ...), subqueries with refs, seed refs). These models should represent the *ideal* SQL a user would write without workarounds. Initially they will fail compilation/execution — that's the point.

**Work**:
- [x] Create `examples/ecommerce/smelt.yml` config (DuckDB backend, view/table materialization)
- [x] Create `examples/ecommerce/sources.yml` defining raw source tables
- [x] Create seed CSVs: `examples/ecommerce/seeds/category_hierarchy.csv`, `examples/ecommerce/seeds/order_statuses.csv`
- [x] Create 6 focused models (one per bug), writing the *natural* SQL without workarounds:
  - `models/staging/stg_products.sql` — JOIN source + seed via `smelt.ref('category_hierarchy')`, CASE expression for product_type (bugs #1, #2, #3)
  - `models/staging/stg_orders.sql` — JOIN source + seed via `smelt.ref('order_statuses')` (bugs #1, #2)
  - `models/staging/stg_events.sql` — `EXTRACT(EPOCH FROM timestamp_col)` for gap calculation (bug #4)
  - `models/intermediate/int_sessions.sql` — CTE to compute session boundaries then aggregate (bug #5)
  - `models/intermediate/int_order_enriched.sql` — subquery with `smelt.ref()` in FROM (bug #6)
  - `models/marts/mart_funnel.sql` — CASE in aggregate expressions, division producing DECIMAL (bugs #3, #7)
- [x] Add `ecommerce_no_diagnostics` test to `example_diagnostics.rs` — will initially fail, driving fixes
- [x] Add compile-and-execute integration test in `crates/smelt-cli/tests/ecommerce_execution.rs`:
  - For each model: load workspace → compile to DuckDB SQL via dialect printer → create source tables in DuckDB → execute compiled SQL → assert no errors
  - This test catches code-gen bugs that diagnostics miss (the key testing gap)

**Verification**: `cargo test -p smelt-cli --test example_diagnostics -- ecommerce` fails (expected — bugs not yet fixed). New execution test also fails. Both tests compile and the failure messages are clear.

---

## Phase 2: EXTRACT(EPOCH FROM ...) Parser Fix `[ ]`

**Priority**: High — isolated parser change, no type inference entanglement.

**Goal**: Parse `EXTRACT(field FROM expr)` as a special form, not a regular function call.

**Work**:
- [ ] Add unit test in `smelt-parser` that parses `EXTRACT(EPOCH FROM ts)` and verifies AST structure (red)
- [ ] In `parser.rs`, add special-case handling when function name is `EXTRACT`: parse `field FROM expr` as the argument, not a regular expression list
- [ ] Ensure the AST node preserves the field name (EPOCH, YEAR, MONTH, DAY, HOUR, MINUTE, SECOND) and the source expression
- [ ] Update type inference in `type_inference.rs` to handle the new EXTRACT AST node — return appropriate types (EPOCH→DOUBLE/BIGINT, YEAR/MONTH/DAY→INTEGER, etc.)
- [ ] Update dialect printer if needed to emit correct SQL for each backend
- [ ] Verify `stg_events.sql` in the ecommerce example now has no diagnostics (green)

**Verification**: `cargo test -p smelt-parser` passes. `cargo test -p smelt-db` passes. The ecommerce `stg_events` model has no diagnostics.

---

## Phase 3: CASE Expression Type Inference Fix `[ ]`

**Priority**: High — CASE is fundamental SQL, breaks virtually every real model.

**Goal**: Fix CASE expressions producing `CAST(? AS TYPE) AS ?` placeholders.

**Work**:
- [ ] Add regression test: model with `CASE WHEN x > 0 THEN 'high' ELSE 'low' END AS label` — assert no `?` in compiled output (red)
- [ ] Fix `infer_case_expr_type()` in `type_inference.rs:379-431` to correctly unify branch types and propagate through the type wrapper
- [ ] Fix `infer_column_name()` in `type_inference.rs:1805` to handle CASE expressions (return the alias if present, or generate a deterministic name)
- [ ] Test CASE variants: simple CASE, searched CASE, CASE with ELSE, CASE without ELSE (nullable), nested CASE
- [ ] Verify `stg_products.sql` and `mart_funnel.sql` CASE expressions compile correctly (green)

**Verification**: `cargo test -p smelt-db` passes. Ecommerce models with CASE have no diagnostics and compiled SQL contains no `?` placeholders.

---

## Phase 4: CTE Type Inference Fix `[ ]`

**Priority**: High — CTEs are the standard way to write multi-step transformations.

**Goal**: Make `build_subquery_context()` able to resolve schemas for `smelt.ref()` and `smelt.source()` calls within CTEs.

**Work**:
- [ ] Add regression test: model with `WITH cte AS (SELECT * FROM smelt.ref('upstream')) SELECT col FROM cte` — assert correct type inference (red)
- [ ] Root cause: `build_subquery_context()` in `type_inference.rs:463-497` is a pure function with no access to resolved model schemas
- [ ] Fix approach: thread a `ResolvedSchemas` map (model_name → Vec<(column, type)>) into `build_subquery_context()` so it can look up ref'd model schemas without Salsa access. This preserves the pure-function architecture.
- [ ] The `type_context()` function in `lib.rs:1035-1070` already resolves schemas via Salsa — pass the resolved schemas down into the pure inference functions
- [ ] Handle CTE chains: CTE B references CTE A, which references `smelt.ref()` — types must propagate through the chain
- [ ] Verify `int_sessions.sql` in ecommerce example has no diagnostics (green)

**Verification**: `cargo test -p smelt-db` passes. CTE models have correct type inference.

---

## Phase 5: Subquery Ref Replacement + JOIN Type Inference `[ ]`

**Priority**: High — same root cause as Phase 4 (subquery context lacks schema access), plus separate JOIN bug.

**Goal**: Fix ref replacement in subqueries and type inference for multi-source JOINs.

**Work**:
- [ ] **Subquery refs** (bug #6):
  - [ ] Add regression test: model with `SELECT * FROM (SELECT * FROM smelt.ref('upstream')) sub` — assert ref is replaced in compiled SQL (red)
  - [ ] Fix `process_table_ref()` in `lib.rs:1162-1205` to recursively process refs in subquery FROM clauses
  - [ ] Reuse the `ResolvedSchemas` threading from Phase 4 for subquery type inference
  - [ ] Verify `int_order_enriched.sql` works (green)
- [ ] **JOIN type inference** (bug #2):
  - [ ] Add regression test: model joining two source tables, assert no incorrect CASTs (red)
  - [ ] Fix: when processing JOINs in `process_from_clause()` (lib.rs:1081-1206), ensure qualified column refs (e.g., `p.column`, `ch.column`) resolve to the correct source table's schema, not a mixed/incorrect context
  - [ ] Verify `stg_products.sql` and `stg_orders.sql` compile without explicit CASTs (green)

**Verification**: `cargo test -p smelt-db` and `cargo test -p smelt-cli` pass. Ecommerce staging models with JOINs produce correct SQL.

---

## Phase 6: Seeds as Ref Targets `[ ]`

**Priority**: High — seeds should be first-class citizens in the dependency graph.

**Goal**: Make `smelt.ref('seed_name')` resolve to seed table schemas.

**Work**:
- [ ] Add regression test: model with `smelt.ref('category_hierarchy')` where category_hierarchy is a seed — assert it resolves (red)
- [ ] Extend `all_models()` or add `all_seeds()` in `lib.rs:349-370` to discover seed definitions from config
- [ ] Extend `resolve_ref()` in `lib.rs:362` to search seeds in addition to models
- [ ] Seed schema inference: parse CSV headers + first N rows to determine column types, or read from smelt.yml config
- [ ] Ensure seeds appear in `smelt explain` DAG output
- [ ] Update ecommerce example: `stg_products.sql` uses `smelt.ref('category_hierarchy')` instead of `smelt.source()`
- [ ] Verify ref resolution works for seeds (green)

**Verification**: `cargo test -p smelt-db` passes. `cargo test -p smelt-cli --test example_diagnostics -- ecommerce` passes (all diagnostic tests green).

---

## Phase 7: Minor Fixes `[ ]`

**Priority**: Medium — quality-of-life improvements that round out the release.

**Work**:
- [ ] **DECIMAL narrowness** (bug #7): In `type_inference.rs`, widen default precision for division results. Test: `SELECT price / 100.0` should not overflow for values > 99.
- [ ] **FLOAT→DOUBLE normalization** (bug #8): Normalize FLOAT to DOUBLE in type inference. Test: `CAST(x AS FLOAT)` should infer as DOUBLE.
- [ ] **Materialization type change** (bug #9): In the DuckDB backend, when materializing a model, DROP the existing object regardless of type (view or table) before creating the new one. Test: change a model from view to table without manual intervention.
- [ ] **Datagen geometric min** (bug #10): Add optional `min` parameter to the geometric generator in smelt-datagen. Default to current behavior (0-based). Test: `min: 1` produces no zero values.
- [ ] Update ecommerce example models to remove remaining explicit CASTs that are no longer needed

**Verification**: `cargo test` passes. All ecommerce models compile and execute cleanly.

---

## Phase 8: End-to-End Execution Tests + Packaging `[ ]`

**Priority**: Medium — closes the testing gap and ensures we don't regress.

**Work**:
- [ ] **Compile-and-execute test** (created in Phase 1, should now pass):
  - Verify all ecommerce models compile to valid DuckDB SQL and execute without errors
  - Extend to cover all non-broken example workspaces (timeseries, retail_analytics, etc.)
  - Add this test to the standard `cargo test` run
- [ ] **Model-level property tests**: Add `prop_model_compilation.rs` to `smelt-db/tests/`:
  - Generate full model SQL with JOINs, CTEs, CASE, subqueries
  - Compile via dialect printer → execute against DuckDB → assert no errors
  - Start with 64 cases, scale up once stable
- [ ] **Packaging**:
  - [ ] Add sdist (source distribution) to the release workflow
  - [ ] Add cp314 wheel targets for all platforms (Linux x86_64, Linux ARM64, Windows, macOS ARM64)
  - [ ] Verify `pip install smelt-sql` works from sdist on a platform without a prebuilt wheel
- [ ] Final pass: run full `cargo test`, `cargo clippy --all-targets`, `cargo fmt --all -- --check`
- [ ] Run the ecommerce execution test end-to-end with a fresh DuckDB database

**Verification**: `cargo test` passes (all tests including new execution + property tests). CI release workflow builds sdist + all wheel targets.

---

## Phase 9: User Documentation + Roadmap Update `[ ]`

**Priority**: Required — user-facing changes need documentation before release.

**Goal**: Update the docs-site user documentation and roadmap to reflect all fixes and new capabilities from this plan.

**Work**:
- [ ] **docs-site/docs/guide/seeds.md**: Document that seeds are now first-class `smelt.ref()` targets (no longer need sources.yml workaround)
- [ ] **docs-site/docs/guide/sql-models.md**: Update with examples showing CTEs, CASE expressions, EXTRACT, and subqueries now work correctly in type inference
- [ ] **docs-site/docs/guide/sources.md**: Remove or note deprecation of the seeds-as-sources workaround
- [ ] **docs-site/docs/guide/materializations.md**: Document that view↔table materialization changes are now handled automatically
- [ ] **docs-site/docs/reference/language.md**: Add EXTRACT(field FROM expr) to supported SQL syntax
- [ ] **docs-site/docs/guide/datagen.md**: Document the `min` parameter for the geometric generator
- [ ] **docs-site/docs/getting-started/installation.md**: Update platform/Python version matrix (sdist available, Python 3.14 wheels)
- [ ] **docs/ROADMAP.md**: Mark "Type Inference, Parser & Ref Resolution Fixes" as ✅ complete with date. Mark "Packaging" as ✅ complete. Mark "Testing Strategy Improvements" as ✅ complete. Move all three to Recently Completed section.
- [ ] **examples/ecommerce/README.md**: Add a brief README explaining the ecommerce example workspace and what patterns it demonstrates
- [ ] Review all changes for consistency and accuracy

**Verification**: All doc pages render correctly (no broken links, no stale workaround references). `docs/ROADMAP.md` accurately reflects completion status.

---

## Decisions Log

*(Populated during implementation)*

---

## Session Log

**2026-04-09 — Phase 1: Complex Example Workspace + Regression Scaffold**
- Created `examples/ecommerce/` with smelt.yml, sources.yml, 2 seed CSVs, and 6 models
- Models written as natural SQL without workarounds — each exercises specific bugs
- Added `ecommerce_no_diagnostics` test to example_diagnostics.rs
- Added `ecommerce_execution.rs` compile-and-execute integration test with DuckDB seeding
- Both tests compile and fail with clear error messages showing all 7 bugs manifesting
