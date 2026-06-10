# Plan: Nullability Soundness (Type-System Axes, axis 1 of 4)

**Date**: 2026-06-10
**Spec**: [`docs/specs/types.md`](../specs/types.md) §11 Nullability
**Spec diff**: uncommitted working tree (sound-upper-bound contract, outer-join / set-operation rules, value-based verification gate)
**Tracking PR / branch**: `worktree-type_system`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/types.md` §11 — it is the correctness oracle. Do not re-open settled spec decisions (the sound-upper-bound contract, the oracle-first approach, and the deferred items were agreed with the user on 2026-06-10).
2. Confirm you are on branch `worktree-type_system`. If not, ask the user before continuing.
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
- Honor architectural invariants from `CLAUDE.md` (Salsa purity: inference fixes are pure functions; Salsa queries stay thin wrappers).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/types.md` and `docs-site/docs/...` describe the feature as if it has always existed. If a phase ships an incomplete surface, the *spec* records the gap under **Known Divergences** in behavioural terms.

**Plan-specific conventions:**
- **Oracle-red handling.** When the property test (Phase 1 onward) finds a violation: minimise the failing case, add an explicit regression test capturing it *before* fixing, then fix. If the violation traces to a construct owned by a *later* phase (joins → Phase 2/3, set ops → Phase 3, rule audit → Phase 4), record it under "Deferred during implementation" with the minimised SQL and move on — do not fix ahead of its phase.
- **No divergence registry for this oracle.** The soundness property is one-sided: conservative (`nullable: true`) answers are always acceptable; only a `nullable: false` claim contradicted by data is a defect. Every red is a bug to fix, never an entry to whitelist.

---

## Context

First axis of ROADMAP item 4 (Type-System Axes), establishing the template the decimal / timezone / collation axes will follow: a contract in the spec plus a value-based DuckDB oracle as the standing gate. Spec §11 now defines `nullable: false` as a hard guarantee (sound upper bound) and enumerates the only non-nullable-claiming rules; the implementation predates that contract and is known-unsound on outer joins (spec Known Divergences). Fingerprint folding is deliberately not here — it lands with ROADMAP item 5.

## Scope

### In scope (spec coverage)
- §11 sound-upper-bound contract: audit and fix every rule that claims `nullable: false`.
- §11 outer-join rule: null-supplying-side columns become nullable (LEFT → right side, RIGHT → left, FULL → both; INNER/CROSS preserve).
- §11 set-operation rule: output column non-nullable only if non-nullable in every branch.
- §11 verification gate: value-based DuckDB nullability property test as a standing CI gate.
- §11 signature nullability: the `NOT NULL` qualifier in `smelt.define` parameter/return types and `TableExpr` rows, with call-site/return/row checking and non-nullable parameter binding.
- Surface §Hover: `NOT NULL` suffix display via one canonical type renderer shared by hover + diagnostics.
- ROADMAP item 4 sequencing (nullability → decimal → timezone → collation) and item 5 fingerprint-fold follow-up.

### Explicitly deferred
- **Fingerprint folding** — `output_fingerprint.md` keeps nullability breaking-by-default until the fingerprint is wired into the runtime (ROADMAP item 5; Phase 7 records the follow-up there). The fold must hash structured `TypedColumn`, not display strings (spec Known Divergences).
- **Precision improvements** — `WHERE x IS NOT NULL` narrowing, join-key non-null reasoning, flow-sensitive refinement. Pure precision wins permitted later without a contract change (spec Known Divergences).
- **Nested composite nullability** — struct fields and array elements stay conservatively nullable; the cross-engine intersection (only Spark tracks nested). Extension point is the composite `DataType` variants (spec Design §"Axis placement"); `NOT NULL` is rejected on struct-field/array-element annotation positions.
- **Spark oracle for nullability** — the gate is DuckDB-only for now, matching the existing type property tests' primary oracle.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     |        | 2026-06-11 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |

---

### Phase 1: Value-based nullability oracle (single-table surface)

**Goal.** Land the standing property-test gate: generated single-table queries over generated data (nullable columns actually populated with NULLs), executed on DuckDB, asserting no output column smelt infers `nullable: false` ever contains a NULL.

