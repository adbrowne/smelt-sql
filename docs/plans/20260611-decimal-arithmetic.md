# Plan: Decimal Arithmetic — Growth Formulas, Division Rejection, ABS Fix

**Date**: 2026-06-11
**Spec**: [`docs/specs/types.md`](../specs/types.md) §15 "Decimal arithmetic"
**Spec diff**: commit `38a6ab5e` (`spec(types): §15 Decimal arithmetic — growth formulas, division rejection, ABS fix`)
**Tracking PR / branch**: `worktree-type_system`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/types.md` §15 — it is the correctness oracle. Do not re-open settled spec decisions (growth formulas, division rejection, engine-binding deferral, and AVG→Double were agreed on 2026-06-11).
2. Confirm you are on branch `worktree-type_system`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just unit tests — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (Salsa purity: pure functions in `type_inference/` and `signatures.rs`; Salsa queries are thin wrappers).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/types.md` and `docs-site/docs/...` describe the feature as if it has always existed — no `Phase N` headings or labels in spec/user-doc body.

**Plan-specific conventions:**
- **Oracle-first.** `cargo test -p smelt-db --test type_property_tests` must stay green throughout. If the property oracle finds a new decimal-type violation, add a deterministic regression test before fixing it. Do not add divergence-registry entries to paper over violations — the goal is to fix them or, for division, to exclude them from the generator.
- **Backward compatibility.** `SUM(Decimal(p, s)) → Decimal(38, s)` and `AVG(Decimal) → Double` are already in the registry and correct; do not change them. Only the binary arithmetic formulas, `ABS`, and `numeric_lub` change.

---

## Context

The type system tracked `Decimal` arithmetic with a fixed `Decimal(38, 10)` placeholder regardless of operand precision, and allowed `Decimal / Decimal` (returning `Decimal(38, 10)`) despite DuckDB returning `Double` and the engines diverging on result-type family. Spec §15 fixes this: growth formulas carry precision/scale through arithmetic; division is rejected in portable code; `ABS(Decimal)` is corrected; and `numeric_lub` applies the UNION coercion rule.

## Scope

### In scope (spec §15 coverage)
- §15 "Arithmetic growth formulas": Spark-style `+/-*%` formulas with integer lifting; result `p' > 38` emits `DecimalPrecisionOverflow`.
- §15 "Decimal LUB": fix `numeric_lub` to use the UNION coercion rule for `(Decimal, Decimal)` and `(Decimal, integer)` pairs.
- §15 "Division rejection": `Decimal / T` → `TypeMismatch` with cast-to-Double message; property-test generator updated to skip Decimal division.
- §15 "Aggregate/unary decimal returns": confirm `SUM`/`AVG` are already correct; fix `ABS(Decimal(p,s)) → Decimal(p,s)`.
- Divergence-registry cleanup: remove `decimal_division` (operation rejected), `abs_decimal` (fixed), `abs_decimal_schema_resolved` (verify fixed by ABS fix).
- `DecimalPrecisionOverflow` diagnostic code in `DiagnosticCode` enum.

### Explicitly deferred
- **Engine-bound decimal division** — `Decimal / T` is rejected in all models (no engine escape hatch yet). Tracked in Known Divergences.
- **UNION overflow check** — `p' > 38` in UNION coercion is not yet emitted as a diagnostic (the LUB fix returns the correct type; the diagnostic walker covers only binary expressions for now).
- **`Float` as a distinct `DataType`** — the spec says `Float` collapses into `Double`; the enum still has `Float`; that rename is a separate plan.
- **Promotion chain `Float < Decimal` ordering** — the existing `Decimal { .. } => 4, Float => 5` rank in `numeric_lub` is a Known Divergence from the spec; fixing it is deferred.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 13f1304a | 2026-06-11 |
| 2     | done     | 63d0cbfb | 2026-06-11 |
| 3     | done     | 625164b7 | 2026-06-11 |
| 4     | pending  |        |      |
| 5     | pending  |        |      |

---

### Phase 1: `DecimalPrecisionOverflow` diagnostic code

