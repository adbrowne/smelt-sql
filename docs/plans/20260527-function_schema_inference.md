# Plan: Function Schema Inference

**Date**: 2026-05-27
**Spec**: [`docs/specs/function_schema_inference.md`](../specs/function_schema_inference.md)
**Spec diff**: new spec (commit `1f4eaef8`) + `types.md` `Unknown`-discriminant edits
**Tracking PR / branch**: PR #124 — branch `worktree-unknown_types`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/function_schema_inference.md` and the relevant sections of `docs/specs/types.md` (§"Strict-by-default doctrine") — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. If every phase is `done`, run the Verification section and stop.

**For each phase, run the per-phase loop in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push to PR #124.

**Phase risk ordering.** Phases 1–3 are concrete inference fixes (low risk, high value); land and push them first. Phases 4–5 (the `Unknown` reason-discriminant and `ColumnTypeUnresolved` enforcement) are deeper type-system changes — if either stalls against a material blocker, mark it `pending`, record the blocker under "Deferred during implementation", and continue to the docs phase rather than blocking the whole run. Phases 1–3 + their docs are the must-land core.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` first).
- `cargo test`/`cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests in `examples/`, not just AST units.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits using the `Commit.` line verbatim; push after each.
- Never skip hooks, never `--no-verify`, never force-push.
- No scope creep into a later phase.
- Honor `CLAUDE.md` invariants: `smelt-db` pure-function rule; Workspace Loading Parity Rule; Run Pipeline Parity Rule.
- **Timeless-oracle rule.** Phase vocabulary lives in *this plan only*. Spec and `docs-site/` edits read as timeless feature descriptions — no `Phase X` headings/labels/callouts in their body. As a gap closes, delete its §Known Divergences entry from the spec in the same commit.

---

## Context

`smelt type` reports silent `Unknown` for many function-derived columns. The function-discovery parity bug behind most of them is already fixed (`init_db`, commit `69d70950`); what remains is the schema-layer surface the spec defines: struct-spread `.*` expansion (spec §Semantics rule 2, invariant 2), two residual `TableExpr` body-schema gaps (rules 3–4), and the no-silent-`Unknown` contract (rule 6, invariant 4) that `types.md` §"Strict-by-default doctrine" already mandates but the implementation does not yet enforce.

## Scope