**Pre-conditions.** Spec §11 contract committed (this plan's first commit includes the spec diff). Existing `prop_helpers` harness (`duckdb_oracle.rs`, `generators.rs`) compiles and `type_property_tests` is green.

**TDD tests to write first.**
- `crates/smelt-db/tests/nullability_property_tests.rs::prop_nullability_sound` — the property: for generated `(schema, data, query)` triples, every column smelt infers non-nullable contains zero NULLs in DuckDB results. Runtime-erroring queries are discarded (same policy as `type_property_tests.rs`).
- `crates/smelt-db/tests/nullability_property_tests.rs::smoke_coalesce_non_nullable_holds` — deterministic smoke: `COALESCE(nullable_col, 0)` inferred non-nullable, and DuckDB returns no NULLs (green path proving the harness can pass).
- `crates/smelt-db/tests/nullability_property_tests.rs::smoke_nullable_column_passthrough` — deterministic smoke: a nullable column projected as-is stays `nullable: true` (proves NULL-bearing data generation works — the test must observe actual NULLs in results).

**Implementation shape.** New `prop_helpers` module (e.g. `null_data.rs`): generate per-column data with high NULL density for `nullable: true` columns and zero NULLs for `nullable: false` columns; CREATE TABLE + INSERT through the existing DuckDB oracle connection. Query generator: reuse/extend `generators.rs` expression generation over one table, weighting NULL-relevant constructs (`COALESCE`, `NULLIF`, `TRY_CAST`, `CASE` with and without `ELSE`, aggregates with `GROUP BY`). Assertion helper compares smelt's inferred `ModelSchema` nullability against observed NULLs per column. Honor `PROPTEST_CASES` like the existing suite.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/nullability_property_tests.rs` — new test
- `crates/smelt-db/tests/prop_helpers/` — new data-generation module; extensions to `generators.rs`, `duckdb_oracle.rs`, `mod.rs`
- `crates/smelt-db/src/type_inference/*` — only if a single-table oracle red is fixed in-phase (each with its explicit regression test)
- `CLAUDE.md` — add the test command to the Type Property Tests section

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: narrow the "verification gate not yet landed" entry to whatever genuinely remains (joins/set-ops coverage arrives in Phase 3); References → Tests gains the new test path.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] The property is value-based (observed NULLs), not Arrow-schema-based, per spec §11
- [ ] NULL-bearing data generation is verified by a test, not assumed
- [ ] Any in-phase inference fix has an explicit regression test added before the fix
- [ ] Reds owned by later phases recorded under "Deferred during implementation", not fixed
- [ ] Spec edits are timeless — no phase vocabulary

**Commit.** `test(types): value-based DuckDB nullability soundness oracle (single-table)`

---

### Phase 2: Outer-join nullability soundness fix

**Goal.** Implement spec §11's outer-join rule: columns from the null-supplying side(s) of an outer join become nullable in the join's output scope, closing the known soundness hole.

**Pre-conditions.** Phase 1 oracle exists (used to confirm the fix class, even though join generation lands in Phase 3).

**TDD tests to write first.**
- `crates/smelt-db/src/type_inference/tests.rs::left_join_right_side_columns_nullable` — a `nullable: false` column from the right table of a `LEFT JOIN` infers `nullable: true` in the output schema.
- `crates/smelt-db/src/type_inference/tests.rs::right_join_left_side_columns_nullable` — mirror for `RIGHT JOIN`.
- `crates/smelt-db/src/type_inference/tests.rs::full_join_both_sides_nullable` — both sides nullable under `FULL JOIN`.
- `crates/smelt-db/src/type_inference/tests.rs::inner_join_preserves_nullability` — `INNER JOIN` keeps `nullable: false` (no precision regression).
- `crates/smelt-db/tests/nullability_property_tests.rs::regression_left_join_declared_not_null` — real-fixture: a model in `examples/test_workspace/` LEFT-JOINing a source column declared `nullable: false`; inferred schema marks it nullable, and the DuckDB value-check agrees.