**Goal.** Add the `DecimalPrecisionOverflow` variant to `DiagnosticCode` so Phases 2 and 3 can emit it. No logic yet — just the enum entry and plumbing.

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::decimal_precision_overflow_code_exists` — construct `DiagnosticCode::DecimalPrecisionOverflow`, assert it is distinct from `TypeMismatch` and `CannotInferType`. Fails until the variant is added.

**Implementation shape.** Add `DecimalPrecisionOverflow` to `DiagnosticCode` in `crates/smelt-db/src/diagnostics_types.rs` with a doc comment matching the spec description ("emitted when a decimal arithmetic expression computes p' > 38"). No other changes.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/diagnostics_types.rs` — new `DecimalPrecisionOverflow` variant
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs` — new test file (create it)

**Docs touched.**
- `docs/specs/types.md` — no change needed (spec already lists the code and its description).

**Review checklist** (material findings only):
- [ ] `DiagnosticCode::DecimalPrecisionOverflow` is defined and compiles
- [ ] Test exists and asserts the code is distinct from sibling codes
- [ ] No logic beyond the enum variant itself (scope check)
- [ ] Spec + docs-site edits are timeless — no `Phase X` labels

**Commit.** `feat(types): add DecimalPrecisionOverflow diagnostic code`

---

### Phase 2: Arithmetic growth formulas and overflow check

**Goal.** Replace the `Decimal(38, 10)` placeholder in `promote_numeric_operands` with the Spark-style growth formulas (spec §15 "Arithmetic growth formulas" + "Integer lifting"). Add a `check_decimal_precision_overflow_diagnostics` walker that emits `DecimalPrecisionOverflow` when `p' > 38`.

**Pre-conditions.** Phase 1 done (`DecimalPrecisionOverflow` compiles).

**TDD tests to write first.**
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::decimal_add_growth_formula` — `SELECT CAST(1 AS DECIMAL(10,2)) + CAST(1 AS DECIMAL(5,1))` infers `Decimal(12, 2)` (`max(8,4)+max(2,1)+1 = 8+2+1=11`... wait, let me recompute: `max(p1-s1, p2-s2) + max(s1,s2) + 1 = max(8,4) + max(2,1) + 1 = 8+2+1=11, s'=max(2,1)=2` → `Decimal(11, 2)`).
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::decimal_mul_growth_formula` — `DECIMAL(10,2) * DECIMAL(5,1)` infers `Decimal(16, 3)` (`p1+p2+1=10+5+1=16, s1+s2=3`).
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::integer_lifting_add_decimal` — `CAST(1 AS INTEGER) + CAST(1 AS DECIMAL(10,2))` — Integer lifts to `Decimal(10,0)`, formula: `max(10,8)+max(0,2)+1=10+2+1=13`, `s'=max(0,2)=2` → `Decimal(13, 2)`.
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::decimal_overflow_check_emits_diagnostic` — `DECIMAL(30,2) * DECIMAL(30,2)` → `p'=30+30+1=61 > 38`; inferred type is `Unknown`; `check_decimal_precision_overflow_diagnostics` produces exactly one `DecimalPrecisionOverflow` anchored at the `*` operator.
- Real-fixture: add a model in `examples/test_workspace/` using `DECIMAL(10,2) + DECIMAL(5,1)` in a SELECT list; verify it infers `Decimal(11, 2)` (not `Decimal(38,10)`) via `cargo test -p smelt-cli --test example_diagnostics`.

**Implementation shape.**
- In `binary.rs`: add a pure helper `decimal_arithmetic_result(p1: u32, s1: u32, p2: u32, s2: u32, op: &str) -> (u32, u32)` returning `(p', s')` per the spec formulas. Add integer-lifting helper `lift_integer_to_decimal(dt: &DataType) -> Option<(u32, u32)>` mapping `SmallInt → (5,0)`, `Integer → (10,0)`, `BigInt → (19,0)`.
- Rewrite the `(Some(DataType::Decimal { .. }), _) | (_, Some(DataType::Decimal { .. }))` arm in `promote_numeric_operands` to: lift integer-family operands, compute `(p', s')`, return `Decimal { precision: p', scale: s' }` if `p' <= 38`, else return `Unknown`.
- Add `pub fn check_decimal_precision_overflow_diagnostics(select_stmt: &SelectStmt, ctx: &TypeContext) -> Vec<Diagnostic>` in `binary.rs`, mirroring the structure of `check_crossfamily_arithmetic_diagnostics`: walk all `BINARY_EXPR` nodes with arithmetic operators, detect when both operands (after lifting) are decimal-family and `p' > 38`, emit `DecimalPrecisionOverflow` at the operator span.
- Wire `check_decimal_precision_overflow_diagnostics` into `lib.rs` alongside the existing `check_crossfamily_arithmetic_diagnostics` call (~line 1917).
- Export from `type_inference/mod.rs`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/type_inference/binary.rs` — growth formulas, integer lifting, overflow check walker
- `crates/smelt-db/src/type_inference/mod.rs` — export new function
- `crates/smelt-db/src/lib.rs` — call site for new walker (~line 1917)
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs` — tests
- `examples/test_workspace/` — fixture model

