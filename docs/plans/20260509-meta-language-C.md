# Plan: Meta-Language Phase C — Narrow reflection: `smelt.columns_of`, `ColumnRef`

**Date**: 2026-05-10
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md) §"Phase C — Narrow reflection: `smelt.columns_of`, `ColumnRef`"; cross-touched in [`docs/specs/expansion.md`](../specs/expansion.md) §"Frame-stack invariants" (`column_origin` extension to anonymous frame), [`docs/specs/lsp.md`](../specs/lsp.md) §"Per-feature LSP obligations" (Phase C constructs)
**Spec diff**: commit `3ec025d` (`spec(meta-language-C): author Phase C surface, semantics, design, invariants`) on branch `research/typed-meta-programming`
**Tracking PR / branch**: PR #117 — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this plan in full. Then read the spec at `docs/specs/meta_language.md` §"Phase C" (Surface, Semantics, Design, Invariants, Known Divergences) plus the cross-spec touches in `expansion.md` and `lsp.md` — they are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec` to revise the spec rather than encode the divergence in code.
2. Confirm you are on branch `research/typed-meta-programming`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 7 is the expert-reviewer dispatch loop** — after Phases 1–6 commit, dispatch the meta-plan §5 expert reviewers applicable to this phase, address material findings, and re-dispatch each expert until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 7. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 7's acceptance gate is met.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` first to update).
- `cargo test` or `cargo clippy --all-targets` surfaces a pre-existing failure unrelated to the plan.
- Phase 7: an expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in the same round.

**Conventions every phase:**

- Real-fixture tests under `examples/meta_columns/` — every phase from Phase 5 onward exercises its feature there; earlier phases have unit tests in `crates/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no wide reflection (`smelt.models.*`, `smelt.sources.*`), no records / `Map<K, V>`, no multi-model production, no parameterised reducers, no user-writable record surface — those are Phases D, E1, E2, F.
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference.rs` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic). The `smelt.columns_of` Salsa query lives in `crates/smelt-db/src/lib.rs` and is called only from the orchestration layer; the inference rule receives resolved schema as a parameter.

---

## Context

The meta-language Phase C spec increment landed in commit `3ec025d`. The spec authors `smelt.columns_of(t: TableExpr) -> List<ColumnRef>` (the narrow-reflection accessor), the closed `ColumnRef` meta-record type with three fields (`name: Text`, `type: DataType`, `is_numeric: Boolean`), the meta-`Text`-as-identifier lift (narrow rule, four enumerated grammar positions), four new diagnostic codes (`ColumnsOfRequiresTableExpr`, `ColumnsOfNamedArgument`, `ColumnsOfUnresolvableSchema`, `ColumnRefFieldUnknown`), and a `column_origin` extension to the anonymous expansion frame from Phase B. This plan drives the implementation, examples, user docs, and skill update for that surface. It is the third of seven implementation phases (A–G); it must land cleanly because Phase D's wide reflection (`smelt.models.*`) and Phase E2's multi-model production both reuse the per-call-site schema-resolution machinery and the meta-`Text`-as-identifier lift this phase commits to.

## Scope

### In scope (spec coverage)

- `meta_language.md` §"Phase C — Surface" — `smelt.columns_of` accessor, `ColumnRef` meta record type, meta-`Text`-as-identifier lift (narrow rule, four positions), four new diagnostic codes, LSP obligations.
- `meta_language.md` §"Per-phase semantic rules" Phase C — eleven normative rules covering Salsa-cached purity, body-check vs expansion-time evaluation, source-schema resolution, field projection, ordering, the lift narrowness rule, non-constructibility, determinism, termination, and the `column_origin` frame extension.
- `meta_language.md` §"Phase C invariants" — preserved as architectural invariants policed by the implementation.
- Four new Phase C diagnostic codes (`ColumnsOfRequiresTableExpr`, `ColumnsOfNamedArgument`, `ColumnsOfUnresolvableSchema`, `ColumnRefFieldUnknown`).
- `expansion.md` cross-spec touch — `column_origin` extension to the anonymous-frame form (a per-element optional source-column span on the HOF frame, populated when the source list comes from `smelt.columns_of`).
- `lsp.md` cross-spec touch — Phase C's per-construct LSP obligations (hover for `smelt.columns_of`, ColumnRef field projection, lifted identifiers; completion for the closed field set and `columns_of` argument positions; goto-def from lifted identifier to source column declaration).
- LSP hover for `smelt.columns_of`, `ColumnRef`-typed bindings, field projections (`c.name` / `c.type` / `c.is_numeric`), and lifted identifiers.
- LSP goto-def on a lifted identifier to the source column's declaration site (when statically traceable).
- LSP completion for the closed `ColumnRef` field set and for in-scope `TableExpr`-valued names at `smelt.columns_of`'s argument position.
- Examples fixture `examples/meta_columns/` covering happy path (`coalesce_numeric` + schema-driven select) plus at least one diagnostic edge case for each new Phase C code, gated by `crates/smelt-cli/tests/example_diagnostics.rs`.
- User docs at `docs-site/docs/meta-language/{reflection,reference}.md`.
- `smelt-app-builder` skill: per-phase reference doc.
- `/smelt-loop` `medium` tier: at least one Phase C-specific ask (e.g. "use `smelt.columns_of` and `filter` to coalesce all numeric columns of a model").

### Explicitly deferred

