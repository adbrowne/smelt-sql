# Plan: SQL-level diagnostics for generator emission bodies

**Date**: 2026-05-29
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md)
**Spec diff**: working-tree only — the §Known Divergences entry "SQL-level inference gaps inside emission bodies do not emit diagnostics" is deleted as the final step of Phase 3. No §Surface or §Semantics change; the spec already names the no-silent-`Unknown` invariant via `types.md` §"Strict-by-default doctrine".
**Tracking PR / branch**: branch `worktree-unknown_types`.
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/meta_language.md` §"Multi-model production" — specifically rules 4 (W4), 6 (body type-check regime), and 7 (`columns:`), and the §Known Divergences entry tracking this work. Read `docs/specs/types.md` §"Strict-by-default doctrine" — this is the correctness oracle for the no-silent-`Unknown` invariant.
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next `pending` phase in the Progress table. If all are `done`, run Verification and stop.

**Per-phase loop (`/smelt:implement`):** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule (update the spec via `/smelt:spec` first).
- A pre-existing failure unrelated to this plan surfaces.
- A pure helper that Phase 1 needs to extract turns out to be deeply entangled with Salsa in a way the plan's mechanical refactor doesn't anticipate — flag before widening.

**Conventions every phase:**
- Real-fixture coverage at every phase; new broken fixtures live under `examples/per_cohort_union_broken_*` matching the existing naming pattern from Phase E2.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commit using the `Commit.` line verbatim; push after each.
- Honor `CLAUDE.md` invariants: `smelt-db` pure-function rule; Workspace-Loading-Parity Rule; Project Isolation Rule.
- **Timeless-oracle rule.** Phase vocabulary lives in this plan only. The spec edit in Phase 3 is a deletion of one §Known Divergences entry — no new phase vocabulary lands in the spec.

---

## Context

`docs/specs/meta_language.md` §Known Divergences currently records: "`check_file_diagnostics` skips SQL diagnostics for generator files … A `ModelDef.body` containing an undeclared column reference, type mismatch, or other SQL-level inference gap produces a partially-typed `ModelSchema` via `emitted_model_typed_schema` but no diagnostic." This violates `types.md` §"Strict-by-default doctrine" for generator-emitted models. The body SQL is already re-parsed and typed once per emission inside `synthesise_emission_schema` (`crates/smelt-db/src/queries/project.rs`); this plan extends that pass to also produce diagnostics, surfaces them through `check_file_diagnostics` for generator files, and propagates per-emission lambda-parameter bindings so lifted identifiers like `c.region` in `examples/per_cohort_union/models/cohorts.gen.sql` continue to resolve cleanly.

## Scope

### In scope (spec coverage)
- `meta_language.md` §Known Divergences — the "SQL-level inference gaps inside emission bodies do not emit diagnostics" entry is closed and deleted.
- `meta_language.md` §"Multi-model production" rule 6 — emission body type-checking is brought to parity with hand-authored model body type-checking, including diagnostic surfacing.
- `types.md` §"Strict-by-default doctrine" — the no-silent-`Unknown` invariant is satisfied for generator-emitted models.

### Explicitly deferred
- Hover / goto-def / completion on emission-body diagnostic locations through the LSP. The pure helpers produce diagnostics with correct generator-file ranges; the LSP backend already routes `file_diagnostics` through `Backend::publish_diagnostics` so these surface in the editor automatically. No new LSP backend dispatch is added.
- Restructuring the existing `_for_file` Salsa wrappers beyond what Phase 1 mechanically requires. If a helper is more entangled with Salsa than expected, leave it as a `_for_file`-only wrapper and skip its body coverage rather than widening the refactor.
- Expansion-time-validated lifted-identifier diagnostics (e.g. detecting that `c.region` references a record field that does not exist in the loader schema). That gap is tracked separately in `meta_language.md` §Known Divergences ("Lift-scope validation at body-check time is suppressed…") and is not closed by this plan.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 8c0e859d | 2026-05-29 |
| 2     | done     | d42a5fca | 2026-05-29 |
| 3     | done     | 5e99419d | 2026-05-29 |

---

### Phase 1: Extract pure `_for_select` shape from SQL-level diagnostic helpers

**Goal.** The SQL-level diagnostic check helpers in `smelt-db` that today live as `_for_file(db, workspace, file)` Salsa wrappers each gain a sibling pure function of shape `_for_select(select_stmt: &SelectStmt, ctx: &TypeContext, text: &str, range_offset: usize) -> Vec<Diagnostic>` (or the natural variant for each — some take `&AstFile` if they walk multiple statements). The existing `_for_file` wrappers become thin adapters that build inputs from Salsa and delegate to the new pure helpers. No behavioural change for hand-authored models — `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` stay green.

**Pre-conditions.** None.

**TDD tests to write first.**
1. `crates/smelt-db/src/queries/check_types.rs::tests` — extend each existing test that exercises a `_for_file` helper to ALSO call the new `_for_select` helper directly with the same inputs and assert identical diagnostic output. Cases: at minimum, the UndeclaredColumn test, the CannotInferType test, the TypeMismatch test, the CTE-cycle test, the `smelt.config.var` test, the `smelt.fn.*` expansion test. (Each is a 5-line addition next to the existing case.)
2. Regression: `cargo test -p smelt-cli --test example_diagnostics` (~79 cases) stays green; `cargo test -p smelt-lsp --test example_workspaces` (~21 cases) stays green; `cargo test -p smelt-db` stays green.

**Implementation shape.**
- For each `_for_file` helper currently called from `check_file_diagnostics` (and from `check_type_diagnostics`), extract the body that walks the SELECT into a sibling pure function. Concrete starting list (verify against current source — extract whichever subset is mechanically tractable):
  - `check_undeclared_columns` is already pure — its `_for_file` callers can be reused as-is; no extraction needed.
  - `check_expression_types` (via `check_type_diagnostics`) — extract into `check_expression_types_for_select`.
  - `CannotInferType` walk in `check_type_diagnostics` — extract into a pure helper that takes a `ModelSchema` and produces the warnings, since it already operates on schema-shaped data.
  - `cte_cycle_diagnostics_for_file` — extract into `cte_cycle_diagnostics_for_select`.
  - `check_config_var_call_diagnostics` is already pure (takes `&syntax, &vars_map, text`) — verify and reuse as-is.
  - `smelt_fn_call_diagnostics_for_file` — extract into `smelt_fn_call_diagnostics_for_select`.
  - `loader_call_diagnostics_for_file` — extract; loader calls inside an emission body are valid and should be checked.
- Each pure helper takes `(SelectStmt | &AstFile, &TypeContext, text: &str, range_offset: usize)`. The `range_offset` parameter is `0` for hand-authored callers; emission-body callers (Phase 2+) pass `body_span.start().into()`. Each helper applies `range + range_offset` before calling `text_range_to_range(generator_or_model_text, shifted)`.
- The `_for_file` Salsa wrapper becomes: gather inputs from Salsa, call the `_for_select` helper with `range_offset = 0` and `text = file.text(db)`, return its result. Behavior identical to today.
- Pure-function rule preserved: extracted helpers take all inputs as plain data; no `&dyn salsa::Database` in pure code.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/check_types.rs` — extract `check_expression_types_for_select`, `cannot_infer_type_for_schema`.
- `crates/smelt-db/src/queries/cte_cycle.rs` (or wherever `cte_cycle_diagnostics_for_file` lives) — extract `cte_cycle_diagnostics_for_select`.
- `crates/smelt-db/src/queries/smelt_fn_expansion.rs` (or wherever `smelt_fn_call_diagnostics_for_file` lives) — extract `smelt_fn_call_diagnostics_for_select`.
- `crates/smelt-db/src/queries/loader.rs` — extract `loader_call_diagnostics_for_select`.
- `crates/smelt-db/src/lib.rs` — no logic change; only re-exports if the new helpers need to be visible to Phase 2.

