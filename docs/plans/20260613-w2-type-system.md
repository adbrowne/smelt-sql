# Plan: W2 — Type-system correctness fixes (D-types)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the second wave of the spec-remediation implementation backlog. Remediates the **D-types** cluster of the 2026-06-13 spec review: the two genuine decisions **D-28** (VALUES temporal-family LUB with strict tz-mixing) and **D-29** (`Char` in the string-equality family), plus the Appendix-A determinate type fixes tagged `impl→ D-types`: **C16** (DECIMAL widening `s2≥s` ∧ `(p2−s2)≥(p−s)`), **C17** (FragmentKindMismatch direction), **C26** (nullability-in-signatures), **NOW()/CURRENT_TIMESTAMP** non-nullable origin, and the **decimal-arithmetic integer-lift trigger**. No hard dependency on W1; ordered second as low-risk foundational analysis. The autonomy loop works this sub-plan phase by phase.

**Date**: 2026-06-13
**Spec**: `docs/specs/types.md` §4 "String unification" (Char folding), §"VALUES" (temporal-family LUB), §11 nullability + §16 "Timezone" (strict tz-mixing reused for VALUES; NOW/CURRENT_TIMESTAMP non-nullable origin), §"Numeric promotion chain" (decimal-arith trigger); `docs/specs/schema_evolution.md` §"Safe scalar type widenings" (C16 DECIMAL widening predicate); `docs/specs/scoping.md` §"Diagnostic codes" (C17 FragmentKindMismatch direction); `docs/specs/gradual_typing.md` §nullability (C26 pointer to types.md §11).
**Spec diff**: `e862ebec..HEAD` — **already landed** (the review committed all of these to the specs above). This plan is code-catching-up-to-spec; no spec edits except the P6 close-out retraction of any now-satisfied Known-Divergence note.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. The specs already landed; no `docs-site/` page changes. Close-out updates the master registry + `docs/ROADMAP.md`.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`) per the per-phase routine below (pre-flight → red-green `/smelt:implement` with implementer + reviewer, spec as oracle → verification gates incl. the type/nullability property oracles → set the row `done` + date → commit + push with the phase's commit message). If that was the last `pending` phase, also flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue, §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>`, or `<<ALL_DONE>>`.

## Goal

Bring the type system into line with the reviewed `types.md`/`schema_evolution.md`/`scoping.md`/`gradual_typing.md`:

- **D-29 (minor).** `normalize()` (`crates/smelt-types/src/lib.rs:134-147`) folds `Text ↔ Varchar(None)` but **not** `Char(_)`, so `Char(5) = Text` fails type-equality despite the prose. Fold `Char(_)` into the canonical string type.
- **D-28 (minor, correctness).** VALUES column LUB (`crates/smelt-db/src/type_inference/values.rs:48-99`) uses `promote_types` (`dispatch.rs:650`), which silently returns `Unknown` on a temporal/tz mix with **no diagnostic**. Apply §16's strict tz-mixing (and cross-temporal-family `Date`/`Timestamp` mixing) to VALUES → `TypeMismatch` at the VALUES span, mirroring the UNION/CASE/arithmetic checks (`dispatch.rs:804,903`, `binary.rs:737`).
- **NOW/CURRENT_TIMESTAMP non-nullable origin + C26 (minor).** Registry-declared non-nullable nullary built-ins (NOW, CURRENT_TIMESTAMP — `crates/smelt-types/src/signatures.rs:3963-3983`) need a non-nullable **origin** (§11). C26: confirm bare param/return annotations stay nullable and `NOT NULL` is opt-in (`SigParam.not_null`, `signatures.rs:1623-1627` — already implemented; this reconciles + locks it).
- **C16 (correctness).** Decimal widening safety is unspecified in the type-compatibility path (`types_assignment_compatible`, `function_body_check.rs:2114-2129` has integer-family widening only). Add the canonical predicate: `Decimal(p2,s2)` can hold `Decimal(p1,s1)` iff `s2≥s1` **and** `(p2−s2)≥(p1−s1)` (integer-digit capacity must not shrink).
- **Decimal-arith trigger (minor).** Integer→decimal lifting must apply only when **≥1 operand is already Decimal-family** — `binary.rs:122-126` already gates on `either_decimal`; lock it with an explicit regression test (likely green on write).
- **C17 (correctness).** `FragmentKindMismatch` (`function_body_check.rs:2695-2757`) currently lets a Scalar-only splice point accept **any** kind (`ExprKind::Scalar => true`). The rule should fire when a fragment's kind is **higher** than the splice point admits — a Scalar-only point must reject Agg/Window fragments.

## Design decisions (resolved — do not re-litigate; from `docs/research/20260613-spec-remediation-decisions.md` Theme 5 + Appendix A)