- Wide reflection (`smelt.models.*`, `smelt.sources.*`, `ModelRef`, `SourceRef`) — Phase D.
- Records, `Map<K,V>`, schema-typed config loaders (`smelt.config.load_yaml`, `smelt.record Name = { … }`) — Phase E1.
- Multi-model production (`generates: models`, `ModelDef`, generator file body shape) — Phase E2.
- Wider meta-`Text`-as-identifier lift positions (CTE names, table aliases, function names, model paths) — explicit decision for the dial to widen only under concrete pressure; Phase E2's multi-model production may add model-path component lift, but the Phase C surface stays narrow.
- Parameterised reducers, multi-arg lambdas, ternary — Phase F.
- LSP rename support for ColumnRef field projections / lifted identifiers — Phase G.
- `c.is_ordered`, `c.is_temporal`, `c.is_string`, `c.nullable` — speced as derivations on `c.type` membership / column metadata; shipped only if examples force them. The Phase C field set is exactly `{name, type, is_numeric}`.
- `smelt.columns_of` accepting a string or path argument (e.g. `smelt.columns_of('orders')`) — single-axis surface (TableExpr-only) per spec design rationale.
- Runtime `Expr<Text>` lifting to identifier — speced as out of scope (the lift is meta-only, not a runtime mechanism).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 4e891c4 | 2026-05-11 |
| 2     | done     | 386d455 | 2026-05-11 |
| 3     | done     | 1eeff24 | 2026-05-11 |
| 4     | done     | 9c7ef7e | 2026-05-11 |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |

---

### Phase 1: Type system — `ColumnRef` witness + `smelt.columns_of` builtin + field projection (pure)

**Goal.** Add the `ColumnRef` `SmeltType` witness with a closed three-field set (`name: Text`, `type: DataType`, `is_numeric: Boolean`); the `smelt.columns_of(t: TableExpr) -> List<ColumnRef>` entry in the built-in registry with positional-only argument validation; and pure type-inference for ColumnRef field projection (closed-set lookup). Three Phase C diagnostics fire from this phase: `ColumnsOfRequiresTableExpr`, `ColumnsOfNamedArgument`, `ColumnRefFieldUnknown`. The fourth diagnostic (`ColumnsOfUnresolvableSchema`) is expansion-time and lands in Phase 3. Pure functions only — no Salsa calls inside `type_inference.rs` or `signatures.rs`. The internal witness shape (`SmeltType::ColumnRef` variant vs an internal `Record` instantiation against a constant `COLUMN_REF_FIELDS` registry) is an implementation choice, subject to the closed-field invariant from the spec.

**Pre-conditions.** Phase B done at commit `7380a94`. Working tree clean. `cargo test`, `cargo clippy --all-targets`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**TDD tests to write first.**

- `crates/smelt-types/src/signatures.rs::tests::columns_of_signature_returns_list_of_column_ref` — `BuiltinRegistry::lookup("smelt.columns_of")` returns a signature with one positional `TableExpr` parameter and `List<ColumnRef>` return.
- `crates/smelt-types/src/signatures.rs::tests::column_ref_field_set_is_closed` — the constant `COLUMN_REF_FIELDS` (or equivalent registry) exposes exactly `{name: Text, type: DataType, is_numeric: Boolean}` and no other field; lookup of any other identifier returns `None`.
- `crates/smelt-db/src/type_inference.rs::tests::columns_of_arg_must_be_table_expr` — `smelt.columns_of(42)` synthesises `List<ColumnRef>` (recoverable) and emits exactly one `ColumnsOfRequiresTableExpr` at the `42` argument span; `smelt.columns_of(<smelt.<path>>)` emits no Phase C diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::columns_of_rejects_named_argument` — `smelt.columns_of(t => orders)` emits exactly one `ColumnsOfNamedArgument` at the `t => orders` span; `smelt.columns_of(orders)` does not.
- `crates/smelt-db/src/type_inference.rs::tests::column_ref_field_projection_synthesises_field_type` — given a binding `c: ColumnRef`, `c.name` synthesises `Text`, `c.type` synthesises `DataType` (meta literal), `c.is_numeric` synthesises `Boolean`.
- `crates/smelt-db/src/type_inference.rs::tests::column_ref_field_projection_rejects_unknown_field` — given a binding `c: ColumnRef`, `c.foo` emits exactly one `ColumnRefFieldUnknown` at the `foo` field token span and synthesises `Unknown` (drop-on-error per gradual_typing).

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs`:
  - `SmeltType::ColumnRef` variant (or alternative internal witness — see goal note). Display impl shows `ColumnRef`.
  - `pub const COLUMN_REF_FIELDS: &[(&str, ColumnRefFieldType)]` (or equivalent struct array) with the three fields. Lookup helper `column_ref_field(name: &str) -> Option<ColumnRefFieldType>`.
  - `BuiltinRegistry` entry for `smelt.columns_of` (one positional `TableExpr` param, `List<ColumnRef>` return, no variadics, no named args).
- `crates/smelt-db/src/type_inference.rs`:
  - Pure inference for `smelt.columns_of(arg)`: dispatch through the existing `smelt.<path>` resolution to detect this as a builtin call; check positional-only (emit `ColumnsOfNamedArgument` at any named arg); check arg type assignable to `TableExpr` (emit `ColumnsOfRequiresTableExpr` if not). Always synthesise `List<ColumnRef>` (recoverable) so downstream HOF type-check still works.
  - Pure inference for ColumnRef field projection: when synthesising a field-access expression `e.f` where `e` synthesises to `ColumnRef`, look up `f` in `COLUMN_REF_FIELDS`; on hit, synthesise the field's type; on miss, emit `ColumnRefFieldUnknown` at the field span and synthesise `Unknown`.