**Docs touched.**
- `docs/specs/types.md` — no change needed (spec §15 already describes the formulas).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified (formulas, lifting, overflow)
- [ ] Integer lifting covers all three integer types (`SmallInt`, `Integer`, `BigInt`)
- [ ] Overflow: type degrades to `Unknown`, diagnostic emitted exactly once at operator span
- [ ] Non-overflowing cases return the precise `Decimal(p', s')`, not `Decimal(38, 10)`
- [ ] `cargo test -p smelt-db --test type_property_tests` still green
- [ ] Salsa purity honored — new helpers are pure functions, no Salsa imports
- [ ] No scope creep into division or `numeric_lub` (Phases 3/4)

**Commit.** `feat(types): decimal arithmetic growth formulas with integer lifting and overflow check`

---

### Phase 3: Division rejection and generator guard

**Goal.** `Decimal / T` (any numeric `T`) emits `TypeMismatch` with the cast-to-Double message (spec §15 "Division rejection"). `infer_binary_expr_type` returns `Unknown` for decimal division. The property-test expression generator is updated to never produce `Decimal / T`. The `decimal_division` divergence-registry entry is removed.

**Pre-conditions.** Phase 2 done.

**TDD tests to write first.**
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::decimal_division_emits_type_mismatch` — `SELECT CAST(1 AS DECIMAL(10,2)) / CAST(1 AS DECIMAL(5,1)) AS result` infers `Unknown` for the `/` result AND produces a `TypeMismatch` diagnostic anchored at `/` with a message containing "cast" or "Double".
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::decimal_integer_division_rejected` — `DECIMAL(10,2) / CAST(1 AS INTEGER)` → same: `Unknown` + `TypeMismatch`.
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::integer_division_still_truncating` — `CAST(7 AS INTEGER) / CAST(2 AS INTEGER)` → `Integer` (no regression; spec §3).
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::double_division_still_works` — `CAST(7.0 AS DOUBLE) / CAST(2.0 AS DOUBLE)` → `Double` (no regression).
- Real-fixture: add a model in `examples/test_workspace/` that uses `CAST(a AS DOUBLE) / CAST(b AS DOUBLE)` (the portable remedy) — verify it is diagnostic-clean.

**Implementation shape.**
- In `infer_binary_expr_type`, in the `"*" | "/" | "%"` arm: before falling through to `promote_numeric_operands`, detect that the operator is `/` and that either operand is `Decimal`-family (after integer lifting). If so, return `Some(TypedColumn { data_type: DataType::Unknown, nullable: true })`.
- Add `pub fn check_decimal_division_diagnostics(select_stmt: &SelectStmt, ctx: &TypeContext) -> Vec<Diagnostic>` in `binary.rs`: walk all `/` `BINARY_EXPR` nodes; when either operand (after lifting) is decimal-family, emit `TypeMismatch` at the `/` operator span with message `"Decimal division is not in the portable surface — cast operands to Double: CAST(a AS DOUBLE) / CAST(b AS DOUBLE)"`.
- Export from `type_inference/mod.rs`; wire into `lib.rs` alongside the Phase 2 walker.
- In `crates/smelt-db/tests/prop_helpers/generators.rs`: filter out `/` binary-op generation when either operand type is `Decimal`-family. The generator must not produce `Decimal / T` combinations.
- In `crates/smelt-db/tests/prop_helpers/divergences.rs`: remove the `decimal_division` entry (it described a divergence in the output type for an operation that is now rejected; there is no longer a divergence to record).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/type_inference/binary.rs` — division guard in `infer_binary_expr_type`, new walker
- `crates/smelt-db/src/type_inference/mod.rs` — export
- `crates/smelt-db/src/lib.rs` — call site
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs` — tests
- `crates/smelt-db/tests/prop_helpers/generators.rs` — skip Decimal division
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — remove `decimal_division`
- `examples/test_workspace/` — cast-to-Double fixture