- **D-29 = A.** Extend `normalize()` to fold `Char(_)` into the string family for equality (downstream cares about family, not padding, at the type level).
- **D-28 = A.** VALUES temporal mixing is **strict**, identical to §16's UNION/CASE rule: a naive `Timestamp` mixed with a `Timestamp WITH TIME ZONE`, or `Date` mixed with `Timestamp`, has no LUB → `TypeMismatch` at the VALUES clause span. No silent widening of naive→tz-aware or Date→Timestamp. The user must `CAST`. Reuse the existing tz-mixing utility rather than duplicating logic.
- **C16.** DECIMAL widening is Safe iff `s2≥s` **and** `(p2−s2)≥(p−s)`. Pure correctness — the integer-digit capacity must not shrink. Implement as a pure predicate (e.g. `decimal_widening_is_safe(p1,s1,p2,s2)`) in the type layer and apply it in `types_assignment_compatible`. The schema-evolution classifier's adoption of the same predicate is covered in the W8 schema_evolution sub-plan (cross-reference, don't reach into `smelt-runtime` here).
- **C17.** Reverse the direction: a Scalar-only splice point admits only Scalar; Agg/Window fragments in a Scalar-only point fire `FragmentKindMismatch`. (Agg admits Agg|Window stays; Window admits Window stays.)
- **C26.** Bare annotations stay nullable; `NOT NULL` is the opt-in (already implemented via `SigParam.not_null`). This wave reconciles the gradual_typing pointer and locks the behavior with a test — no behavior change expected.
- **Decimal-arith trigger.** Already correct (`either_decimal` gate); add the regression test, do not refactor.
- **Type-inference purity invariant.** All changes keep analysis logic pure (`type_inference` module is pure functions; the Salsa-purity rule in `architecture.md`). The DuckDB **property-test oracles** (`type_property_tests.rs`, `nullability_property_tests.rs`) are the correctness gate — a new behavior that diverges from DuckDB is either a real bug or a registered divergence (`tests/prop_helpers/divergences.rs`), never a silent change.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. Red on this phase's own target → proceed; red on **unrelated** breakage → block (§"Block conditions").
2. **Red-green `/smelt:implement`.** Failing test(s) first, then implementation, spec as oracle. Implementer then reviewer (material findings only). When a property test failure surfaces, add an explicit unit test capturing it before fixing (CLAUDE.md).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; the relevant oracle (`cargo test -p smelt-db --test type_property_tests` and/or `--test nullability_property_tests`); the dual gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`.
4. **Record + commit.** Set the row `done` + date; commit + push tests + impl + table together with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)
Set the row to `blocked` + one-line reason; append a dated §"Blocked phases" entry; restore a clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- A design decision not answered by this plan or the spec — e.g. the property oracle flags a DuckDB divergence that needs a judgment call on whether to register it vs change inference; or C16's predicate turns out to be consumed only by `smelt-runtime` schema-evolution (defer to W8 rather than widen scope here).
- Pre-flight red on unrelated breakage.
- The tree can't be returned to green.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | `Char(_)` folded into string family in `normalize()` | done | D-29 | feat(types): fold Char into the string-equality family in normalize (D-29) | 2026-06-14 |
| P2 | VALUES temporal-family LUB + strict tz-mixing | done | D-28 | feat(db): strict temporal/tz mixing in VALUES columns → TypeMismatch (D-28) | 2026-06-14 |
| P3 | Non-nullable origin for nullary built-ins; signature-nullability reconcile | done | NOW-null, C26 | feat(types): non-nullable origin for NOW/CURRENT_TIMESTAMP; lock bare-stays-nullable (C26) | 2026-06-14 |
| P4 | Decimal widening predicate + arithmetic lift-trigger lock | pending | C16, decimal-arith | feat(types): decimal widening safety predicate (s2≥s ∧ p2−s2≥p−s); lock decimal-arith trigger (C16) | |
| P5 | FragmentKindMismatch direction (Scalar-only rejects Agg/Window) | pending | C17 | fix(db): FragmentKindMismatch fires for higher-kind fragment in scalar-only splice (C17) | |
| P6 | Close-out: property oracles green + registry + ROADMAP | pending | — | docs(spec-impl): close out W2 — type-system fixes landed; registry + roadmap | |

**Status values**: `pending`, `done`, `blocked`.

---

### Phase P1: `Char(_)` into the string family

**Goal.** Fold `Char(_)` into the canonical string type in `normalize()` so `Char(n)`, `Varchar(_)`, and `Text` are interchangeable for type-equality.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-types/src/lib.rs::tests::char_normalizes_to_string_family` — `DataType::Char { length: 5 }.normalize() == DataType::Text.normalize()` and `== Varchar{None}.normalize()`.
- A type-equality test asserting `Char(5)` compares equal to `Text` and `Varchar(10)` through the normalize path.