- `crates/smelt-db/src/lib.rs::DiagnosticCode`:
  - Three new variants: `ColumnsOfRequiresTableExpr`, `ColumnsOfNamedArgument`, `ColumnRefFieldUnknown`. Message templates per spec §"Diagnostic codes (new in Phase C)".

**Critical files (allowed to touch in this phase).**

- `crates/smelt-types/src/signatures.rs` — `SmeltType::ColumnRef` (or equivalent), `COLUMN_REF_FIELDS`, `BuiltinRegistry` entry.
- `crates/smelt-types/src/lib.rs` — only if a re-export is needed.
- `crates/smelt-db/src/type_inference.rs` — pure inference for `columns_of` arg-checking and ColumnRef field projection.
- `crates/smelt-db/src/lib.rs` — `DiagnosticCode` variants and `Display` impl entries; NOT the Salsa query (Phase 3).
- `crates/smelt-types/src/signatures.rs::tests` and `crates/smelt-db/src/type_inference.rs::tests` — the unit tests above.

**Docs touched.**

- None new in this phase. The spec rules cited above are normative; this phase makes them executable.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `SmeltType` addition is non-breaking (no missed exhaustive matches across crates; check with `cargo clippy --all-targets`).
- [ ] `COLUMN_REF_FIELDS` is the single source of truth for the v1 field set; no field name appears as a string literal outside the registry definition (closed-registry invariant).
- [ ] `smelt.columns_of` built-in entry has signature exactly `(TableExpr) -> List<ColumnRef>` — no variadic, no named-arg surface.
- [ ] `type_inference.rs` and `signatures.rs` remain pure (no `db.` Salsa calls).
- [ ] Three diagnostics anchor at the correct CST span (named-arg span, argument expression span, field token span respectively).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-C): ColumnRef witness + smelt.columns_of builtin + field projection (pure)`

---

### Phase 2: Meta-`Text`-as-identifier lift (narrow rule, 4 positions)

**Goal.** Implement the meta-`Text`-as-identifier lift detection and transformation for the four enumerated grammar positions (column-reference inside expression, AS-alias of a SELECT item, ORDER BY column-reference, GROUP BY column-reference). The lift is a compile-time-only operation: when a meta-`Text`-valued expression (in Phase C, the only producer is a `ColumnRef.name` field projection) appears in one of the four positions, the type checker recognises the lift, the lifted identifier is then validated against the surrounding splice-context column-resolution scope (existing `UnknownColumn` rule), and the produced SQL renders the identifier verbatim. Runtime `Expr<Text>` values do not lift; in any other position, a meta-`Text` retains its `Text` value. No new diagnostic codes are introduced in this phase — the lift produces the existing `UnknownColumn` when the lifted identifier names no in-scope column.

**Pre-conditions.** Phase 1 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/type_inference.rs::tests::lift_in_column_reference_position_resolves_to_column` — given a HOF body whose `c: ColumnRef` is bound and the surrounding splice context exposes columns `{name: Text, amount: Numeric}`, `c.name` (where `c.name` evaluates at compile time to `"name"`) in column-reference position resolves as the column `name` and produces `Expr<Text>` (or whatever the column's actual type is). With the same setup, `c.foo` (where the lifted text is `"foo"` not in scope) emits `UnknownColumn` at the lifted source span.
- `crates/smelt-db/src/type_inference.rs::tests::lift_in_as_alias_position_renders_identifier` — given the same binding, `SUM(amount) AS c.name` lifts `c.name` as an alias identifier; the SELECT item's output column name equals the resolved text (no `UnknownColumn` because aliases introduce names rather than reference them).
- `crates/smelt-db/src/type_inference.rs::tests::lift_in_order_by_position_resolves_to_column` — `ORDER BY c.name` lifts; the resolved column must exist (`UnknownColumn` otherwise).
- `crates/smelt-db/src/type_inference.rs::tests::lift_in_group_by_position_resolves_to_column` — `GROUP BY c.name` lifts; same scope rule as ORDER BY.
- `crates/smelt-db/src/type_inference.rs::tests::no_lift_in_function_argument_position` — `UPPER(c.name)` does **not** lift; `c.name` retains `Text` value and `UPPER` synthesises `Expr<Text>` per the existing built-in. (No `UnknownColumn` even if the lifted text would name no column.)
- `crates/smelt-db/src/type_inference.rs::tests::no_lift_for_runtime_expr_text` — a runtime `Expr<Text>` (e.g. `UPPER('foo')`) in column-reference position does not lift; existing splice-context rules emit a Data-World type error.
- `crates/smelt-db/src/type_inference.rs::tests::lift_only_for_compile_time_meta_text` — a `Text`-valued runtime expression that is not a meta-evaluable projection (e.g. a SQL string literal `'foo'` in the same position) does not lift; the expression remains a runtime `Text` literal and triggers the existing splice-context error path.

**Implementation shape.**

- `crates/smelt-db/src/type_inference.rs`:
  - Add a position predicate `is_lift_position(ctx: &SpliceContext) -> bool` that returns true exactly in the four enumerated positions. The position is determined by the calling context's splice-context tag (column-reference, AS-alias, ORDER BY, GROUP BY); other positions return false.
  - Add a `is_meta_text_value(expr: &Expr, ctx: &TypeContext) -> bool` predicate that returns true only when `expr` is a compile-time-resolvable meta-`Text` value (in Phase C: a `ColumnRef.name` field projection on a binding traceable to a HOF lambda parameter sourced from `smelt.columns_of`). Runtime `Expr<Text>` (e.g. function-call results) returns false.
  - When both predicates fire at a splice point, the type checker re-enters column resolution with the lifted identifier. Existing `UnknownColumn` resolution handles the in-scope check; the lift adds no new diagnostic of its own.
  - Per spec §6 / §7 of Phase C semantics: the lift is invisible to the type system except as the identity transform `Text → identifier`. Producing the SQL rendering happens in Phase 3 (expansion-time materialisation); this phase adds the type-check-time decision.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/type_inference.rs` — lift position predicate, meta-text-value predicate, integration into existing column-resolution path for the four positions.
- `crates/smelt-db/src/type_inference.rs::tests` — the unit tests above.
- No new files; no diagnostic-code additions; no parser changes; no Salsa queries.

**Docs touched.**

- None new in this phase. The lift narrowness rule and lift positions table from spec §"Meta-`Text`-as-identifier lift (narrow rule)" are normative; this phase makes them executable.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] Lift fires in exactly the four spec-enumerated positions; no other position lifts.
- [ ] Lift fires only for compile-time meta-`Text` values; runtime `Expr<Text>` does not lift (verified by negative test).
- [ ] Lifted identifier validation reuses the existing `UnknownColumn` resolution path; no new diagnostic code is introduced.
- [ ] `type_inference.rs` purity preserved (no Salsa imports in the lift predicates).
- [ ] No regression in Phase A / B tests (existing `Text`-in-`Expr<Text>`-position cases unchanged).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-C): meta-Text-as-identifier lift in 4 enumerated positions`

---

### Phase 3: Expansion-time materialisation + `column_origin` frame extension + `ColumnsOfUnresolvableSchema`

**Goal.** Wire `smelt.columns_of` to materialise the concrete `List<ColumnRef>` at expansion time using a Salsa query that resolves the source schema via the existing `ModelSchema` machinery. Add the `column_origin` extension to the anonymous expansion frame from Phase B (a per-element optional source-column span). Add the fourth Phase C diagnostic `ColumnsOfUnresolvableSchema` for cases where the schema cannot be statically resolved (with drop-on-error recovery — the surrounding splice drops without further diagnostics, same policy as `MetaSpreadInForbiddenPosition`). Land the `expansion.md` cross-spec touch normatively.

**Pre-conditions.** Phases 1–2 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-db/src/lib.rs::tests::columns_of_salsa_query_resolves_smelt_path_schema` — given a workspace with model `orders(id: Integer, amount: Decimal)`, the `columns_of_for_table_expr(<smelt.<path:orders>>)` Salsa query returns three columns? no — returns two columns in declaration order with name and DataType correctly populated; `is_numeric` derived from `types.md` Numeric constraint membership.
- `crates/smelt-db/src/lib.rs::tests::columns_of_invalidates_when_upstream_schema_changes` — modifying the upstream model's schema invalidates the `columns_of` query result (Salsa-cache invariant); re-evaluation produces the new schema.
- `crates/smelt-db/src/function_body_check.rs::tests::columns_of_unresolvable_schema_drops_with_diagnostic` — given a `smelt.columns_of(t)` whose `t` resolves to an `Unknown` schema (e.g. an upstream model with a parse error), the surrounding HOF call emits `ColumnsOfUnresolvableSchema` at the `columns_of` call site and the splice drops without cascading diagnostics.
- `crates/smelt-db/src/function_body_check.rs::tests::columns_of_hof_lambda_carries_column_origin_frame` — given a HOF body `map(smelt.columns_of(orders), fn c => COALESCE(c.name, 0))` that surfaces a diagnostic from inside the lambda (e.g. a type mismatch on `COALESCE`'s second arg per a particular column), the resulting frame stack includes a Phase B anonymous frame with `function = "map"` and a Phase C extension `column_origin = Some(span_of_column_in_source_schema)` — the optional source-column span on the per-element entry.
- `crates/smelt-db/src/function_body_check.rs::tests::columns_of_expansion_preserves_source_ordering` — `smelt.columns_of(t)` produces ColumnRef values in the declared column order of `t`'s schema; reordering is forbidden.
- `crates/smelt-db/src/function_body_check.rs::tests::columns_of_in_table_expr_parameter_uses_call_site_schema` — given a `smelt.define f(t: TableExpr)` whose body calls `smelt.columns_of(t)`, expansion at a call site `f(orders)` materialises the columns of `orders` (call-site schema), not the parameter's declared row-tail.

**Implementation shape.**

- `crates/smelt-db/src/lib.rs`:
  - New Salsa query `columns_of_for_table_expr(table_expr: TableExprId) -> Result<Vec<ColumnRef>, UnresolvableSchema>`. Takes the resolved TableExpr-id, resolves to a `ModelSchema` via the existing schema-resolution path, projects each `TypedColumn` into a ColumnRef value (`name = column.name`, `type = column.data_type`, `is_numeric = column.data_type.is_numeric()` per `types.md` §"Type constraints"). Returns `Err(UnresolvableSchema)` when the underlying resolution returns `Unknown`.
  - `DiagnosticCode::ColumnsOfUnresolvableSchema` new variant; message per spec.
- `crates/smelt-db/src/function_body_check.rs`:
  - Extend the anonymous expansion frame from Phase B with an optional `column_origin: Option<TextRange>` field (or equivalent shape, subject to `expansion.md`'s Phase C cross-spec touch).
  - At HOF call-site materialisation, when the source list is the result of `smelt.columns_of(t)`: per element, look up the column's source span in `t`'s `ModelSchema` and stamp the resulting frame's `column_origin`. The Salsa query produces the columns; the frame producer reads the spans alongside.
  - On `ColumnsOfUnresolvableSchema`: emit the diagnostic and drop the surrounding splice (the HOF call's result is empty / absent at this position, mirroring the spread drop-on-error policy).
- `docs/specs/expansion.md` Phase C cross-spec touch — register the `column_origin` extension to the anonymous-frame form (per-element optional source-column span). Brief note that producers populate it when the source list comes from a reflection accessor; v1 LSP renderer surfaces it as an optional trailer.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/lib.rs` — `columns_of_for_table_expr` Salsa query, `ColumnsOfUnresolvableSchema` variant, `Display` impl entry.
- `crates/smelt-db/src/function_body_check.rs` — frame extension, expansion-time materialisation, drop-on-error handling.
- `crates/smelt-db/src/lib.rs::tests`, `crates/smelt-db/src/function_body_check.rs::tests` — the unit tests above.
- `docs/specs/expansion.md` — Phase C cross-spec touch (one paragraph in the relevant frame-stack section, plus the `column_origin` field added to the anonymous-frame schema description).

**Docs touched.**

- `docs/specs/expansion.md` — Phase C cross-spec touch as described above. The meta-language spec already describes the extension; this codifies it in the expansion spec.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `columns_of_for_table_expr` is a Salsa query (correctly memoised, correctly invalidated on upstream-schema change — verified by `columns_of_invalidates_when_upstream_schema_changes`).
- [ ] `column_origin` field is producer-side populated for `columns_of`-sourced lists; absent (`None`) for other anonymous frames (no regression in Phase B frame producers).
- [ ] `ColumnsOfUnresolvableSchema` drops the splice cleanly (no cascading diagnostics from inside the HOF body when the source list cannot be materialised).
- [ ] Source ordering preserved (call-site schema's declared order, not alphabetical or some other canonicalisation).
- [ ] `expansion.md` Phase C touch describes the extension normatively (frame field, when populated, what the renderer surfaces).
- [ ] `type_inference.rs` and `signatures.rs` purity preserved.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-C): expansion-time materialisation + column_origin frame + ColumnsOfUnresolvableSchema`

---

### Phase 4: LSP — hover, goto-def, completion for Phase C constructs

**Goal.** Add LSP support for every Phase C surface element per spec §"LSP support required by Phase C": hover for `smelt.columns_of`, ColumnRef-typed bindings, field projections (`c.name` / `c.type` / `c.is_numeric`), and lifted identifiers; goto-def from a lifted identifier to the source column's declaration site; completion for the closed ColumnRef field set and for in-scope `TableExpr`-valued names at `smelt.columns_of`'s argument position. Land the `lsp.md` cross-spec touch.

**Pre-conditions.** Phases 1–3 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-lsp/src/lib.rs::tests::hover_on_smelt_columns_of_call_shows_list_column_ref` — hovering on `smelt.columns_of(orders)` returns `List<ColumnRef>` plus, when the schema is statically resolvable, a column-count line and the first five column names.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_column_ref_lambda_parameter_shows_field_set` — hovering on `c` inside `map(smelt.columns_of(t), fn c => …)` shows `ColumnRef` plus the closed field list with each field's type.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_column_ref_field_projection_shows_field_type` — hovering on the `name` token of `c.name` shows `name: Text`; on `type` shows `type: DataType`; on `is_numeric` shows `is_numeric: Boolean`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_on_lifted_identifier_shows_lift_target` — hovering on a `c.name` in column-reference position shows the lift (`Text → identifier`) and the resolved upstream column when traceable.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_from_lifted_identifier_resolves_to_source_column` — go-to-definition on a `c.name` lifted into a column-reference position resolves to the source column's declaration in the upstream model (or source / seed) when statically traceable; no-op otherwise.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_column_ref_field_offers_closed_set` — completion at `c.<cursor>` offers exactly `{name, type, is_numeric}` and no other identifier.
- `crates/smelt-lsp/src/lib.rs::tests::completion_at_columns_of_argument_offers_table_expr_names` — completion at `smelt.columns_of(<cursor>)` offers in-scope `TableExpr`-valued names (`smelt.<path>` references, enclosing function's `TableExpr` parameters) and not arbitrary other names.

**Implementation shape.**

- `crates/smelt-lsp/src/lib.rs`:
  - Hover handler: on `smelt.columns_of` call site, return Phase C hover body. On a `ColumnRef`-typed binding, return the closed field list. On a field projection, return the field's declared type. On a lifted identifier, return the lift target plus the resolved column when traceable.
  - Goto-def handler: extend the existing column-resolution goto-def path to recognise lifted identifiers (compile-time-resolved meta-`Text` in the four lift positions) and route to the source column's declaration span.
  - Completion handler: at `c.<cursor>` where `c: ColumnRef`, offer the closed field set; at `smelt.columns_of(<cursor>)`, offer in-scope `TableExpr`-valued names.
- `docs/specs/lsp.md` Phase C cross-spec touch — register Phase C's per-construct LSP obligations (one short paragraph per LSP feature, mirroring the spec §"LSP support required by Phase C" entries).

**Critical files (allowed to touch in this phase).**

- `crates/smelt-lsp/src/lib.rs` — hover, goto-def, completion handlers for Phase C constructs.
- `crates/smelt-lsp/src/lib.rs::tests` — the unit tests above.
- `docs/specs/lsp.md` — Phase C cross-spec touch as described above.

**Docs touched.**

- `docs/specs/lsp.md` — Phase C cross-spec touch.

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] All four hover paths (smelt.columns_of, ColumnRef binding, field projection, lifted identifier) return the spec'd content.
- [ ] Goto-def from lifted identifier resolves to source column or no-ops gracefully (never panics).
- [ ] Completion offers exactly the closed field set at field-projection sites; no leakage of arbitrary identifiers.
- [ ] Completion at columns_of argument position filters to TableExpr-valued names; no leakage of arbitrary expressions.
- [ ] No regression in Phase A / B LSP paths (hover/goto/completion for lists, lambdas, HOFs, pipes, reducers, config-vars).
- [ ] `lsp.md` Phase C touch lists the per-construct obligations normatively.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**Commit.** `feat(meta-language-C): LSP hover + goto-def + completion for reflection constructs`

---

### Phase 5: Examples fixture + `smelt-app-builder` skill update + `/smelt-loop` medium tier

**Goal.** Add `examples/meta_columns/` exercising the Phase C surface end-to-end (`coalesce_numeric` worked example from research §5.2 and a schema-driven select-list); add at least one broken-fixture sub-workspace per Phase C diagnostic code; extend the `smelt-app-builder` skill with a Phase C reference doc; extend the `/smelt-loop` `medium` tier with a Phase C-specific ask. The `example_diagnostics` test must pass for the clean fixture and report exactly the right diagnostic code on each broken sub-fixture.

**Pre-conditions.** Phases 1–4 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- `crates/smelt-cli/tests/example_diagnostics.rs` — extend to walk `examples/meta_columns/` and its broken sub-fixtures. The clean workspace must report zero diagnostics. Each broken sub-fixture must report exactly one Phase C diagnostic code matching the fixture name (per Phase B's pattern: `examples/meta_columns_broken_columns_of_requires_table_expr/`, `examples/meta_columns_broken_columns_of_named_argument/`, `examples/meta_columns_broken_columns_of_unresolvable_schema/`, `examples/meta_columns_broken_column_ref_field_unknown/`).

**Implementation shape.**

- `examples/meta_columns/` (clean fixture):
  - `smelt.yml` — minimal workspace config.
  - `models/orders.sql` — a small upstream model with a mix of numeric and non-numeric columns (e.g. `id: Integer`, `customer_name: Text`, `amount: Decimal`, `discount: Decimal`).
  - `models/orders_safe.sql` — uses a `coalesce_numeric` function from `functions/coalesce_numeric.sql` (mirroring research §5.2 verbatim).
  - `functions/coalesce_numeric.sql` — `smelt.define coalesce_numeric(t: TableExpr) -> SelectItems<Scalar, t> AS (smelt.columns_of(t) |> filter(fn c => c.is_numeric) |> map(fn c => COALESCE(c.name, 0) AS c.name))`.
  - At least one model exercising a schema-driven SELECT list (e.g. unioning columns by name across a single model — keep narrow, no `smelt.models.*` since Phase D).
- `examples/meta_columns_broken_*/` — one sub-fixture per Phase C diagnostic, narrowly designed so the fixture reports exactly one diagnostic and no cascading errors.
- `.claude/skills/smelt-app-builder/references/20260510-meta-columns.md` — a per-phase reference doc following the Phase A/B precedent (workflow gotchas only; point at user docs for syntax). Capture: the lift narrowness rule (don't expect `c.name` to lift in arbitrary positions), the body-check vs expansion-time evaluation (`smelt.columns_of(t)` synthesises `List<ColumnRef>` parametrically inside a function body but only materialises at the call site), and the closed `ColumnRef` field set.
- `.claude/commands/smelt-loop.md` (or the appropriate fixture file referenced by it) — extend the `medium` tier with a Phase C-specific ask. One concrete shape: "Use `smelt.columns_of` and `filter` to write a `coalesce_numeric` function that COALESCEs every numeric column to 0; apply it to one model in this workspace."

**Critical files (allowed to touch in this phase).**

- `examples/meta_columns/` and `examples/meta_columns_broken_*/` (new directories).
- `crates/smelt-cli/tests/example_diagnostics.rs` — extend the test to gate the new fixture.
- `.claude/skills/smelt-app-builder/references/20260510-meta-columns.md` (new file).
- `.claude/commands/smelt-loop.md` and any associated tier-fixture file under `.claude/commands/` — Phase C ask addition.

**Docs touched.**

- None new in `docs-site/` this phase (Phase 6 is the user-docs phase). Skill ref doc and loop tier are neither user docs nor specs — they are workflow artifacts.

**Review checklist** (material findings only):

- [ ] Clean fixture reports zero diagnostics; every broken sub-fixture reports exactly one Phase C diagnostic with the correct code.
- [ ] Fixture is minimal-but-realistic — no contrived shapes; the `coalesce_numeric` example mirrors research §5.2.
- [ ] Skill ref doc captures workflow gotchas not derivable from user docs (lift narrowness, body-check vs expansion-time).
- [ ] Loop tier ask is solvable with Phase A–C constructs only (no reflection of models, no records).
- [ ] `cargo test -p smelt-cli --test example_diagnostics` green.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` all green.

**Commit.** `feat(meta-language-C): examples/meta_columns + skill ref + smelt-loop medium tier ask`

---

### Phase 6: User docs — `reflection.md` + `reference.md` extension

**Goal.** Author `docs-site/docs/meta-language/reflection.md` covering `smelt.columns_of`, `ColumnRef`, the closed field set, the four-position identifier lift, and the `coalesce_numeric` worked example. Extend `docs-site/docs/meta-language/reference.md` (alphabetical, per Phase A/B precedent) with `smelt.columns_of`, `ColumnRef`, and the lift positions table.

**Pre-conditions.** Phases 1–5 complete and committed. Working tree clean. `cargo test` green.

**TDD tests to write first.**

- None — user docs are not gated by test runners. Verification is by docs-reviewer in Phase 7.

**Implementation shape.**

- `docs-site/docs/meta-language/reflection.md` (new):
  - Concept: reflection is the workspace-introspection API; Phase C ships the narrow accessor.
  - `smelt.columns_of(t: TableExpr) -> List<ColumnRef>` — signature + small example.
  - `ColumnRef` — closed record, three fields with types and meanings.
  - The four-position identifier lift — table of lift positions with one example each, plus the narrowness rule (lift only meta-`Text`, only in the four positions).
  - Worked example: `coalesce_numeric` (mirrors research §5.2 verbatim, plus a paragraph explaining what the type checker sees at body-check vs expansion time).
  - Diagnostics: one paragraph per Phase C diagnostic code with a "what to fix" hint.
  - Pointers to `lists.md` (Phase A) and `hofs.md` / `pipes.md` (Phase B) for the building blocks.
- `docs-site/docs/meta-language/reference.md` (extend):
  - Add `smelt.columns_of` entry (signature, one-line semantics, link to `reflection.md`).
  - Add `ColumnRef` entry (closed field set with types, link to `reflection.md`).
  - Add a "Meta-`Text`-as-identifier lift positions" subsection under reflection or as its own anchor; the four-row table mirrors the spec.
  - Maintain alphabetical order across the page.

**Critical files (allowed to touch in this phase).**

- `docs-site/docs/meta-language/reflection.md` (new file).
- `docs-site/docs/meta-language/reference.md` (extend).
- `docs-site/docs/meta-language/index.md` — only if a brief Phase C entry needs adding to a contents block.

**Docs touched.**

- `docs-site/docs/meta-language/reflection.md` (new).
- `docs-site/docs/meta-language/reference.md` (extend).

**Review checklist** (material findings only):

- [ ] Every Phase C Surface item from `meta_language.md` is documented in `reflection.md` or `reference.md`.
- [ ] No syntax appears in docs that is not speced (no inventing examples that exercise unspeced surface).
- [ ] Every Phase C diagnostic code has a "what to fix" hint in `reflection.md`.
- [ ] Reference page remains alphabetical and complete (Phases A + B + C entries).
- [ ] Worked example matches the `coalesce_numeric` shape from research §5.2 and the Phase 5 fixture.
- [ ] Lift positions table matches the spec exactly (four rows, no others).

**Commit.** `docs(meta-language-C): reflection user docs + reference page extension`

---

### Phase 7: Expert reviewer dispatch loop

**Goal.** Run each Phase C applicable expert reviewer from meta-plan §5 over the Phase C diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below. This phase is the realisation of the user's original ask: "Use expert reviews by subagents with specific context to help guide the implementation."

**Pre-conditions.** Phases 1–6 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase C subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **type-expert** | sonnet | `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/type_inference.rs` | `SmeltType::ColumnRef` (or alternative witness) addition is non-breaking (no missed exhaustive matches); `COLUMN_REF_FIELDS` is the single source of truth for the v1 field set; `smelt.columns_of` builtin signature is exactly `(TableExpr) -> List<ColumnRef>`; ColumnRef field projection inference is closed-set; the meta-`Text`-as-identifier lift fires in exactly the four spec-enumerated positions and only for compile-time meta-`Text`; `type_inference.rs` purity preserved. |
| **lsp-expert** | sonnet | `crates/smelt-lsp/src/lib.rs` | Hover on `smelt.columns_of` / ColumnRef binding / field projection / lifted identifier returns the spec'd content; goto-def from lifted identifier to source column resolves correctly or no-ops gracefully; completion at field-projection site offers exactly the closed field set; completion at `columns_of` argument position offers in-scope TableExpr names; spans line up with CST; no panics on partial parses; no regressions in Phase A/B hover/goto/completion paths. |
| **examples-curator** | haiku | `examples/meta_columns/` and `examples/meta_columns_broken_*/` | Clean fixture is minimal-but-realistic and mirrors `coalesce_numeric` from research §5.2; every Phase C diagnostic code has a corresponding broken sub-fixture; broken fixtures report exactly one Phase C diagnostic with no cascading errors; passes `cargo test -p smelt-cli --test example_diagnostics`. |
| **docs-reviewer** | haiku | `docs-site/docs/meta-language/{reflection,reference,index}.md` | Every Phase C Surface item is documented; every Phase C diagnostic code has a "what to fix" hint; reference page is alphabetical and complete (Phases A+B+C); the lift positions table matches the spec exactly; no syntax in docs that is not speced. |
| **cross-feature-impact-reviewer** | sonnet | meta-plan §6 cross-feature implications table; `docs/specs/{expansion,schema_evolution,lsp}.md` | The meta-plan §6 cross-feature implications row for Phase C is complete after this phase: `expansion.md` Phase C touch lands the `column_origin` extension; `lsp.md` Phase C touch lists per-construct LSP obligations; `schema_evolution.md` records the implication that a column added to a source must propagate to `smelt.columns_of`-sourced HOF outputs (informational note, not normative behaviour change). Confirms no spec touch was missed and no scope expanded beyond the table. |

**Loop discipline.**

1. **Round 1.** Dispatch all five experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - The phase plan path (`docs/plans/20260509-meta-language-C.md`) and the spec sections that are the oracle (`docs/specs/meta_language.md` Phase C, plus `expansion.md` / `lsp.md` / `schema_evolution.md` cross-touches).
   - The exact file scope from the table above.
   - The diff range to review (commits since the start of Phase C — typically `git log --oneline 3ec025d..HEAD`, where `3ec025d` is the spec-increment commit; the implementation commits land after).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into other phases.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-C): address {expert-name} feedback` (e.g. `review(meta-language-C): address type-expert feedback`).
   - Push after each commit (so the user sees progress on PR #117).

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were addressed, not the whole panel. Provide the same prompt as round 1 plus a diff of what changed since round N−1. If the expert returns "no material findings", that expert is **clean** and exits the loop.

4. **Repeat** step 2 → step 3 until **every** expert is clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of the following fires:
   - Same expert flags a material finding on round 3 (per-expert bound). The third repeat means the fix is wrong or the spec is wrong; the user must arbitrate.
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7). That's a design problem, not an implementation problem.
   - An expert's findings would force a spec change. Run `/smelt:spec meta_language` first; if the spec edit is non-trivial or contentious, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase C. Pause; the autonomy loop should not silently absorb pre-existing breakage.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus `docs/plans/20260509-meta-language-C.md` (to record round counts and final clean status).

**Docs touched.** None new — fixes may amend `docs-site/docs/meta-language/*` if the docs-reviewer flags a surface drift; or `docs/specs/expansion.md` / `docs/specs/lsp.md` / `docs/specs/schema_evolution.md` if the cross-feature-impact-reviewer flags a missing or drifted touch.

**Review checklist** (material findings only — applied to the expert-dispatch *process*, not to a code diff):

- [ ] All five experts were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below (see acceptance gate).
- [ ] No fix touched files outside the dispatching expert's scope (no scope creep).
- [ ] No expert ran more than 3 rounds; if any did, the autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 7 expert review: type-expert clean (R{n}), lsp-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}), cross-feature-impact-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-C): address {expert-name} feedback`. If round 1 came back clean for an expert, no commit for that expert. The acceptance-gate summary line lands in the next commit naturally (or in a tiny `chore(meta-language-C): record Phase 7 review summary` if no other phase-7 commits were made).

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 3 — production HOF expansion dispatcher integration (follow-up).**
  Two Phase 3 TDD tests are marked `#[ignore]` because the production HOF expansion dispatcher
  does not yet call `columns_of_for_table_expr`. This is consistent with the Phase B pattern —
  `walk_hof_lambda_body_with_anonymous_frame` is exercised by tests directly, not driven from a
  production `file_diagnostics` orchestrator. The Phase B → Phase C wiring is the same problem,
  and the gap pre-exists Phase 3.
  1. `columns_of_hof_lambda_carries_column_origin_frame` — needs the dispatcher to call
     `columns_of_for_table_expr`, iterate the `ColumnRefValue` list, and pass each
     `ColumnRefValue::source_span` as the `column_origin` argument to
     `walk_hof_lambda_body_with_anonymous_frame_and_origin`.  The frame-stamping mechanism is
     verified by the direct-call form; it's the dispatcher invocation that is deferred.
  2. `columns_of_unresolvable_schema_drops_with_diagnostic` — the drop-on-error invariant requires
     the dispatcher to translate `columns_of_for_table_expr` returning `Err(())` into exactly one
     `ColumnsOfUnresolvableSchema` diagnostic with no cascading HOF-body diagnostics.

  Phase 5's real-fixture coverage (`examples/meta_columns/`) will exercise the end-to-end path
  once a HOF body-check dispatcher exists and naturally surface the wiring gap. Phase 5 may
  therefore land the dispatcher as part of making the fixture pass, OR a dedicated Phase B/C
  follow-up plan may be needed. Decision deferred until Phase 5 surfaces the concrete shape
  needed by the fixture.

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes.
- `cargo test -p smelt-cli --test example_diagnostics` passes — `examples/meta_columns/` clean, broken sub-fixtures report the exact Phase C diagnostic codes.
- `/smelt:validate meta_language` reports zero drift.
- LSP smoke test in `examples/meta_columns/`: hover, goto-def, completion all work for `smelt.columns_of`, ColumnRef bindings, field projections, and lifted identifiers per spec.
- Phase 7 acceptance gate met: every applicable expert reviewer (type-expert, lsp-expert, examples-curator, docs-reviewer, cross-feature-impact-reviewer) reported "no material findings" on its final dispatch, recorded in "Deferred during implementation" with round counts per expert. No stop-the-line condition fired.
