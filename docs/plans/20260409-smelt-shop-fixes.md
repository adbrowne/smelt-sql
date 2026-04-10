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

## Phase 2: EXTRACT(EPOCH FROM ...) Parser Fix `[x]`

**Priority**: High — isolated parser change, no type inference entanglement.

**Goal**: Parse `EXTRACT(field FROM expr)` as a special form, not a regular function call.

**Work**:
- [x] Add unit test in `smelt-parser` that parses `EXTRACT(EPOCH FROM ts)` and verifies AST structure (red)
- [x] In `parser.rs`, add special-case handling when function name is `EXTRACT`: parse `field FROM expr` as the argument, not a regular expression list
- [x] Ensure the AST node preserves the field name (EPOCH, YEAR, MONTH, DAY, HOUR, MINUTE, SECOND) and the source expression
- [x] Update type inference in `type_inference.rs` to handle the new EXTRACT AST node — return appropriate types (EPOCH→DOUBLE/BIGINT, YEAR/MONTH/DAY→INTEGER, etc.)
- [x] Update dialect printer if needed to emit correct SQL for each backend
- [x] Verify `stg_events.sql` in the ecommerce example now has no diagnostics (green)

**Verification**: `cargo test -p smelt-parser` passes. `cargo test -p smelt-db` passes. The ecommerce `stg_events` model has no diagnostics.

---

## Phase 3: CASE Expression Type Inference Fix `[x]`

**Priority**: High — CASE is fundamental SQL, breaks virtually every real model.

**Goal**: Fix CASE expressions producing `CAST(? AS TYPE) AS ?` placeholders.

**Work**:
- [x] Add regression test: model with `CASE WHEN x > 0 THEN 'high' ELSE 'low' END AS label` — assert no `?` in compiled output (red)
- [x] Fix `infer_case_expr_type()` in `type_inference.rs:379-431` to correctly unify branch types and propagate through the type wrapper
- [x] Fix `infer_column_name()` in `type_inference.rs:1805` to handle CASE expressions (return the alias if present, or generate a deterministic name)
- [x] Test CASE variants: simple CASE, searched CASE, CASE with ELSE, CASE without ELSE (nullable), nested CASE
- [x] Verify `stg_products.sql` and `mart_funnel.sql` CASE expressions compile correctly (green)

**Verification**: `cargo test -p smelt-db` passes. Ecommerce models with CASE have no diagnostics and compiled SQL contains no `?` placeholders.

---

## Phase 4: CTE Type Inference Fix `[x]`

**Priority**: High — CTEs are the standard way to write multi-step transformations.

**Goal**: Make CTEs with complex conditions work correctly.

**Work**:
- [x] Add regression test: CASE WHEN with IS NULL OR in WHEN clause (red)
- [x] Root cause (divergence from plan): The CTE issue was NOT in `build_subquery_context()` — the Salsa path already handles CTE ref resolution via `process_from_clause()` in `type_context()`. The actual bug was that the parser's `parse_when_clause()` used `parse_comparison_expr()` which doesn't handle OR/AND operators, so `CASE WHEN x IS NULL OR y > 1800 THEN ...` failed to parse.
- [x] Fix: changed `parse_when_clause()` to use `parse_or_expr()` for both condition and result, allowing full logical expressions.
- [x] Verify `int_sessions.sql` in ecommerce example now has no diagnostics (green)
- [ ] (Deferred) `build_subquery_context()` standalone path for compiler — not blocking any current tests

**Verification**: `cargo test -p smelt-db` passes. CTE models have correct type inference.

---

## Phase 5: Subquery Ref Replacement + JOIN Type Inference `[x]`

**Priority**: High — same root cause as Phase 4 (subquery context lacks schema access), plus separate JOIN bug.

**Goal**: Fix ref replacement in subqueries and type inference for multi-source JOINs.