**Docs touched.** None — this phase is a pure refactor with no user-visible surface change. The plan's spec touch lands in Phase 3.

**Review checklist (material findings only):**
- [ ] Every `_for_file` helper extracted has a TDD test asserting `_for_select` output equals `_for_file` output for the same case.
- [ ] No behavioral change on hand-authored models (regression gates green).
- [ ] `smelt-db` pure-function rule preserved (no `&dyn salsa::Database` in extracted helpers).
- [ ] Each pure helper accepts a `range_offset: usize` parameter; the value is plumbed into the `TextRange` → `Range` conversion before output.
- [ ] No scope creep into Phases 2 or 3.

**Commit.** `refactor(db): extract pure _for_select shape from SQL-level diagnostic helpers`

---

### Phase 2: Pure body-analysis helper + Salsa query + lambda-binding propagation

**Goal.** `synthesise_emission_schema` becomes `synthesise_emission_body_analysis(body_sql, body_offset, refs, legacy_sources, per_entity_sources, all_function_sigs, lambda_bindings) -> EmissionBodyAnalysis { schema: ModelSchema, diagnostics: Vec<Diagnostic> }`. A new Salsa query `emitted_model_body_analysis(workspace, generator_file, name) -> Arc<EmissionBodyAnalysis>` wraps it. `emitted_model_typed_schema` becomes a `.map(|a| a.schema.clone())` adapter over the new query — its existing call sites are unchanged. `EmittedModelDef` gains a `lambda_bindings: Vec<(String, SmeltType)>` field populated by `evaluate_generator` from the type of the lambda parameter active at body-construction time, so `c.region`-style lifts in bodies resolve to typed projections rather than triggering `UndeclaredColumn`. The diagnostics from the new query are not yet surfaced through `check_file_diagnostics` — Phase 3 wires that — but the query is unit-tested end-to-end here.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**
1. `crates/smelt-db/tests/emitted_model_body_diagnostics.rs` (new integration test). Cases — all assert on `emitted_model_body_analysis(db, ws, generator_file, name).diagnostics`:
   - **UndeclaredColumn inside body** (no lift): `ModelDef { name: 'x', body: SELECT foo_does_not_exist FROM smelt.orders }` produces exactly one `UndeclaredColumn` anchored at the body-local position of `foo_does_not_exist`, translated to generator-file coordinates.
   - **Expression TypeMismatch inside body**: a `WHERE` clause comparing `Integer + 'string'` produces the expected `TypeMismatch` at the offending expression.
   - **CannotInferType at SELECT-list**: a body whose SELECT list cannot infer one column produces exactly one `CannotInferType` warning anchored at the SELECT-list item's position.
   - **ParseError on malformed body**: a body like `body: SELEKT 1` (typo) produces a single `ParseError` diagnostic anchored at `body_span`. The returned `schema` is `ModelSchema::empty()`.
   - **CTE-cycle inside body**: a body whose `WITH a AS (SELECT * FROM a)` produces the expected CTE-cycle diagnostic.
   - **`smelt.config.var` error inside body**: a body referencing an unknown config var produces `ConfigVarNotFound`.
   - **Lift-protection regression**: `examples/per_cohort_union/models/cohorts.gen.sql` (which uses `WHERE region = c.region AND revenue >= c.min_revenue`) produces *zero* `UndeclaredColumn` diagnostics on any of its three emissions. This is the headline lift-protection assertion.
   - **Multi-emission independence**: a generator emitting two `ModelDef`s, one with a body error and one without, produces diagnostics only for the erroring emission. The two are independent.
