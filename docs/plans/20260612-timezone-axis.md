# Plan: Timezone Type-System Axis

**Date**: 2026-06-12
**Spec**: [`docs/specs/types.md`](../specs/types.md) §16
**Spec diff**: commit `7e46aadc` (spec(types): add §16 Timezone)
**Tracking PR / branch**: `worktree-timezone-axis` (to be created)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/types.md` §16 — it is the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-timezone-axis`. If not, ask the user before continuing.
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
- Honor architectural invariants from `CLAUDE.md` (e.g., `type_inference/` purity, Salsa purity rule).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/types.md` and `docs-site/docs/...` describe the feature as if it has always existed — no `### Phase N — …` headings, no `(Phase N)` inline labels in spec/user-doc body.

---

## Context

This plan implements `docs/specs/types.md` §16 (Timezone). The `DataType::Timestamp { with_timezone: bool }` representation already exists; the bug is that several functions hardcode `with_timezone: false` when they should return `true`, and the UNION/CASE LUB silently widens mixed pairs instead of emitting `TypeMismatch`. The plan wires the existing representation correctly: fix the three function return types, make `DATE_TRUNC` tz-preserving, enforce the strict mixing rule, and extend the property-test oracle to cover `TimestampTz` inputs.

## Scope

### In scope (spec coverage)
- §16 Two-variant type: representation already exists; this plan corrects the wiring
- §16 Function returns: `NOW`, `CURRENT_TIMESTAMP` → `TimestampTz`; `MAKE_TIMESTAMPTZ` → `TimestampTz`; `DATE_TRUNC` → tz-preserving from input
- §16 Strict mixing rule: UNION/EXCEPT/INTERSECT/CASE + arithmetic with mixed tz → `TypeMismatch`
- §16 Soundness oracle: add `TimestampTz` to property-test generators
- §16 Surface verification: `TIMESTAMP WITH TIME ZONE` hover display and `smelt.define` signature notation

### Explicitly deferred
- `AT TIME ZONE` expression (parser work; documented as Known Divergence in §16)
- `TIMEZONE(zone, expr)` scalar function (different Spark/DuckDB argument semantics; deferred)
- Collation axis (separate axis, no research doc yet)

---

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     |        | 2026-06-12 |
| 2     | done     |        | 2026-06-12 |
| 3     | done     |        | 2026-06-12 |
| 4     | pending  |        |      |

---

### Phase 1: Timezone-sensitive function return fixes

**Goal.** Correct `NOW`, `CURRENT_TIMESTAMP`, and `MAKE_TIMESTAMPTZ` to return `Timestamp WITH TIME ZONE`; correct `DATE_TRUNC` to preserve the tz-axis of its first `Timestamp` argument.

**Pre-conditions.** None — this is the first code phase.

**TDD tests to write first.** Listed verbatim — write these as failing tests before any implementation:
- `crates/smelt-db/tests/ts_function_returns.rs::now_returns_timestamptz` — assert `SELECT NOW()` infers `Timestamp { with_timezone: true }`, non-nullable.
- `crates/smelt-db/tests/ts_function_returns.rs::current_timestamp_returns_timestamptz` — assert `SELECT CURRENT_TIMESTAMP` infers `Timestamp { with_timezone: true }`, non-nullable.
- `crates/smelt-db/tests/ts_function_returns.rs::make_timestamptz_returns_timestamptz` — assert `SELECT MAKE_TIMESTAMPTZ(2024, 1, 1, 0, 0, 0)` infers `Timestamp { with_timezone: true }`, nullable.
- `crates/smelt-db/tests/ts_function_returns.rs::make_timestamp_returns_naive` — assert `SELECT MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0)` infers `Timestamp { with_timezone: false }` (no regression).
- `crates/smelt-db/tests/ts_function_returns.rs::date_trunc_preserves_naive` — assert `SELECT DATE_TRUNC('day', ts_col)` over a naive `Timestamp` column infers `Timestamp { with_timezone: false }`.
- `crates/smelt-db/tests/ts_function_returns.rs::date_trunc_preserves_timestamptz` — assert `SELECT DATE_TRUNC('day', tstz_col)` over a `Timestamp { with_timezone: true }` column infers `Timestamp { with_timezone: true }`.