**Implementation shape.** Add a `DataType::Char { .. } => DataType::Varchar { max_length: None }` arm to `normalize()` (`smelt-types/src/lib.rs:134-147`), beside the existing `Text` arm.

**Critical files.** `crates/smelt-types/src/lib.rs` (normalize + tests).

**Review checklist:** Char folds to the same canonical type as Text/Varchar; no change to backend emission (`to_backend_sql` still emits CHAR where relevant); `is_string` unaffected.

**Commit.** `feat(types): fold Char into the string-equality family in normalize (D-29)`

---

### Phase P2: VALUES temporal-family LUB + strict tz-mixing

**Goal.** A VALUES column that mixes naive `Timestamp` with `Timestamp WITH TIME ZONE`, or `Date` with `Timestamp`, is a `TypeMismatch` (anchored at the VALUES clause span) — not a silent `Unknown`.

**Pre-conditions.** None (independent of P1).

**TDD tests to write first:**
- `crates/smelt-db/src/type_inference/values.rs::tests::values_mixed_tz_is_type_mismatch` — `(VALUES (TIMESTAMP '…'), (TIMESTAMPTZ '…'))` column → `TypeMismatch` at the VALUES span; no silent `Unknown`.
- `...::values_date_timestamp_mix_is_type_mismatch` — `Date`/`Timestamp` mix → `TypeMismatch`.
- `...::values_homogeneous_temporal_ok` — all-naive or all-tz column infers the concrete temporal type (no false positive).
- A `type_property_tests` run stays green (or a new divergence is registered, not silently changed).

**Implementation shape.** Extract/reuse the strict tz/temporal-mismatch check the set-op and CASE paths use (`check_mixed_tz_setop_diagnostics` `dispatch.rs:804`, `check_mixed_tz_case_diagnostics` `dispatch.rs:903`) into a shared `check_mixed_temporal_mismatch(types, span)` and call it from `infer_values_columns` (`values.rs:48-99`) after column-wise `promote_types`. Emit `TypeMismatch` at the VALUES clause span. Fix the dangling §5 reference behavior (the spec already corrected the prose).

**Critical files.** `crates/smelt-db/src/type_inference/values.rs`, `crates/smelt-db/src/type_inference/dispatch.rs` (extract shared helper — keep pure).

**Review checklist:** strict mixing matches §16 UNION/CASE exactly; anchored at VALUES span; homogeneous temporal columns still infer concretely; type-inference purity preserved; property oracle green or divergence registered.

**Commit.** `feat(db): strict temporal/tz mixing in VALUES columns → TypeMismatch (D-28)`

---

### Phase P3: Non-nullable origin for nullary built-ins; signature-nullability reconcile

**Goal.** Give registry-declared non-nullable nullary built-ins (NOW, CURRENT_TIMESTAMP) a non-nullable nullability origin (§11), and lock that bare param/return annotations stay nullable while `NOT NULL` is the opt-in (C26).

**Pre-conditions.** None.

**TDD tests to write first:**
- A nullability test (`nullability_property_tests` smoke or a unit) asserting `SELECT NOW()` / `CURRENT_TIMESTAMP` columns are **non-nullable** with a registry/built-in origin.
- `crates/smelt-types/src/signatures.rs::tests::bare_annotation_is_nullable` and `::not_null_annotation_opts_in` — confirm `SigParam.not_null` semantics (lock-in; expected green).

**Implementation shape.** Tag NOW/CURRENT_TIMESTAMP (and any registry-declared non-nullable nullary built-in) with a non-nullable origin where built-in return nullability is derived (`smelt-types/src/signatures.rs:3963-3983` + the nullability-origin plumbing introduced by `docs/plans/20260610-nullability-soundness.md`). If C26 is already satisfied in code, P3 reduces to the NOW-origin tag + the two lock-in tests.

**Critical files.** `crates/smelt-types/src/signatures.rs`; nullability-origin site in `crates/smelt-db/src/type_inference/` if the origin is assigned there.

**Review checklist:** NOW/CURRENT_TIMESTAMP non-nullable with correct origin; bare-stays-nullable + NOT-NULL-opt-in locked; nullability property oracle green.

**Commit.** `feat(types): non-nullable origin for NOW/CURRENT_TIMESTAMP; lock bare-stays-nullable (C26)`

---

### Phase P4: Decimal widening predicate + arithmetic lift-trigger lock