**Implementation shape.** In `crates/smelt-db/src/queries/schema.rs`: `process_from_clause_pure` / `process_table_ref_pure` gain join-type awareness (`smelt_parser::ast::JoinType` is already parsed). LEFT: force-nullable the joined table's columns as they bind. RIGHT: force-nullable all columns bound so far on the left. FULL: both. Keep the logic in the pure helpers (Salsa purity); the `TypeContext` may need a "mark scope columns nullable" operation in `type_inference/type_context.rs`. Mirror the same handling in `function_body_check.rs` if `TableExpr` function bodies bind joins through a separate path.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/schema.rs` — join-type-aware scope binding
- `crates/smelt-db/src/type_inference/type_context.rs` — force-nullable operation
- `crates/smelt-db/src/type_inference/tests.rs` — unit tests
- `crates/smelt-db/tests/nullability_property_tests.rs` — regression test
- `examples/test_workspace/` — fixture model exercising LEFT JOIN over a `nullable: false` source column

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: remove the "outer-join nullability is unsound today" entry (the rule-audit remainder stays until Phase 4).
- `docs-site/docs/reference/sources-yml.md` — `nullable` field row: note that the declared guarantee is honored through inference, and outer joins make the null-supplied side nullable downstream.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec §11 outer-join rule satisfied for LEFT / RIGHT / FULL; INNER / CROSS preserve
- [ ] Multi-join chains handled (e.g. `a LEFT JOIN b INNER JOIN c` — b stays nullable)
- [ ] Salsa purity honored — fix lives in pure functions
- [ ] No scope creep into set-operation handling (Phase 3)
- [ ] Spec + docs-site edits are timeless

**Commit.** `fix(types): outer-join null-supplying side columns infer nullable`

---

### Phase 3: Oracle coverage for joins and set operations

**Goal.** Extend the property-test generators to two-table joins (INNER / LEFT / RIGHT / FULL) and set operations (UNION / INTERSECT / EXCEPT), locking Phase 2 under the oracle and verifying the set-operation rule (non-nullable only if non-nullable in every branch).

**Pre-conditions.** Phases 1–2 done.

**TDD tests to write first.**
- `crates/smelt-db/tests/nullability_property_tests.rs::prop_nullability_sound_joins` — property over generated two-table join queries of all four join types.
- `crates/smelt-db/tests/nullability_property_tests.rs::prop_nullability_sound_setops` — property over generated `UNION [ALL]` / `INTERSECT` / `EXCEPT` queries with branch-varying nullability.
- `crates/smelt-db/tests/nullability_property_tests.rs::smoke_union_mixed_nullability` — deterministic: `non_nullable UNION ALL nullable` infers nullable; `non_nullable UNION ALL non_nullable` infers non-nullable and DuckDB shows no NULLs.