**Implementation shape.** All changes are in `crates/smelt-db/src/type_inference/function_call.rs`:
- `SqlFunction::Now | SqlFunction::CurrentTimestamp` arm: flip `with_timezone: false` → `with_timezone: true`.
- `SqlFunction::MakeTimestamptz` arm: separate from `MakeTimestamp`; return `with_timezone: true`.
- `SqlFunction::DateTrunc` arm: inspect the second argument's inferred type via `ctx`; if it is `Timestamp { with_timezone }`, mirror that flag; fall back to `with_timezone: false` if the argument type is unknown/non-timestamp.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/type_inference/function_call.rs` — three arms as above
- `crates/smelt-db/tests/ts_function_returns.rs` — new test file

**Docs touched.**
- `docs/specs/types.md` — §16 function-returns table is already written; no edits needed unless a divergence surfaces.

**Review checklist** (material findings only):
- [ ] All six TDD tests exist and pass after the fix
- [ ] `MAKE_TIMESTAMP` (no TZ) still returns `with_timezone: false` (regression test above)
- [ ] `DATE_TRUNC` with non-timestamp input falls back gracefully (no panic)
- [ ] Spec §16 function-returns table is satisfied for all four functions
- [ ] Architectural invariants honored: `function_call.rs` stays pure (no Salsa calls)
- [ ] No scope creep into Phase 2 (mixing rule)

**Commit.** `fix(types): tz-aware returns for NOW, CURRENT_TIMESTAMP, MAKE_TIMESTAMPTZ, DATE_TRUNC (§16)`

---

### Phase 2: Strict timezone mixing rule

**Goal.** Change the UNION/CASE LUB in `dispatch.rs` and the arithmetic paths in `binary.rs` to emit `TypeMismatch` when a naive `Timestamp` and a `Timestamp WITH TIME ZONE` are mixed, instead of silently widening.

**Pre-conditions.** Phase 1 done — `NOW()` / `CURRENT_TIMESTAMP` now return `TimestampTz`, so the mixing rule is immediately exercised by any query that joins naive and tz-aware columns.

**TDD tests to write first.** Listed verbatim:
- `crates/smelt-db/tests/ts_mixing.rs::union_mixed_tz_is_type_mismatch` — assert a `SELECT ts_col UNION SELECT tstz_col` model produces a `TypeMismatch` diagnostic.
- `crates/smelt-db/tests/ts_mixing.rs::union_same_tz_naive_ok` — assert `SELECT ts1 UNION SELECT ts2` over two naive columns produces no diagnostic (no regression).
- `crates/smelt-db/tests/ts_mixing.rs::union_same_tz_aware_ok` — assert two `TimestampTz` columns UNION cleanly.
- `crates/smelt-db/tests/ts_mixing.rs::arithmetic_mixed_tz_is_type_mismatch` — assert `SELECT tstz_col - ts_col` emits `TypeMismatch`.
- `crates/smelt-db/tests/ts_mixing.rs::case_mixed_tz_is_type_mismatch` — assert a CASE expression whose THEN branch is `Timestamp` and ELSE is `TimestampTz` emits `TypeMismatch`.

**Implementation shape.**
- `crates/smelt-db/src/type_inference/dispatch.rs`, `both_temporal` branch: replace the `(Timestamp{tz1}, Timestamp{tz2})` arm that does `tz1 || tz2` with a branch that, when `tz1 != tz2`, pushes a `TypeMismatch` diagnostic at the set-operator span and returns `Unknown(Unresolved)`.
- `crates/smelt-db/src/type_inference/binary.rs`: add a guard in the `Timestamp`/`Timestamp` arithmetic arms — if both are `Timestamp` and `with_timezone` differs, emit `TypeMismatch` and return `Unknown(Unresolved)`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/type_inference/dispatch.rs` — `both_temporal` LUB arm
- `crates/smelt-db/src/type_inference/binary.rs` — timestamp arithmetic guards
- `crates/smelt-db/tests/ts_mixing.rs` — new test file

**Docs touched.**
- `docs/specs/types.md` — §16 strict mixing rule and arithmetic rule already written; no edits unless a divergence surfaces.

**Review checklist** (material findings only):
- [ ] All five TDD tests exist and pass
- [ ] `TypeMismatch` diagnostic spans the set-operator or arithmetic operator token (not the whole expression)
- [ ] Same-tz UNION and arithmetic produce no spurious diagnostics
- [ ] `Unknown` reason is `Unresolved` (not `Dynamic` or `Propagated`)
- [ ] Spec §16 strict mixing rule is satisfied
- [ ] No scope creep into Phase 3

**Commit.** `fix(types): TypeMismatch for naive+tz Timestamp mixing in UNION/CASE/arithmetic (§16)`

---

### Phase 3: TimestampTz property-test oracle

**Goal.** Add `TimestampTz` as a `BaseType` variant in the property-test generators so the DuckDB oracle exercises tz-aware timestamp columns; register any new divergences.

**Pre-conditions.** Phases 1–2 done — the function return types and mixing rule are correct before the oracle runs, otherwise it will surface pre-existing wrong values as failures.

**TDD tests to write first.** Listed verbatim:
- `crates/smelt-db/tests/ts_oracle.rs::timestamptz_column_infers_correctly` — smoke test: a single-column `TIMESTAMPTZ` input table with `SELECT tstz_col` infers `Timestamp { with_timezone: true }`.
- `crates/smelt-db/tests/ts_oracle.rs::now_oracle_matches_duckdb` — assert `SELECT NOW()` smelt type matches DuckDB's actual return type (the failing test before Phase 1 becomes green here).

**Implementation shape.**
- `crates/smelt-db/tests/prop_helpers/generators.rs`: add `TimestampTz` to the `BaseType` enum; wire a SQL literal `CAST('2024-01-01 12:00:00+00' AS TIMESTAMPTZ)`; add a column name (e.g. `tstz_col`); map to `DataType::Timestamp { with_timezone: true }`.
- Run `cargo test -p smelt-db --test type_property_tests` locally; add any new `ByDesign` divergences to `divergences.rs`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/prop_helpers/generators.rs` — `BaseType::TimestampTz` addition
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — any new ByDesign divergences
- `crates/smelt-db/tests/ts_oracle.rs` — new smoke tests