2. `crates/smelt-db/src/queries/project.rs::tests` (or sibling) — `EmittedModelDef.lambda_bindings` is populated correctly for the `cohorts.gen.sql` shape: each survivor carries `[("c", Record { fields: [("name", Text), ("region", Text), ("min_revenue", Integer)] })]`.
3. Regression: `cargo test -p smelt-cli --test example_diagnostics` stays green; `cargo test -p smelt-lsp --test example_workspaces` stays green; `examples/per_cohort_union/` continues to produce typed columns end-to-end via the existing typed-schema path (Phase 1 of the prior plan's behaviour is preserved through the `emitted_model_typed_schema` adapter).

**Implementation shape.**
- **`EmittedModelDef.lambda_bindings`**: extend the struct in `crates/smelt-db/src/queries/project.rs:337` with a new field. Populate inside `evaluate_generator` at the W2 stage: when the generator body is `pipeline_driver |> map(fn c => ModelDef { … })` shape (the only driver currently supported per the existing `evaluate_body_emissions` divergence), the lambda parameter binding type is the element type of the pipeline driver's output. The list-of-records type from `smelt.config.load_yaml(path, List<{...}>)` already flows through W2's type inference; capture the element type as the binding for the parameter name in each emitted `EmittedModelDef`. Multiple lambda parameters (if a multi-arg lambda is ever the driver shape) bind in parameter-list order.
- **`EmissionBodyAnalysis`**: new struct in `crates/smelt-db/src/queries/project.rs`, fields `{ schema: ModelSchema, diagnostics: Vec<Diagnostic> }`. Both fields are owned; the wrapping `Arc` is added at the Salsa query layer.
- **`synthesise_emission_body_analysis`**: extract the existing `synthesise_emission_schema` body, add diagnostic collection. After `build_type_context(&ast, legacy_sources, refs)` and `add_source_info_to_type_context`, seed the lambda bindings: for each `(param_name, param_type)`, register a qualified identifier in `TypeContext` whose type is the parameter's record type, so `c.region` projects cleanly through the existing record-field-projection path. Then run each pure check helper from Phase 1: `check_undeclared_columns`, `check_expression_types_for_select`, `cannot_infer_type_for_schema`, `cte_cycle_diagnostics_for_select`, `check_config_var_call_diagnostics`, `smelt_fn_call_diagnostics_for_select`, `loader_call_diagnostics_for_select`. Each takes `range_offset = body_offset`. Append parse errors from the body re-parse first (with `body_offset` shift). If body re-parse produces no `SelectStmt` (malformed beyond recovery), emit a single `ParseError` anchored at `body_offset..body_offset` and return `(ModelSchema::empty(), vec![diag])`.
- **`emitted_model_body_analysis`** Salsa query: same input-gathering shape as today's `emitted_model_typed_schema` (lines 1711-1763 of `project.rs`), plus lookup of `lambda_bindings` from the matching `EmittedModelDef`. Delegates to the pure helper. Returns `Arc<EmissionBodyAnalysis>`. Salsa-tracked.
- **`emitted_model_typed_schema`** becomes: `let analysis = emitted_model_body_analysis(db, workspace, generator_file, name); Arc::new(analysis.schema.clone())`. Cheap clone of `ModelSchema`. Behavior identical for consumers.
- **Discarded emissions**: the new query is only meaningful for `survivors`, but it cannot fail safely on a `discarded` name — discarded emissions are not in `evaluated.emissions`. Returning an empty `EmissionBodyAnalysis` (matching the existing `emitted_model_typed_schema` fallback at line 1721) is correct; Phase 3 ensures the query is only invoked for survivors.
- **TypeContext seeding for lambda bindings**: the existing `TypeContext` has facilities for record-typed identifiers (used by record-field-projection elsewhere in the codebase). Add a method `register_lambda_binding(name: &str, ty: SmeltType)` or reuse an existing record-binding registration path. The seeding must take effect *before* `check_undeclared_columns` runs.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/project.rs` — `EmittedModelDef.lambda_bindings`, `EmissionBodyAnalysis`, `synthesise_emission_body_analysis`, `emitted_model_body_analysis`, `emitted_model_typed_schema` adapter.
- `crates/smelt-db/src/type_inference/type_context.rs` (or wherever `TypeContext` lives) — lambda-binding registration helper, if not already present.
- `crates/smelt-db/src/type_inference/multi_model.rs` — lambda-parameter-type extraction during W2 evaluation (if the binding type is not already plumbed through).
- `crates/smelt-db/src/lib.rs` — export the new query / struct if Phase 3 needs them.
- `crates/smelt-db/tests/emitted_model_body_diagnostics.rs` — new integration test.

**Docs touched.**
- None in this phase. The §Known Divergences entry is deleted in Phase 3 once the user-visible behavior actually surfaces.

**Review checklist (material findings only):**
- [ ] TDD tests drive the real `emitted_model_body_analysis` Salsa query (not a sub-helper).
- [ ] Lift-protection assertion: `examples/per_cohort_union/` produces zero spurious `UndeclaredColumn` from any of its three emissions.
- [ ] `EmittedModelDef.lambda_bindings` populated for the `|> map(fn c => …)` driver shape; multi-arg lambda support is correct or explicitly deferred (note in commit body if so).
- [ ] `emitted_model_typed_schema` adapter returns identical `ModelSchema` to today's direct query (no behavioural change for consumers).
- [ ] `smelt-db` pure-function rule preserved.
- [ ] Caching shape correct: a change to an upstream model's schema invalidates the body analysis through the existing edges; a change to a non-referenced upstream does not.
- [ ] Discarded emissions handled gracefully (empty `EmissionBodyAnalysis` returned for non-`survivor` names).

**Commit.** `feat(types): synthesise diagnostics alongside schemas for generator-emitted bodies`

---

### Phase 3: Wire diagnostics through `check_file_diagnostics` + close the divergence

**Goal.** `check_file_diagnostics`, in the `Ok(FileMetadata::Generator { .. })` branch (around line 833 of `crates/smelt-db/src/lib.rs`), iterates each surviving emission filtered to the current generator file and accumulates `emitted_model_body_analysis(db, workspace, file, name).diagnostics` into `DiagnosticAcc` after the existing W2/W3 diagnostic emission, before the existing early `return`. Discarded emissions are skipped (they don't ship). New broken-emission-body example fixtures cover each headline diagnostic kind. `examples/per_cohort_union/` continues to produce zero diagnostics end-to-end. The §Known Divergences entry on `meta_language.md` is deleted.

**Pre-conditions.** Phases 1 and 2 done.

**TDD tests to write first.**
1. `crates/smelt-cli/tests/example_diagnostics.rs` — add cases for the new broken fixtures:
   - `examples/per_cohort_union_broken_emission_body_undeclared_column/` — generator body references a column that does not exist on `smelt.orders`. Asserts exactly one `UndeclaredColumn` diagnostic, anchored inside the generator file at the body position.
   - `examples/per_cohort_union_broken_emission_body_parse_error/` — generator body contains a SQL syntax error. Asserts a `ParseError` anchored at `body_span`.
   - `examples/per_cohort_union_broken_emission_body_cte_cycle/` — generator body's `WITH` clause is cyclic. Asserts the expected CTE-cycle diagnostic.
2. `crates/smelt-lsp/tests/example_workspaces.rs` — the same broken fixtures, validated via the real LSP backend. This is the standing gate for asymmetric-discovery bugs (CLAUDE.md Workspace-Loading-Parity Rule).
3. Regression: `examples/per_cohort_union/` produces zero diagnostics end-to-end (the headline positive case from the prior plan stays clean). `examples/staging_from_sources/` (the other live generator fixture) likewise stays clean.
4. **Discarded-emission suppression**: a new fixture `examples/per_cohort_union_broken_emission_body_collision_suppression/` declares a generator that emits two `ModelDef`s with the same name, where one body also contains an `UndeclaredColumn`. Asserts: exactly one `ModelDefHandAuthoredCollision`-class diagnostic (or `ModelDefDuplicateName`, whichever W3 emits for emission-vs-emission collision); zero body diagnostics for the discarded emission.

**Implementation shape.**
- In `crates/smelt-db/src/lib.rs` around line 891 (immediately after the W3 collision-diagnostic loop, before the early `return` at line 900):
  ```text
  for survivor in &emitted_result.survivors {
      if survivor.generator_file != gen_file_path { continue; }
      let analysis = emitted_model_body_analysis(
          db, workspace, file, survivor.name.clone(),
      );
      for diag in analysis.diagnostics.iter() {
          DiagnosticAcc(diag.clone()).accumulate(db);
      }
  }
  ```
  (`survivors` are the only emissions that ship; discarded emissions are skipped naturally because they aren't in `survivors`.)
- The broken-fixture example workspaces follow the existing `examples/per_cohort_union_broken_*` pattern: one `smelt.yml`, one generator file with the trigger condition, the same upstream `models/orders.sql` as `examples/per_cohort_union/`.
- Delete the §Known Divergences entry from `docs/specs/meta_language.md` (the entry quoted in the Context section of this plan).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/lib.rs` — `check_file_diagnostics` integration (the ~10-line loop above).
- `examples/per_cohort_union_broken_emission_body_undeclared_column/` — new fixture.
- `examples/per_cohort_union_broken_emission_body_parse_error/` — new fixture.
- `examples/per_cohort_union_broken_emission_body_cte_cycle/` — new fixture.
- `examples/per_cohort_union_broken_emission_body_collision_suppression/` — new fixture.
- `crates/smelt-cli/tests/example_diagnostics.rs` — assertions for the new fixtures.
- `crates/smelt-lsp/tests/example_workspaces.rs` — assertions for the new fixtures.
- `docs/specs/meta_language.md` — delete the one §Known Divergences entry.

**Docs touched.**
- `docs/specs/meta_language.md` — delete the §Known Divergences entry "SQL-level inference gaps inside emission bodies do not emit diagnostics". No other spec edit.
- No `docs-site/` edit. The user-facing surface — "generator-emitted models are type-checked identically to hand-authored models" — is already described in `docs/specs/meta_language.md` and `docs-site/`. The behavior change is that previously-silent gaps now surface as diagnostics; that is a quality improvement, not a new user feature.

**Review checklist (material findings only):**
- [ ] Diagnostics appear in the editor at correct positions inside the generator file body (verified via `examples/per_cohort_union_broken_*` + LSP test).
- [ ] `examples/per_cohort_union/` continues to produce zero diagnostics end-to-end (no spurious lift-related diagnostics).
- [ ] Discarded emissions do not produce body diagnostics.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` green.
- [ ] `cargo test -p smelt-lsp --test example_workspaces` green.
- [ ] §Known Divergences entry deleted; `/smelt:validate meta_language` reports no drift on this divergence.
- [ ] Project Isolation Rule respected: a broken-fixture project in one workspace folder does not leak diagnostics into another project's emissions.

**Commit.** `feat(types): surface SQL-level diagnostics for generator emission bodies`

---

## Deferred during implementation

(Append-only.)

- Expansion-time-validated lifted-identifier diagnostics: tracked in `meta_language.md` §Known Divergences ("Lift-scope validation at body-check time is suppressed…"); not closed by this plan.
- Restructuring of `_for_file` Salsa wrappers beyond what Phase 1 mechanically requires: if a helper is more entangled than expected, leave it as-is and document the gap.
- `body_position_to_byte` (the helper inside `synthesise_emission_body_analysis` that backs `remap_body_range` for CTE-cycle and `smelt.config.var` diagnostics) advances by one codepoint per character rather than `ch.len_utf8()` bytes. For non-ASCII emission bodies this would miscount the byte offset and shift those diagnostics to slightly wrong line/columns. All current fixtures are ASCII so no test exercises the path; a one-line fix (replace `+= 1` with `+= ch.len_utf8()`) will land when a non-ASCII fixture surfaces.

## Verification

- `target/debug/smelt type --project-dir examples/per_cohort_union` — no `UNKNOWN` columns; the three emissions and `all_cohorts_unioned` all type concretely (regression of prior plan's headline assertion).
- `target/debug/smelt type --project-dir examples/per_cohort_union_broken_emission_body_undeclared_column` — emits exactly the expected `UndeclaredColumn` diagnostic at the body position.
- `cargo test -p smelt-cli --test example_diagnostics` — green.
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo test -p smelt-db --test emitted_model_body_diagnostics` — green.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` — green.
- `/smelt:validate meta_language` — no drift on the deleted §Known Divergences entry; no new divergence introduced.