**Implementation shape.** Generator extensions in `prop_helpers/generators.rs` (or a sibling module): join-query generation with ON predicates engineered so outer joins actually produce non-matching rows (the null-supplying side must be exercised — e.g. disjoint key ranges); set-op generation reusing single-table SELECT generation with type-aligned branches. If the set-operation reds show smelt over-claims non-nullable, fix the branch-combination rule in inference (explicit regression test first, per convention).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/prop_helpers/` — generator extensions
- `crates/smelt-db/tests/nullability_property_tests.rs` — new properties + smokes
- `crates/smelt-db/src/type_inference/*`, `crates/smelt-db/src/queries/schema.rs` — set-op nullability combination fix if red

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: the verification-gate entry is removed entirely (gate now covers the §11 surface).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Join generation provably exercises non-matching rows (NULL-supplied side observed in data)
- [ ] Set-operation rule from spec §11 satisfied in inference
- [ ] No divergence-registry-style whitelisting introduced — every red fixed or minimised + deferred with rationale
- [ ] Spec edits are timeless

**Commit.** `test(types): nullability oracle covers joins and set operations`

---

### Phase 4: Non-nullable-claim audit

**Goal.** Enumerate every code site that can produce `nullable: false` and verify each against spec §11's closed list of non-nullable origins; fix violators (each with an explicit regression test). This discharges the spec's "audit pending" divergence.

**Pre-conditions.** Phases 1–3 done (the oracle catches regressions while audit fixes land).

**TDD tests to write first.** (Per finding — the list below seeds the audit; each confirmed violation gets a deterministic regression test in `nullability_property_tests.rs` or `type_inference/tests.rs` before its fix.)
- `crates/smelt-db/tests/nullability_property_tests.rs::regression_case_without_else_nullable` — `CASE WHEN p THEN non_nullable END` infers nullable.
- `crates/smelt-db/tests/nullability_property_tests.rs::regression_try_cast_nullable` — `TRY_CAST(non_nullable AS T)` infers nullable.
- `crates/smelt-db/tests/nullability_property_tests.rs::regression_nullif_nullable` — `NULLIF(non_nullable, x)` infers nullable.
- `crates/smelt-db/tests/nullability_property_tests.rs::regression_lag_without_default_nullable` — `LAG(non_nullable) OVER (...)` infers nullable.
- Audit-completeness check: `rg` sweep for `nullable: false` / non-nullable constructors across `crates/smelt-db/src/` and `crates/smelt-types/src/`, mapped site-by-site to a §11 origin; the mapping table goes in the phase's commit message or a review note, not the spec.

**Implementation shape.** Mechanical audit: every constructor/assignment producing a non-nullable `TypedColumn` (literal inference, builtin registry returns in `signatures.rs::BuiltinRegistry`, `CAST`, `COALESCE`/`CASE` handling in `ternary.rs`/`function_call.rs`/`dispatch.rs`) is justified by a §11 origin or fixed. Builtin-registry returns that hardcode non-nullable (e.g. anything beyond `COUNT`) are prime suspects.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/type_inference/*` — rule fixes
- `crates/smelt-types/src/signatures.rs` — builtin-registry nullability fixes
- `crates/smelt-db/tests/nullability_property_tests.rs`, `crates/smelt-db/src/type_inference/tests.rs` — regression tests

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: remove the audit-pending clause; §11 lists corrected if the audit shows the spec's enumeration itself was wrong (spec edit comes first if so, per spec-first rule).

**Review checklist** (material findings only):
- [ ] Every confirmed violation has a regression test that predates its fix
- [ ] Audit covered all non-nullable production sites (sweep documented)
- [ ] No precision chasing — fixes only flip false→true (soundness), never true→false (precision; deferred)
- [ ] Spec §11 enumeration and implementation agree at phase end
- [ ] Spec edits are timeless

**Commit.** `fix(types): audit non-nullable claims against §11 sound-upper-bound contract`

---

### Phase 5: Signature nullability (`NOT NULL` qualifier)

**Goal.** Implement spec §11 "Signature nullability": the `NOT NULL` qualifier on `smelt.define` parameter/return types and `TableExpr` row columns, with one-way subtyping (non-nullable <: nullable), call-site/return/row checking, and non-nullable parameter binding in function bodies.

**Pre-conditions.** Phase 4 done — the qualifier's checking is only honest once the `nullable` flags it compares against are sound.

**TDD tests to write first.**
- `crates/smelt-parser/...::parses_not_null_qualifier_on_expr_param` — `Expr<Integer NOT NULL>` parses in parameter and return positions; `TableExpr<{id: Integer NOT NULL}>` parses in row positions.
- `crates/smelt-parser/...::rejects_not_null_on_struct_field` — `Expr<Struct<{a: Integer NOT NULL}>>` is a parse/check error (nested positions excluded per spec §11).
- `crates/smelt-db/tests/function_body_check.rs::not_null_param_rejects_nullable_argument` — passing a nullable column to an `Expr<T NOT NULL>` parameter emits `ArgTypeMismatch` with a nullability-aware message.
- `crates/smelt-db/tests/function_body_check.rs::not_null_param_accepts_non_nullable_argument` — and the parameter binds non-nullable inside the body (a body returning the param satisfies a `NOT NULL` return).
- `crates/smelt-db/tests/function_body_check.rs::not_null_return_rejects_nullable_body` — body synthesising nullable against a `NOT NULL` return emits `ReturnTypeMismatch`.
- `crates/smelt-db/tests/tableexpr_arg_shapes.rs::not_null_row_column_requires_non_nullable_caller` — `TableExpr<{id: Integer NOT NULL}>` vs a nullable caller column emits `RowRequirementUnsatisfied`.
- Real fixture: a function in `examples/test_workspace/` declaring a `NOT NULL` parameter, called with a `nullable: false` source column — diagnostic-clean via `example_diagnostics`.

**Implementation shape.** Parser: accept the `NOT NULL` qualifier in type-annotation positions (lexer already knows the keywords; the annotation grammar gains an optional trailing qualifier). Representation: signature types carry the flag — extend where `SmeltType`/`FunctionSig` parameter entries hold the data type (likely a `nullable: bool` alongside, mirroring `TypedColumn`). Checking: `unify_call` / body-check paths compare argument `TypedColumn.nullable` against the declared flag with the one-way rule; parameter seeding (`add_function_param`) honors the flag. Bare annotations default nullable — zero behaviour change for existing signatures.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/` — annotation grammar
- `crates/smelt-types/src/signatures.rs` — signature representation + `unify_call`
- `crates/smelt-db/src/type_inference/type_context.rs`, `function_body_check.rs` — binding + checking
- `crates/smelt-db/tests/`, `examples/test_workspace/` — tests + fixture

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: narrow the "signature nullability not yet implemented" entry to the hover/renderer remainder (Phase 6).
- `docs-site/docs/guide/functions.md` (and reference page if signatures are documented there) — document the `NOT NULL` qualifier with an example, written timelessly.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] One-way subtyping only — no nullable-to-non-nullable acceptance anywhere
- [ ] Existing unqualified signatures behave identically (no default flip)
- [ ] Nested positions reject the qualifier
- [ ] Reused diagnostic codes, no new ones (spec §11)
- [ ] Docs-site edits are timeless

**Commit.** `feat(types): NOT NULL qualifier in smelt.define signatures`

---

### Phase 6: Canonical type renderer + hover display

**Goal.** One canonical type renderer shared by hover and diagnostics; non-nullable columns display as `T NOT NULL`, nullable as bare `T` (spec Surface §Hover).

**Pre-conditions.** Phase 4 done (never display an unsound claim); Phase 5 done (display notation matches writable syntax).

**TDD tests to write first.**
- `crates/smelt-db/tests/...::hover_shows_not_null_for_non_nullable_column` — hovering a `nullable: false` source column renders `Integer NOT NULL`.
- `crates/smelt-db/tests/...::hover_bare_type_for_nullable_column` — nullable column renders bare `Integer`.
- `crates/smelt-db/tests/...::hover_left_join_column_drops_not_null` — real fixture: the Phase 2 LEFT-JOIN example's null-supplied column hovers without `NOT NULL` (end-to-end: declared non-null source → join → display).
- Renderer unit test: nullability-aware messages in `ArgTypeMismatch`/`ReturnTypeMismatch` use the same renderer output.

**Implementation shape.** Locate the existing hover type-rendering path (smelt-db hover/LSP layer); extract/confirm a single render function over `TypedColumn` (not `DataType` alone) and route hover + the Phase 5 diagnostic messages through it. Tracked axes appear in one place; future axes (collation) extend the renderer once.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/` hover/rendering path, `crates/smelt-lsp/src/` if hover formatting lives there
- `crates/smelt-db/tests/`, `crates/smelt-lsp/tests/` — display tests

**Docs touched.**
- `docs/specs/types.md` — Known Divergences: remove the signature-nullability/hover entry entirely.
- `docs-site/docs/` LSP/editor page — hover shows nullability, one line + screenshot-free description.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Single renderer — no second format string for types anywhere in hover/diagnostics paths
- [ ] Display matches writable syntax exactly (`NOT NULL` suffix, bare = nullable)
- [ ] Docs edits are timeless

**Commit.** `feat(lsp): canonical type renderer; hover displays NOT NULL`

---

### Phase 7: Roadmap sequencing + closure

**Goal.** Record the agreed Type-System-Axes execution shape in the roadmap and close out the cycle's documentation.

**Pre-conditions.** Phases 1–6 done; `cargo test -p smelt-db --test nullability_property_tests` green.

**TDD tests to write first.** None (docs-only phase).

**Implementation shape.** ROADMAP item 4: record per-axis sequencing — one axis end-to-end at a time, order **nullability → decimal → timezone → collation** — with nullability marked ✅ (date), decimal next (citing `docs/research/20260516-decimal-type-system.md` as its design input, needing a spec cycle to ratify the portable-surface position), timezone then collation (collation needs a research doc first). ROADMAP item 5: add the explicit "fold tracked axes into the output fingerprint" step, citing the §11 contract as the precondition. Note the oracle gate command alongside the existing property-test commands.

**Critical files (allowed to touch in this phase).**
- `docs/ROADMAP.md` — item 4 + item 5 updates
- `docs/specs/types.md` — final Known Divergences pass (fingerprint-fold and precision entries remain, accurately worded)

**Docs touched.**
- `docs/ROADMAP.md` (roadmap vocabulary is fine there; it is not spec body)
- `docs/specs/types.md` — as above, timeless

**Review checklist** (material findings only):
- [ ] ROADMAP item 4 records sequencing + nullability completion with date
- [ ] ROADMAP item 5 gains the fingerprint-fold step
- [ ] Remaining spec divergences are accurate (fingerprint fold, precision coarseness)
- [ ] No phase vocabulary leaked into spec body

**Commit.** `docs(roadmap): type-system axes sequencing; nullability axis complete`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-db --test nullability_property_tests` — the standing gate, green (run locally with `PROPTEST_CASES=1000` for deeper coverage)
- `cargo test -p smelt-db --test type_property_tests` — existing type oracle unaffected
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` — examples stay diagnostic-clean (the LEFT JOIN fixture included)
- `cargo fmt --all -- --check` and `cargo clippy --all-targets` clean
- `/smelt:validate types` reports zero drift on §11