**Docs touched.**
- `docs/specs/types.md` — §16 soundness oracle gate is already written; annotate any ByDesign divergences found here in Known Divergences if material.

**Review checklist** (material findings only):
- [ ] `BaseType::TimestampTz` added and wired through `to_data_type`, `to_sql_literal`, `to_column_name`
- [ ] `cargo test -p smelt-db --test type_property_tests` green (256 cases minimum)
- [ ] Any new divergences are `ByDesign` or fixed — no silent failures left
- [ ] Smoke tests pass
- [ ] Spec §16 soundness oracle gate satisfied

**Commit.** `feat(types): add TimestampTz to property-test oracle; register ByDesign divergences (§16)`

---

### Phase 4: Hover/signature surface verification + ROADMAP

**Goal.** Verify `TIMESTAMP WITH TIME ZONE` renders correctly in hover output and in `smelt.define` signatures; fix if not. Mark the timezone axis complete in `docs/ROADMAP.md`.

**Pre-conditions.** Phases 1–3 done.

**TDD tests to write first.** Listed verbatim:
- `crates/smelt-db/tests/ts_function_returns.rs::timestamptz_hover_string` — assert `format_smelt_type_hover` (or equivalent) for `DataType::Timestamp { with_timezone: true }` produces `"TIMESTAMP WITH TIME ZONE"`.
- `crates/smelt-db/tests/ts_function_returns.rs::timestamp_hover_string` — assert naive `Timestamp` renders `"TIMESTAMP"` (no regression).
- `crates/smelt-db/tests/ts_function_returns.rs::define_signature_timestamptz_parses` — assert a `smelt.define` function with `Expr<Timestamp WITH TIME ZONE>` parameter annotation resolves without error in LSP diagnostics (use an `examples/test_workspace` fixture or inline model).