**Docs touched.**
- `docs-site/docs/guide/sql-models.md` — add a short note (timeless, feature-description tone) that `Decimal / Decimal` is not in the portable surface and the remedy is to cast to `Double`; link the error code `TypeMismatch`.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] `Decimal / T` returns `Unknown` and emits `TypeMismatch` with cast-to-Double message
- [ ] `Integer / Integer` and `Double / Double` are unaffected (spec §3)
- [ ] Generator no longer produces `Decimal / T` expressions
- [ ] `decimal_division` registry entry removed
- [ ] `cargo test -p smelt-db --test type_property_tests` still green (no new division violations to whitelist)
- [ ] docs-site edit is timeless — no phase vocabulary

**Commit.** `feat(types): reject Decimal division in portable code (TypeMismatch)`

---

### Phase 4: `numeric_lub` UNION/LUB coercion fix

**Goal.** Fix `numeric_lub` in `signatures.rs` to apply the spec §15 "Decimal LUB" formula instead of the `Decimal(38, 10)` placeholder for `(Decimal, integer)` pairs and the discriminant-match early-out for `(Decimal, Decimal)` pairs with different `(p, s)`.

**Pre-conditions.** Phase 2 done (growth formulas landed; `numeric_lub` fix is independent of division).

**TDD tests to write first.**
- `crates/smelt-types/src/signatures.rs::tests::decimal_decimal_lub_coercion_formula` — `numeric_lub(&Decimal(10,2), &Decimal(8,3))` → `Decimal(11, 3)` (formula: `max(8,5)+max(2,3)=8+3=11, s'=3`).
- `crates/smelt-types/src/signatures.rs::tests::decimal_same_params_lub_unchanged` — `numeric_lub(&Decimal(10,2), &Decimal(10,2))` → `Decimal(10, 2)` (same-params case returns unchanged).
- `crates/smelt-types/src/signatures.rs::tests::integer_decimal_lub_lifting` — `numeric_lub(&DataType::Integer, &Decimal(10,2))` → Integer lifts to `Decimal(10,0)`, formula: `max(10,8)+max(0,2)=10+2=12, s'=2` → `Decimal(12, 2)`.
- `crates/smelt-types/src/signatures.rs::tests::bigint_decimal_lub_lifting` — `numeric_lub(&DataType::BigInt, &Decimal(5,2))` → BigInt lifts to `Decimal(19,0)`, formula: `max(19,3)+max(0,2)=19+2=21, s'=2` → `Decimal(21, 2)`.
- `crates/smelt-types/src/signatures.rs::tests::numeric_lub_chain_unaffected` — `numeric_lub(&DataType::Integer, &DataType::Double)` → `Double` (non-Decimal cases unchanged).

**Implementation shape.**
- In `numeric_lub`:
  - Remove the discriminant-based early-out (or keep it only for non-Decimal types) so that `(Decimal(p1,s1), Decimal(p2,s2))` with `p1!=p2 || s1!=s2` falls through to the formula.
  - Add a match arm `(Decimal { precision: p1, scale: s1 }, Decimal { precision: p2, scale: s2 })` that applies the coercion formula: `s' = max(s1, s2)`, `p' = max(p1-s1, p2-s2) + s'`. Return `Decimal { precision: p'.min(38), scale: s' }` (saturate at 38; the overflow diagnostic walker is arithmetic-only for now — see Deferred).
  - Replace the old `(Decimal, SmallInt|Integer|BigInt)` arm with one that lifts the integer to its natural equivalent and applies the coercion formula.
  - The `signatures.rs` pure-function constraint is maintained — no diagnostic emission; overflow saturates silently at 38 in the LUB case (acceptable per Deferred).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs` — `numeric_lub` function + new unit tests

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: the "Promotion chain implementation drift" entry already notes `Decimal(38,10)` behaviour; update the entry to narrow its scope to the `Float < Decimal` ordering issue only (the `Decimal(38,10)` from integer-mixing is now fixed).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert the formula produces the right `(p', s')`
- [ ] `(Decimal, Decimal)` with same params still returns unchanged (no unnecessary LUB widening)
- [ ] `(integer, Decimal)` and `(Decimal, integer)` use integer lifting, not `Decimal(38, 10)`
- [ ] Non-Decimal numeric pairs (`Integer / Double`, etc.) are unaffected
- [ ] `cargo test -p smelt-types` and `cargo test -p smelt-db --test type_property_tests` still green
- [ ] Salsa purity: pure function, no Salsa imports

**Commit.** `fix(types): numeric_lub uses UNION coercion formula for Decimal pairs`

---

### Phase 5: ABS(Decimal) registry fix and divergence cleanup

**Goal.** Fix `ABS(Decimal(p,s)) → Decimal(p,s)` per spec §15 (currently returns `Unknown` for property-test Decimal inputs, ByDesign for schema-resolved Decimal inputs). Remove `abs_decimal` (KnownBug) and `abs_decimal_schema_resolved` (ByDesign) from the divergence registry once fixed.

**Pre-conditions.** Phases 2–4 done (the Decimal type paths are now sound, so `ABS` can be tested cleanly).

**TDD tests to write first.**
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::abs_decimal_preserves_precision_scale` — a SELECT `ABS(CAST(-1.23 AS DECIMAL(10,2)))` infers `Decimal(10, 2)`. Fails until the fix lands.
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::abs_integer_unaffected` — `ABS(CAST(-1 AS INTEGER))` → `Integer` (no regression on the generic path).
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs::abs_decimal_schema_col_preserves_type` — given a source column declared `DECIMAL(18,2)`, `ABS(that_col)` infers `Decimal(18, 2)`.
- Real-fixture: a model in `examples/test_workspace/` selecting `ABS(decimal_col)` — no diagnostics via `example_diagnostics`.