### In scope (spec coverage)
- §Semantics rule 2 — `smelt.functions.<f>(...).*` and `.field` expand declared struct fields into typed schema columns (closes §Known Divergence #1; satisfies invariant 2).
- §Semantics rules 3–4 — `TableExpr` argument that is a local CTE/derived table is seeded; residual body-computed projections infer (closes §Known Divergences #2, #3).
- `types.md` — `Unknown` reason-discriminant (`Unresolved`/`Dynamic`/`Propagated`).
- §Semantics rule 6 / invariant 4 — `ColumnTypeUnresolved` fires by default at the origin for function-schema `Unresolved` columns (closes §Known Divergence #4).
- §References → User docs — document `.*` schema projection in `docs-site/docs/reference/language.md`.

### Explicitly deferred
- Generator-emitted and `smelt.columns_of`-reflected model schemas, and meta-language HOF values in SQL column position — owned by `meta_language.md` (spec invariant 5). These remain non-erroring; Phase 4 must classify their unknowns as non-`Unresolved` so Phase 5 does not flag them.
- Disambiguating a bare body `*` across multiple `TableExpr` parameters (spec invariant 5).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 7d61882f | 2026-05-27 |
| 2     | done     | 4b8911f5 | 2026-05-27 |
| 3     | done     |        | 2026-05-27 |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |

---

### Phase 1: Struct-spread `.*` / `.field` schema expansion

**Goal.** A `smelt.functions.<f>(...).*` over an `Expr<Struct<{…}>>`-returning function contributes the declared fields (in declared order, with field types) to the containing model's resolved `ModelSchema`; `.field` contributes the single named field. Closes the schema-layer/codegen disagreement (invariant 2).

**Pre-conditions.** Function discovery in `init_db` (already landed, `69d70950`).

**TDD tests to write first.**
- `crates/smelt-db/...` unit test — a `SELECT id, f(x).* FROM t` model where `f -> Expr<Struct<{a: Text, b: Text}>>` resolves output columns `{id, a: Text, b: Text}` (currently `{id}` only).
- `crates/smelt-db/...` unit test — `f(x).field` resolves the single field's type; an unknown field name is unresolved (no panic).
- `crates/smelt-db/...` unit test — row-tail `Struct<{…, ..r}>` spread binds extras from the struct argument's call-site schema.
- Real-fixture assertion in `crates/smelt-cli/tests/type_command_function_returns.rs` — `examples/web_analytics` `events_parsed` output schema includes `event_name: TEXT, platform: TEXT, url: TEXT`.

**Implementation shape.** In the model-schema builder (`crates/smelt-db/src/queries/schema.rs`, `typed_model_schema`/`resolved_model_schema` and the select-item column extraction), detect a SELECT item that is a `SmeltPathCall` with a trailing `.*` or `.field`, resolve the call's struct return type via the existing `resolve_struct_return_type` (`type_inference/function_call.rs`), and emit one `Column` per declared field (or the single named field). Reuse the struct-field resolution already used for the scalar type; the new work is turning those fields into schema columns rather than a single struct column.

**Critical files.**
- `crates/smelt-db/src/queries/schema.rs` — select-item → column emission for struct-spread.
- `crates/smelt-db/src/type_inference/function_call.rs` — reuse `resolve_struct_return_type`; no behavior change to scalar path.

**Docs touched.**
- `docs/specs/function_schema_inference.md` — remove §Known Divergence #1.
- `docs-site/docs/reference/language.md` — document that `smelt.functions.<f>(...).*` projects the function's struct fields as columns (deferred detail may land with Phase 6; at minimum note the schema behavior).

**Review checklist:**
- [ ] TDD tests exist and assert the field set + types
- [ ] Spec rule 2 satisfied; invariant 2 (schema == codegen) holds for the spread
- [ ] `smelt-db` purity preserved
- [ ] `example_diagnostics` + `example_workspaces` stay green (web_analytics now richer, still zero-diagnostic)
- [ ] No scope creep into TableExpr phases
- [ ] Spec edit is timeless

**Commit.** `feat(types): expand smelt.functions struct-spread .* into model output schema`

---

### Phase 2: Seed `TableExpr` argument that is a local CTE / derived table

**Goal.** When a `TableExpr`-function argument references a CTE or derived table defined in the caller (not a `smelt.<path>` ref or nested function call), the body context is seeded with that source's schema so body-computed columns resolve. Closes §Known Divergence #2.

**Pre-conditions.** None beyond current `resolve_smelt_path_call_schema`.

**TDD tests to write first.**
- `crates/smelt-db/...` unit test — a model `WITH x AS (SELECT … literal cols …) SELECT col FROM smelt.functions.f(x)` resolves `col`'s type where `f` is `TableExpr`-returning and `col` is body-computed.
- Real-fixture assertion in `crates/smelt-cli/tests/type_command_function_returns.rs` — `examples/functions_demo` `margin_via_cte.margin` resolves (currently `Unknown`).

**Implementation shape.** In `SalsaRefSchemaProvider::resolve_smelt_path_call_schema` (`queries/schema.rs`), extend `TableExpr`-argument resolution: when the argument is a bare identifier matching a CTE/derived-table in the caller's scope, resolve that CTE's schema (reuse `extract_function_body_cte_schemas` / the caller's CTE schemas) and seed `add_tableexpr_param`. Today only `smelt.<path>` refs and nested calls are handled.

**Critical files.**
- `crates/smelt-db/src/queries/schema.rs` — `resolve_smelt_path_call_schema` argument resolution.

**Docs touched.**
- `docs/specs/function_schema_inference.md` — remove §Known Divergence #2.

**Review checklist:**
- [ ] TDD tests exist; `margin_via_cte.margin` resolves
- [ ] Spec rule 3 (TableExpr argument schema resolution) satisfied
- [ ] `smelt-db` purity preserved
- [ ] No regression in other functions_demo / web_analytics columns
- [ ] Spec edit is timeless

**Commit.** `fix(types): seed local CTE/derived-table args into TableExpr function body schema`

---

### Phase 3: Residual `TableExpr` body-computed projection inference

**Goal.** Explicit body projections in `TableExpr` function bodies that currently fail to infer (e.g. certain expression forms) resolve to their type. Closes §Known Divergence #3.

**Pre-conditions.** Phase 2 (so CTE-arg cases are already seeded; isolate the genuinely-uninferred forms).

**TDD tests to write first.**
- `crates/smelt-db/...` unit test reproducing the specific failing projection form from `examples/functions_demo` `session_rollup` (the `session_id` column), asserting its resolved type.
- Real-fixture assertion in `crates/smelt-cli/tests/type_command_function_returns.rs` — `rollup_dashboard.session_id` resolves (currently `Unknown`).

**Implementation shape.** Trace why `infer_tableexpr_return_schema`'s explicit-projection branch (`function_body_check.rs`) returns `Unknown` for the offending expression under the seeded body context; fix the inference for that expression form. Scope strictly to the form the fixture exercises — enumerate, don't broaden.

**Critical files.**
- `crates/smelt-db/src/function_body_check.rs` — `infer_tableexpr_return_schema` explicit-projection inference.
- possibly `crates/smelt-db/src/type_inference/*` — the expression inference for the specific form.

**Docs touched.**
- `docs/specs/function_schema_inference.md` — remove §Known Divergence #3.

**Review checklist:**
- [ ] TDD test reproduces the exact failing form; now green
- [ ] Spec rule 3 satisfied for the form
- [ ] `smelt-db` purity preserved
- [ ] No scope creep; no regression
- [ ] Spec edit is timeless

**Commit.** `fix(types): infer residual body-computed columns in TableExpr function bodies`

---

### Phase 4: `Unknown` reason-discriminant

**Goal.** `Unknown` carries a reason — `Unresolved` (compiler-resolvable gap), `Dynamic` (legitimately unknowable, e.g. `Expr<Any>`), `Propagated` (downstream of an already-`Unknown` input). Meta-language-derived unknowns (generator/reflection/HOF-in-data-position) are classified non-`Unresolved` so Phase 5 does not flag out-of-scope cases. `types.md` §"Strict-by-default doctrine" / Constraints own the discriminant.

**Pre-conditions.** Phases 1–3 (so the function-schema `Unresolved` set in green examples is empty before enforcement).

**TDD tests to write first.**
- `crates/smelt-db/...` unit tests — a cross-family op and an unresolved function-derived column classify as `Unresolved`; an `Expr<Any>` return classifies `Dynamic`; a column whose only defect is an `Unknown` input classifies `Propagated`.
- `crates/smelt-db/...` unit test — a meta-language-derived `Unknown` (reflected/columns_of-style column) does **not** classify `Unresolved`.

**Implementation shape.** Prefer the least-invasive form that satisfies the contract: compute the reason at the schema/diagnostic layer rather than refactoring the `DataType::Unknown` enum variant across the codebase — a column is `Propagated` when a referenced input column is already `Unknown`, `Dynamic` when sourced from an `Expr<Any>`/dynamic call, else `Unresolved`. If a carried discriminant is genuinely needed, attach it to the column/diagnostic metadata, keeping `DataType` churn minimal. Decide and record the chosen representation in the phase commit; do not change user-visible hover (`Unknown` still renders as `Unknown`).

**Critical files.**
- `crates/smelt-types/src/*` and/or `crates/smelt-db/src/{type_inference/*, queries/schema.rs}` — reason classification (kept pure).

**Docs touched.**
- `docs/specs/types.md` — Surface/Semantics already describe the discriminant; reconcile wording if the chosen representation differs from the spec's framing (update the spec, don't drift).

**Review checklist:**
- [ ] TDD tests cover all three reasons + the meta-exclusion
- [ ] `smelt-db`/`smelt-types` purity preserved
- [ ] Hover output unchanged (`Unknown`)
- [ ] No `ColumnTypeUnresolved` emission yet (Phase 5)
- [ ] Spec edit is timeless

**Commit.** `feat(types): classify Unknown by reason (Unresolved/Dynamic/Propagated)`

---

### Phase 5: `ColumnTypeUnresolved` diagnostic (fires by default)

**Goal.** A resolved schema column that is `Unknown` with reason `Unresolved` and originates from a `smelt.functions.*` call emits `ColumnTypeUnresolved` at the projection. `Propagated` and `Dynamic` emit nothing (origin-only). Closes §Known Divergence #4; satisfies invariant 4. No opt-in flag.

**Pre-conditions.** Phases 1–4. Verify green examples have zero function-schema `Unresolved` columns before enabling (else the gate breaks).

**TDD tests to write first.**
- New broken fixture `examples/function_schema_broken_column_type_unresolved/` — a model with a genuinely-unresolvable function-derived column; `crates/smelt-cli/tests/example_diagnostics.rs` (broken-workspace path) asserts it emits exactly `ColumnTypeUnresolved` and no other code.
- `crates/smelt-db/...` unit test — `Propagated` and `Dynamic` columns emit no diagnostic (origin-only).
- Regression: `example_diagnostics` + `example_workspaces` stay green on all in-scope examples (web_analytics, functions_demo).

**Implementation shape.** Mint `DiagnosticCode::ColumnTypeUnresolved` (`crates/smelt-db/src/lib.rs` / `diagnostics_types.rs`); in `file_diagnostics`, walk resolved-schema columns (or the select-item resolution) and emit at the projection span for `Unresolved` function-derived columns. Message names the column and the producing call. Scope the emission to the function-schema domain — do **not** fire on meta-language-derived unknowns (deferred to `meta_language.md`).

**Critical files.**
- `crates/smelt-db/src/diagnostics_types.rs`, `crates/smelt-db/src/lib.rs` — code + emission.
- `examples/function_schema_broken_column_type_unresolved/` — fixture.

**Docs touched.**
- `docs/specs/function_schema_inference.md` — remove §Known Divergence #4; confirm Surface `ColumnTypeUnresolved` description matches.
- `docs/specs/types.md` — remove the silent-`Unknown` divergence entry.
- `docs-site/docs/reference/language.md` — note that an unresolved function-derived column type is a `ColumnTypeUnresolved` error.

**Review checklist:**
- [ ] Broken fixture emits exactly `ColumnTypeUnresolved`
- [ ] Origin-only: `Propagated`/`Dynamic` silent
- [ ] Meta-language examples (per_cohort_union, meta_columns, meta_*) still green — not flagged
- [ ] Spec rule 6 + invariant 4 satisfied
- [ ] Spec/docs edits timeless
- [ ] `smelt-db` purity preserved

**Commit.** `feat(diagnostics): emit ColumnTypeUnresolved for unresolved function-derived columns`

---

### Phase 6: User-docs reconciliation

**Goal.** `docs-site/docs/reference/language.md` documents the `smelt.functions.<f>(...).*` schema-projection surface and the `ColumnTypeUnresolved` behavior, consistent with the spec Surface.

**Pre-conditions.** Phases 1, 5 (the documented behaviors exist).

**TDD tests to write first.** Docs phase — no code tests. Verification is `/smelt:validate function_schema_inference` reporting no Surface/docs drift.

**Implementation shape.** Add a short section to `language.md` under the `smelt.functions` surface: struct-returning functions, `.*` projection into columns, and the unresolved-column error. Keep it timeless.

**Critical files.**
- `docs-site/docs/reference/language.md`.

**Docs touched.**
- `docs-site/docs/reference/language.md` — the above.
- `docs/specs/function_schema_inference.md` — confirm §References → User docs reconciled.

**Review checklist:**
- [ ] Surface items all documented; nothing in docs absent from spec
- [ ] `/smelt:validate function_schema_inference` reports no Surface/docs drift
- [ ] Timeless phrasing

**Commit.** `docs(site): document smelt.functions struct-spread schema projection and ColumnTypeUnresolved`

---

## Deferred during implementation

(Append-only.)

- **Phase 1 — `.field` single-field projection and row-tail (`..r`) struct-spread descoped.** The call surface has no field-postfix on a function call (`SMELT_PATH_CALL` only carries `.*` via `SMELT_PATH_CALL_STAR`), so `f(x).field` is not implementable without parser work. Row-tail struct-spread expansion at the schema layer diverges from codegen (`expand_smelt_path_call_star` falls back to verbatim on `SPREAD_ITEM`), which would violate the schema/codegen-agreement invariant. Phase 1 ships closed-struct `.*` only; both sub-cases are recorded in the spec's Known Divergences. Unifying schema + codegen for row-tail, and adding `.field`, are future work.

## Verification

The spec is satisfied at the end when:
- `target/debug/smelt type --project-dir examples/web_analytics` shows no `UNKNOWN` (events_parsed/eventstream/sessions all resolved).
- `target/debug/smelt type --project-dir examples/functions_demo` shows no `UNKNOWN`.
- `cargo test -p smelt-cli --test example_diagnostics` — green (incl. the new broken fixture asserting `ColumnTypeUnresolved`).
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` — green.
- `/smelt:validate function_schema_inference` and `/smelt:validate types` — zero drift on closed entries.