**Goal.** Add the canonical decimal-widening safety predicate (`s2≥s ∧ (p2−s2)≥(p−s)`) and apply it in type-assignment compatibility; lock the decimal-arithmetic integer-lift trigger (already gated on `either_decimal`).

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-types/src/...::tests::decimal_widening_safe_and_unsafe` — `Decimal(10,2)` holds `Decimal(5,2)` (safe); `Decimal(5,4)` does **not** hold `Decimal(5,2)` (integer digits shrink → unsafe); equal-precision identity safe.
- `crates/smelt-db/src/function_body_check.rs::tests::decimal_arg_widens_to_param` — a `Decimal(5,2)` argument satisfies a `Decimal(10,2)` parameter via `types_assignment_compatible`; a narrowing case is rejected.
- `crates/smelt-db/src/type_inference/binary.rs::tests::integer_arith_no_decimal_lift` — `1 + 1` stays Integer (no lift); `1 + CAST(1 AS DECIMAL(5,2))` lifts (regression-lock for the trigger).

**Implementation shape.** Add a pure `decimal_widening_is_safe(p1,s1,p2,s2) -> bool` (in `smelt-types`, beside the numeric helpers) and call it from `types_assignment_compatible` (`function_body_check.rs:2114-2129`) for the `(Decimal,Decimal)` case. Leave `binary.rs:122-126` unchanged (already correct) — only add the regression test.

**Critical files.** `crates/smelt-types/src/lib.rs` (or numeric helper module) for the predicate; `crates/smelt-db/src/function_body_check.rs` for the compat wiring; `crates/smelt-db/src/type_inference/binary.rs` (test only).

**Review checklist:** predicate matches `s2≥s ∧ (p2−s2)≥(p−s)`; assignment compat uses it; decimal-arith trigger unchanged but locked; if the predicate's only real consumer is `smelt-runtime` schema-evolution, note the cross-ref to W8 (don't reach into `smelt-runtime` here).

**Commit.** `feat(types): decimal widening safety predicate (s2≥s ∧ p2−s2≥p−s); lock decimal-arith trigger (C16)`

---

### Phase P5: FragmentKindMismatch direction

**Goal.** A Scalar-only splice point rejects Agg/Window fragments (the rule fires when a fragment's kind is higher than the splice point admits).

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-db/src/function_body_check.rs::tests::scalar_splice_rejects_agg_fragment` — a `SelectItems<Scalar>` parameter bound to an Agg/Window fragment fires `FragmentKindMismatch`.
- `...::agg_splice_accepts_agg_and_window` and `...::scalar_splice_accepts_scalar` — no false positives (the surviving directions).

**Implementation shape.** In `check_fragment_context_bindings` (`function_body_check.rs:2721-2754`), change the `req_kind` match so `ExprKind::Scalar => matches!(found_kind, ExprKind::Scalar)` (was `=> true`). Keep Agg → `Agg|Window`, Window → `Window`.

**Critical files.** `crates/smelt-db/src/function_body_check.rs`.

**Review checklist:** Scalar-only rejects higher kinds; Agg/Window directions unchanged; existing function/scoping examples still green (audit `examples/test_workspace/functions/` for a now-correctly-rejected case — block if a fixture is genuinely wrong rather than the rule).

**Commit.** `fix(db): FragmentKindMismatch fires for higher-kind fragment in scalar-only splice (C17)`

---

### Phase P6: Close-out

**Goal.** Confirm the type/nullability oracles are green across the wave, retract any now-satisfied Known-Divergence note, and roll up.

**Pre-conditions.** P1–P5 done.

**TDD tests to write first:** none new — this phase runs the full oracles as the gate.

**Implementation shape.** `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests` and `--test nullability_property_tests` green (register any genuine new divergence in `tests/prop_helpers/divergences.rs` with rationale). Retract any types.md/schema_evolution.md Known-Divergence note this wave satisfies (timeless edit). Flip the master registry W2 row to `done (2026-06-13)`; add a `docs/ROADMAP.md` line.

**Critical files.** `crates/smelt-db/tests/prop_helpers/divergences.rs` (only if a divergence is registered), the relevant spec (KD retraction only), `docs/plans/20260613-spec-impl.md`, `docs/ROADMAP.md`.

**Review checklist:** oracles green at 1000 cases; any divergence registered with rationale; spec edits timeless; registry row `done`; ROADMAP updated.

**Commit.** `docs(spec-impl): close out W2 — type-system fixes landed; registry + roadmap`

---

## Deferred during implementation

(Append-only.)

- The schema-evolution classifier's adoption of the C16 decimal-widening predicate lands in the **W8 schema_evolution** sub-plan (alongside D-58); W2 owns only the type-layer predicate.

## Blocked phases

Append-only log. None yet.

## Verification

- `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests` and `--test nullability_property_tests` green.
- `cargo test -p smelt-types`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` green.
- Manual smoke: `Char(5) = Text` type-checks; a mixed-tz VALUES column errors with `TypeMismatch`; `SELECT NOW()` is non-nullable; a `Decimal(5,4)`-into-`Decimal(5,2)` assignment is rejected.
- `/smelt:validate types` and `/smelt:validate scoping` report no behavioural drift on these surfaces.