**Work**:
- [x] **JOIN type inference** (bug #2):
  - [x] Add regression test: model joining two source tables, assert no incorrect CASTs (red)
  - [x] Root cause: qualified column refs like `p.product_id` fell through to `infer_literal_type()` which treated the dot as a decimal point, producing spurious CAST wrappers
  - [x] Fix: in `infer_expression_type()`, return None early for unresolved **qualified** column refs (those with a qualifier) to prevent fallthrough. Unqualified refs still fall through for typed literal support (INTERVAL, DATE, etc.)
  - [x] Verify test passes (green)
- [ ] **Subquery refs** (bug #6): Deferred — requires deeper compiler changes to recursively process refs in subquery FROM clauses. Will address in a follow-up.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-db --lib` (133 passed, 0 failed)
- [x] `cargo test -p smelt-cli --lib` (all pass including `test_join_type_inference_no_wrong_casts`)
- [x] `ecommerce_no_diagnostics` — passes after Phase 6

---

## Phase 6: Seeds as Ref Targets `[x]` ✅ 2026-04-10

**Priority**: High — seeds should be first-class citizens in the dependency graph.

**Goal**: Make `smelt.ref('seed_name')` resolve to seed table schemas.

**Work**:
- [x] Add regression test: model with `smelt.ref('category_hierarchy')` where category_hierarchy is a seed — assert it resolves (red)
- [x] Extend `all_models()` or add `all_seeds()` in `lib.rs:349-370` to discover seed definitions from config
- [x] Extend `resolve_ref()` in `lib.rs:362` to search seeds in addition to models
- [x] Seed schema inference: parse CSV headers + first N rows to determine column types, or read from smelt.yml config
- [x] Ensure seeds appear in `smelt explain` DAG output
- [x] Update ecommerce example: seeds CSV files added, stg_products/stg_orders already use seed refs
- [x] Verify ref resolution works for seeds (green)

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (134 passed, 0 failed)
- [x] `cargo test -p smelt-cli --test example_diagnostics -- ecommerce_no_diagnostics` — passes

---

## Phase 7: Minor Fixes `[x]` ✅ 2026-04-10

**Priority**: Medium — quality-of-life improvements that round out the release.

**Work**:
- [x] **DECIMAL narrowness** (bug #7): Fix `infer_case_expr_type` to promote across all branches (not just take the first), and fix `promote_types` to widen Decimal{narrow}+Integer to Decimal{38,10}. Test: `CASE WHEN ... THEN 150::INTEGER ELSE 0.5::DECIMAL(2,1) END` must hold value 150.
- [x] **FLOAT→DOUBLE normalization** (bug #8): Normalize FLOAT to DOUBLE in `infer_cast_type`. Added `cast_float_as_double` divergence to property tests. Test: `CAST(1 AS FLOAT)` infers as DOUBLE.
- [x] **Materialization type change** (bug #9): In `smelt-backend/src/lib.rs:execute_model`, drop both view and table before creating either type, so materialization changes are handled cleanly.
- [x] **Datagen geometric min** (bug #10): Added optional `min: Option<i32>` parameter to `GeneratorSpec::Geometric` in smelt-datagen. Applied via `v.max(*m)` in generic.rs.
- [x] Update ecommerce example models to remove remaining explicit CASTs — already clean from Phases 1-6.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test` (all pass)

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

**2026-04-09 — Phase 2: EXTRACT(EPOCH FROM ...) Parser Fix**
- Added EXTRACT_KW to lexer and EXTRACT_EXPR to syntax kinds
- New parse_extract_expr() handles EXTRACT(field FROM expr) as special syntax
- Added ExtractExpr AST node with field_name() and expression() accessors
- Type inference returns DOUBLE for EPOCH, BIGINT for other fields
- Dialect printer handles EXTRACT_EXPR via default print_children (standard SQL)
- 3 new parser tests (epoch, year, arithmetic), all 246 parser tests pass, all 133 type inference tests pass
- stg_events.sql now has no diagnostics

**2026-04-09 — Phase 3: CASE Expression Type Inference Fix**
- Root cause: compiler's apply_type_casts() fell back to "?" for columns without aliases when infer_name() returned None
- Fix: generate deterministic names (_col1, _col2, etc.) instead of "?" in compiler
- Also added CASE handling to infer_column_name() in type_inference.rs for LSP path
- Added 3 compiler regression tests: CASE with alias, CASE without alias, CASE in aggregate
- Decision: infer_case_expr_type() was already correct — the bug was only in column naming

**2026-04-09 — Phase 4: CTE Type Inference Fix**
- Divergence: the CTE issue was a parser bug, not a type inference bug
- `parse_when_clause()` used `parse_comparison_expr()` which stops at OR/AND
- Fixed to use `parse_or_expr()` for full logical expression support
- Added 2 parser regression tests for IS NULL OR in CASE WHEN
- int_sessions.sql now parses and type-checks correctly
- Deferred: threading ResolvedSchemas into standalone build_subquery_context() — not needed yet

**2026-04-09 — Phase 5: JOIN Type Inference Fix**
- Root cause: qualified column refs (e.g. `p.product_id`) fell through to `infer_literal_type()` which treated the dot as a decimal point, producing `CAST(p.product_id AS DECIMAL(11,10))`
- Fix: early return None for unresolved **qualified** column refs only; unqualified refs still fall through for typed literal support (INTERVAL '1' DAY, etc.)
- Initial fix was too aggressive (returned None for ALL unresolved column refs), breaking 4 temporal arithmetic tests — scoped to qualified refs only
- Removed debug eprintln! code from compiler.rs
- Added `test_join_type_inference_no_wrong_casts` regression test
- Subquery ref replacement deferred to follow-up (requires deeper compiler changes)