**Implementation shape.**
- Read `crates/smelt-types/src/lib.rs` `to_display_string` / `format_smelt_type_hover` path; verify `Timestamp { with_timezone: true }` already emits `"TIMESTAMP WITH TIME ZONE"`. If it does, the tests become immediately green — that's fine (they act as regression guards).
- Read `crates/smelt-types/src/parse.rs`; verify `TIMESTAMPTZ` → `Timestamp { with_timezone: true }` already works for `smelt.define` type annotations. Fix if not.
- Update `docs/ROADMAP.md`: mark the Timezone axis `✅` with date `2026-06-12` (or actual completion date).
- Add this plan to the References → Plans section of `docs/specs/types.md`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/lib.rs` — display/hover if broken
- `crates/smelt-types/src/parse.rs` — annotation parsing if broken
- `docs/ROADMAP.md` — timezone axis ✅
- `docs/specs/types.md` — add plan to References

**Docs touched.**
- `docs/ROADMAP.md` — timezone axis marked complete
- `docs/specs/types.md` — References → Plans entry added

**Review checklist** (material findings only):
- [ ] Hover renders `"TIMESTAMP WITH TIME ZONE"` for tz-aware and `"TIMESTAMP"` for naive
- [ ] `smelt.define` with `Expr<Timestamp WITH TIME ZONE>` parameter parses without LSP error
- [ ] ROADMAP timezone axis marked ✅ with date
- [ ] `docs/specs/types.md` References → Plans includes this plan
- [ ] `cargo test -p smelt-db` and `cargo test -p smelt-cli --test example_diagnostics` green

**Commit.** `feat(types): timezone axis complete — hover/sig surface verified, ROADMAP updated (§16)`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Pre-existing baseline repairs (committed before Phase 1, off-plan, user-approved).** The branch base (main) had four standing gates red from the decimal/parser work, unrelated to timezone: malformed `--- name: X` multi-model delimiters in three `test_workspace` decimal models + five inline strings in `decimal_arithmetic_tests.rs` (`example_diagnostics`, `decimal_arithmetic_tests`); pre-existing `cargo fmt` violations in smelt-cli/core/lsp; three uncatalogued `DiagnosticCode` variants in `docs/specs/diagnostics.md` (`diagnostics_catalogue`). Repaired in commit `32b1d0be` so timezone phases verify against a clean baseline.
- **Pre-existing decimal-division coercion bug (off-plan, user-approved).** `prop_coercion_matrix` was red: `Integer / Decimal` coerced to `Decimal(38, 10)` where DuckDB returns `Double`. Per spec §15 division with any Decimal operand is rejected; the inference + diagnostic only checked the left operand. Fixed to reject integer-family-over-Decimal too (→ Unknown + TypeMismatch), with the `Float/Double / Decimal` promotion carve-out preserved, in commit `feb32e0b` (which also remediated the affected ecommerce staging + multi_engine mart example models — the idiomatic `SUM(cents) / 100.0` pattern — to the portable `CAST(<int> AS DOUBLE) / 100.0`). Spec §15 Known Divergences wording clarified.
- **Pre-existing smelt-lsp compile errors (off-plan, user-approved).** `smelt-lsp` did not build on the branch base: `backend.rs` lagged a data-structure migration (a `str::Lines.rev()` that needs collecting first; a `(PathBuf, u32, u32)` tuple destructured as `(vp, _)`). Fixed in commit `46266989` so the full workspace builds. (The user also has an in-progress fix for this file on `main`.)
- **`unknown_census` gate red on baseline (known, not repaired).** 36 stale/unclassified entries from earlier merges drifting `function_body_check.rs`/`signatures.rs` line numbers — pre-existing, left as-is by user decision. Phase 2's own 2 new `DataType::Unknown` sites (mixed-tz LUB in `dispatch.rs`, mixed-tz arithmetic in `binary.rs`) ARE classified `error` (paired with a `TypeMismatch` diagnostic). The gate fails only on the unrelated pre-existing drift.
- **Build-env note.** `/dev/shm` (tmpfs) had filled with a stale 30G cargo cache from a prior session, starving the linker (SIGBUS on large DuckDB-linked test binaries). Cleared `/dev/shm/cargo-target`; not a code issue.
- **Test-helper note (no current impact).** The generator helper `function_return_type("DATE_TRUNC", ...)` in `generators.rs` returns naive `Timestamp { with_timezone: false }` regardless of argument tz. It populates `TypedExpr.expected_smelt_type`, which the property oracle never reads (the comparison uses live smelt inference vs DuckDB), so there is no false green today. If a future test wires `expected_smelt_type` into a comparison for a `TimestampTz` input, fix the helper to mirror the argument's tz-axis (as production `DATE_TRUNC` does).
- **Phase 4 follow-up: stale `BuiltinRegistry` tz entries.** Phase 1 removed `NOW`/`CURRENT_TIMESTAMP`/`DATE_TRUNC` from the `REGISTRY_MIGRATED` allowlist so they fall through to the corrected legacy match; the `BuiltinRegistry` signatures in `crates/smelt-types/src/signatures.rs` still carry `with_timezone: false`. No live path surfaces them today (verified by the Phase 1 reviewer), but Phase 4 should update those registry entries to `with_timezone: true` and restore the allowlist entries.

---

## Verification

How to confirm the spec is satisfied at the end:

```bash
# All unit + integration tests including property tests
cargo test -p smelt-db

# Property test with deeper coverage
PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference

# Example workspaces have no regressions
cargo test -p smelt-cli --test example_diagnostics
cargo test -p smelt-lsp --test example_workspaces

# Drift report
# /smelt:validate types
```