**Implementation shape.**
- Investigate root cause: in `BuiltinRegistry`, `ABS` is registered as `ABS<T: Numeric>(T) → T`. When `T` resolves to `Decimal { precision, scale }` via generic binding, the return-type substitution should produce the same `Decimal { precision, scale }`. The bug is likely in how generic binding handles `DataType::Decimal { .. }` variants (which have fields) vs. unit-like variants.
- Fix options (implementer picks based on root cause):
  a. Fix the generic binding substitution to propagate `precision`/`scale` from the bound type (preferred — keeps registry clean).
  b. Add a separate concrete `ABS(Decimal) → Decimal` overload with a precise signature (if the registry supports per-precision entries — likely not; option a is preferable).
  c. Add Decimal-specific dispatch logic in the ABS call-site handler before the generic path.
- After fixing: remove `abs_decimal` and `abs_decimal_schema_resolved` entries from `divergences.rs`, and update any tests that reference those entries.
- Verify `cargo test -p smelt-db --test type_property_tests` still green (the oracle no longer encounters `ABS(Decimal) → Unknown`; no new divergence entry needed).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs` — generic binding fix (or ABS dispatch), BuiltinRegistry
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — remove `abs_decimal`, `abs_decimal_schema_resolved`
- `crates/smelt-db/tests/decimal_arithmetic_tests.rs` — tests
- `examples/test_workspace/` — fixture

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: no entry needed for ABS after fix (was already the spec-desired behaviour). Verify the "ByDesign aggregate divergences" entry accurately reflects only `SUM`/`AVG`/`SIGN` after `abs_*` entries are removed from the registry.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert `ABS(Decimal(p,s)) → Decimal(p,s)`
- [ ] Generic `ABS(T: Numeric) → T` still works for non-Decimal types
- [ ] `abs_decimal` and `abs_decimal_schema_resolved` removed from divergence registry
- [ ] `cargo test -p smelt-db --test type_property_tests` green with no new whitelist entries
- [ ] `cargo test -p smelt-cli --test example_diagnostics` and `example_workspaces` green
- [ ] Spec Known Divergences entry is accurate post-fix

**Commit.** `fix(types): ABS(Decimal(p,s)) preserves precision and scale; remove abs_decimal divergences`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

---

## Verification

How to confirm the spec §15 is satisfied at the end:

- `cargo test -p smelt-db --test type_property_tests` — existing oracle, green (no new decimal-arithmetic violations; generator skips Decimal division).
- `cargo test -p smelt-db --test nullability_property_tests` — §11 gate unaffected, must stay green.
- `cargo test -p smelt-db --test decimal_arithmetic_tests` — the new unit/integration suite from Phases 1–5.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` — example fixtures from each phase are diagnostic-clean.
- `cargo fmt --all -- --check` and `cargo clippy --all-targets` clean.
- `/smelt:validate types` reports zero drift on §15.
