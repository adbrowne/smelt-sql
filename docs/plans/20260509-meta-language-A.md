# Plan: Meta-Language Phase A — `List<T>`, list literals, spread

**Date**: 2026-05-09
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md) §"Phase A — `List<T>`, list literals, spread"; cross-touched in [`docs/specs/types.md`](../specs/types.md) §"smelt.define type annotations" and [`docs/specs/gradual_typing.md`](../specs/gradual_typing.md) §"`List<Unknown>` widening (Phase A — meta-language)"
**Spec diff**: uncommitted working tree (Phase A surface, semantics, design, constraints; cross-spec touches in `types.md` + `gradual_typing.md`)
**Tracking PR / branch**: PR #117 — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this plan in full. Then read the spec at `docs/specs/meta_language.md` §"Phase A" and the cross-spec touches in `types.md` / `gradual_typing.md` — they are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec` to revise the spec rather than encode the divergence in code.
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
- Real-fixture tests under `examples/meta_lists/` — every phase from Phase 5 onward exercises its feature there; earlier phases have unit tests in `crates/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no HOFs (`map`/`filter`/`reduce`), no lambdas, no pipe operator, no reflection — those are Phase B+.
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference.rs` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic).

---

## Context

The meta-language Phase A spec increment landed in this session's earlier commit. The spec authors `List<T>` as a meta-only fragment sort, list literals `[a, b, c]` (with bidirectional meta-vs-data disambiguation against runtime `Array<U>` literals), and the `...xs` spread operator. This plan drives the implementation, examples, user docs, and skill update for that surface. It is the first of seven implementation phases (A–G) in the meta-language sequence; it must land cleanly because every later phase plugs into the type-checker hooks Phase A introduces.

## Scope

### In scope (spec coverage)

- `meta_language.md` §"Phase A — Surface" — `List<T>` sort entry, `[a, b, c]` literal, `...xs` spread (positions and forbidden-position list).
- `meta_language.md` §"Per-phase semantic rules" Phase A — eleven normative rules covering list type formation, literal evaluation, bidirectional disambiguation, empty/heterogeneous handling, covariance, spread evaluation, empty-spread elision, position validation, non-list spread, compile-time-only invariant, termination.
- `meta_language.md` §"Per-phase design rationale" Phase A — captured in spec; no plan action.
- `meta_language.md` §"Phase A invariants" — preserved as architectural invariants policed by the implementation.
- Four new diagnostic codes (`MetaListEmptyTypeUnknown`, `MetaListHeterogeneous`, `MetaSpreadInForbiddenPosition`, `MetaSpreadOnNonList`).
- `types.md` `List<T>` sort vocabulary entry — already in spec; runtime witness is `SmeltType::List(Box<SmeltType>)`.
- `gradual_typing.md` `List<Unknown>` widening — already in spec; the implementation must emit at the source of widening, not at downstream consumers.
- LSP hover for list literal and spread.
- Examples fixture `examples/meta_lists/` covering happy path + at least one diagnostic edge case, gated by `crates/smelt-cli/tests/example_diagnostics.rs`.
- User docs at `docs-site/docs/meta-language/{index,lists,reference}.md`.
- `smelt-app-builder` skill: per-phase reference doc.

### Explicitly deferred

- `map` / `filter` / `reduce` HOFs and lambdas (`fn x => body`) — Phase B.
- Pipe operator `|>` — Phase B.
- Contextual reducers (`comma_sep`, `union_all`, `and_all`, …) — Phase B.
- `smelt.config.var(...)` — Phase B.
- Reflection (`smelt.columns_of`, `smelt.models.*`) — Phases C–D.
- Records, `Map<K,V>`, config loaders — Phase E1.
- Multi-model production (`generates: models`, `ModelDef`) — Phase E2.
- LSP completion / goto-definition into lambda bodies / rename for new constructs — Phase G (per-phase fragments land as later phases ship; Phase A only owns hover).
- The bridge rule between `List<Expr<T>>` and `SelectItems<Scalar>` — Phase B.
- The `Array<U>(…)` runtime-array constructor — Phase E2 (until then, the only Data-World runtime array path remains the existing `Expr<Array<U>>` literal route).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 3d965fb | 2026-05-09 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |

---

### Phase 1: Parser surface — `[…]` literal + `...xs` spread CST nodes

**Goal.** Lex and parse list literals and the triple-dot spread operator. Existing `LBRACKET` / `RBRACKET` tokens and the existing `ARRAY_LITERAL` CST node are reused; the new work is (a) introducing a `DOT_DOT_DOT` token distinct from the existing `..` (two-dot) struct-spread token and (b) introducing a `LIST_SPREAD` CST production that carries `DOT_DOT_DOT` + an expression. The spec rule "the parser produces a single `LIST_LITERAL` CST node" is satisfied by reusing `ARRAY_LITERAL` (renaming or aliasing is implementer's choice; either way, one CST kind covers both meta and Data-World readings).

**Pre-conditions.** None — Phase 1 is the entry phase.

**TDD tests to write first.** Listed verbatim — write these as failing tests before any implementation:

- `crates/smelt-parser/src/lexer.rs::tests::tokenize_triple_dot` — assert that `...` lexes as a single `DOT_DOT_DOT` token, distinct from the existing `..` token sequence.
- `crates/smelt-parser/src/lexer.rs::tests::triple_dot_disambiguates_from_double_dot` — assert that `..foo` lexes as `DOT_DOT IDENT(foo)` (existing struct-spread) and `...foo` lexes as `DOT_DOT_DOT IDENT(foo)`.
- `crates/smelt-parser/src/parser.rs::tests::parse_list_literal_homogeneous` — `[1, 2, 3]` parses to one CST list-literal node with three child expressions and no errors.
- `crates/smelt-parser/src/parser.rs::tests::parse_list_literal_trailing_comma` — `[1, 2, 3,]` parses identically (no separator-related diagnostics).
- `crates/smelt-parser/src/parser.rs::tests::parse_list_literal_singleton` — `[x]` parses to one CST node with one child.
- `crates/smelt-parser/src/parser.rs::tests::parse_list_literal_empty` — `[]` parses to one CST node with zero children, no errors.
- `crates/smelt-parser/src/parser.rs::tests::parse_list_literal_nested` — `[[1, 2], [3, 4]]` parses to a nested-list shape.
- `crates/smelt-parser/src/parser.rs::tests::parse_spread_in_select_list` — `SELECT id, ...metric_exprs, created_at FROM users` produces a SELECT with a `LIST_SPREAD` child between two column references.
- `crates/smelt-parser/src/parser.rs::tests::parse_spread_in_function_args` — `coalesce(...numerics, 0)` produces a function call with one `LIST_SPREAD` argument and one literal argument.
- `crates/smelt-parser/src/parser.rs::tests::parse_spread_of_list_literal` — `SELECT id, ...[a, b, c] FROM t` parses with a `LIST_SPREAD` node whose child is a list-literal node.
- `crates/smelt-parser/src/parser.rs::tests::parse_list_literal_error_recovery_unterminated` — `SELECT [a, b FROM t` recovers (parser does not crash; produces a partial list-literal node and continues parsing).

**Implementation shape.**

- `crates/smelt-parser/src/syntax_kind.rs`: add `DOT_DOT_DOT` token; add `LIST_SPREAD` syntax kind. (`LBRACKET`, `RBRACKET`, and the existing `ARRAY_LITERAL` are reused; whether to alias `ARRAY_LITERAL` as `LIST_LITERAL` is implementer's choice — the spec rule constrains the count of CST kinds for `[…]`, not the name.)
- `crates/smelt-parser/src/lexer.rs`: extend the `.` / `..` lookahead to recognise three-dot as `DOT_DOT_DOT`. Confirm no conflict with SQL dialect tokens (`...` is currently unused per `DESIGN.md`; the meta-plan §"New parser tokens" pins this).
- `crates/smelt-parser/src/parser.rs`: extend the existing `parse_array_literal` (or equivalent) to accept trailing commas and empty literals if it does not already; add a `parse_list_spread` recogniser called from any comma-separated grammar position (SELECT list, GROUP BY, ORDER BY, function args, IN-list, VALUES rows, list-literal element).
- `crates/smelt-parser/src/ast.rs`: typed wrappers `ListLiteral` (or extend the existing `ArrayLiteral`) and `ListSpread`.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-parser/src/syntax_kind.rs`
- `crates/smelt-parser/src/lexer.rs`
- `crates/smelt-parser/src/parser.rs`
- `crates/smelt-parser/src/ast.rs`

**Docs touched.**

- None (parser surface is internal; the user-visible surface lands when type-checking + diagnostics fire in Phase 3).

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified, with red→green observed.
- [ ] `DOT_DOT_DOT` does not regress any existing `..` (struct-spread) test.
- [ ] Parser recovery: an unterminated list literal does not crash and does not corrupt downstream parsing of the rest of the file.
- [ ] No analysis logic or Salsa imports added to `smelt-parser` (it remains the standalone parser per `CLAUDE.md` "Parser Architecture").
- [ ] Spread CST node accepted in every comma-separated position the spec enumerates as "valid"; not accepted in WHERE / FROM / boolean / named-arg positions (parser produces an error node or rejects).
- [ ] `cargo fmt --all -- --check` and `cargo clippy --all-targets` pass.

**Commit.** `feat(parser): list-literal + spread CST + DOT_DOT_DOT token (meta-language Phase A)`

---

### Phase 2: Type system — `SmeltType::List<T>` + LUB + covariance (pure)

**Goal.** Add the `List<T>` runtime witness and the pure inference rules for list literals (LUB-based element typing, empty-literal target inference, covariance on subtype check). Diagnostic emission is wired in Phase 3; Phase 2 produces `Unknown` placeholders where Phase 3 will emit codes.

**Pre-conditions.** Phase 1 done — parser produces list-literal CST nodes.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-types/src/signatures.rs::tests::list_type_round_trip` — `SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))))` parses-from / formats-to `List<Expr<Integer>>` correctly.
- `crates/smelt-types/src/signatures.rs::tests::list_type_nested` — `List<List<Expr<Text>>>` round-trips.
- `crates/smelt-types/src/signatures.rs::tests::list_subtype_covariant` — `is_subtype_of(List<Expr<Integer>>, List<Expr<Numeric>>) == true`; `is_subtype_of(List<Expr<Numeric>>, List<Expr<Integer>>) == false`.
- `crates/smelt-types/src/signatures.rs::tests::list_subtype_invariant_when_element_unrelated` — `is_subtype_of(List<TableExpr>, List<Expr<Numeric>>) == false`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_list_literal_homogeneous_integer` — `[1, 2, 3]` infers to `List<Expr<Integer>>` (element LUB).
- `crates/smelt-db/src/type_inference.rs::tests::infer_list_literal_lub_promotion` — `[1, 1.5]` infers to `List<Expr<Double>>` (Numeric LUB per `types.md` §"Numeric promotion chain").
- `crates/smelt-db/src/type_inference.rs::tests::infer_list_literal_heterogeneous_unknown` — `[1, 'hello']` infers to `List<Unknown>` (no diagnostic emission yet — that's Phase 3; the inference function returns the Unknown-element list and a recorded "would emit MetaListHeterogeneous" sentinel that Phase 3 wires).
- `crates/smelt-db/src/type_inference.rs::tests::infer_list_literal_empty_with_target` — `[]` in a position with expected `List<Expr<Integer>>` infers to `List<Expr<Integer>>`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_list_literal_empty_without_target` — `[]` in an unconstrained position returns `List<Unknown>` + sentinel for `MetaListEmptyTypeUnknown`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_list_literal_nested` — `[[1, 2], [3, 4]]` infers to `List<List<Expr<Integer>>>`.

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs`:
  - Add `SmeltType::List(Box<SmeltType>)` variant. Update `SmeltTypeParseError::UnsupportedSort` paths so `List<T>` parses; ensure existing exhaustive matches over `SmeltType` are extended (compiler-checked).
  - Extend `is_subtype_of` (or whatever the existing subtype primitive is) with the covariant `List<S> <: List<T>` rule.
  - Update Display / formatter for `List<T>` rendering.
- `crates/smelt-db/src/type_inference.rs`:
  - Add `pub fn infer_list_literal(elements: &[Expr], ctx: &TypeContext, expected: Option<&SmeltType>) -> ListLiteralInferResult` where the result carries the inferred `SmeltType` and any pending diagnostic sentinels. Pure function; no Salsa imports.
  - The LUB walks elements left-to-right; the empty-literal branch consults `expected`.
  - Reuse `numeric_lub` and existing fragment-sort subtyping helpers.
- No diagnostic codes are emitted in this phase; the sentinel pattern lets Phase 3 add the wiring without restructuring.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-types/src/signatures.rs`
- `crates/smelt-db/src/type_inference.rs`

**Docs touched.**

- None (the surface is already pinned by the spec; no implementation note belongs in `docs/specs/`).

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified.
- [ ] `is_subtype_of` is covariant in `List<T>` and rejects unrelated element types.
- [ ] `type_inference.rs` and `signatures.rs` remain pure — `grep -n 'use.*salsa' crates/smelt-types/src/signatures.rs crates/smelt-db/src/type_inference.rs` returns nothing under analysis logic (the existing acceptable exceptions per `CLAUDE.md` are not extended).
- [ ] Element-LUB respects the strict-by-default doctrine — heterogeneous element families produce `List<Unknown>`, never a silent widening.
- [ ] No diagnostic codes wired yet — this phase records sentinels; Phase 3 emits.
- [ ] Display / formatter renders `List<T>` per `types.md` §"smelt.define type annotations".

**Commit.** `feat(types): SmeltType::List + LUB + covariance (meta-language Phase A)`

---

### Phase 3: Diagnostics + bidirectional disambiguation + spread expansion

**Goal.** Wire the four Phase A diagnostic codes; implement the bidirectional rule that disambiguates `[…]` between meta-list and Data-World runtime array based on the splice point's expected sort; implement spread expansion (forbidden-position validation, empty-list elision, non-list error, provenance origin tags) into the existing splice-point machinery.

**Pre-conditions.** Phases 1–2 done — parser and type system both know about the new shapes.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_meta_list_empty_type_unknown` — `MetaListEmptyTypeUnknown` exists in the `DiagnosticCode` enum and renders the spec message format.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_meta_list_heterogeneous` — same, `MetaListHeterogeneous`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_meta_spread_in_forbidden_position` — same, `MetaSpreadInForbiddenPosition`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_meta_spread_on_non_list` — same, `MetaSpreadOnNonList`.
- `crates/smelt-db/src/type_inference.rs::tests::list_literal_disambiguation_meta_list_target` — `[1, 2, 3]` at a splice point expecting `List<Expr<Integer>>` evaluates as meta-list (not Data-World array).
- `crates/smelt-db/src/type_inference.rs::tests::list_literal_disambiguation_data_array_target` — `[1, 2, 3]` at a splice point expecting `Expr<Array<Integer>>` evaluates as runtime array.
- `crates/smelt-db/src/type_inference.rs::tests::list_literal_disambiguation_both_admissible_meta_wins` — at a position admitting both meta-list and Data-World array, the literal evaluates as meta-list.
- `crates/smelt-db/src/type_inference.rs::tests::list_literal_heterogeneous_emits_diagnostic` — `[1, 'hello']` emits exactly one `MetaListHeterogeneous` anchored at the literal's source span.
- `crates/smelt-db/src/type_inference.rs::tests::list_literal_empty_unknown_target_emits_diagnostic` — `[]` in an unconstrained position emits exactly one `MetaListEmptyTypeUnknown` anchored at the literal's source span.
- `crates/smelt-db/src/type_inference.rs::tests::spread_in_select_list_expands` — `SELECT id, ...[a, b], created_at` produces a SELECT type-checked as if `SELECT id, a, b, created_at` were written; each spread-emitted item carries `Synthesized(SpreadFrom(span_of_list_literal))` provenance.
- `crates/smelt-db/src/type_inference.rs::tests::spread_empty_list_elides` — `SELECT id, ...[], created_at` type-checks identically to `SELECT id, created_at`; no diagnostic emitted.
- `crates/smelt-db/src/type_inference.rs::tests::spread_in_where_clause_emits_diagnostic` — `WHERE x = 1 AND ...preds` emits `MetaSpreadInForbiddenPosition` at the spread span.
- `crates/smelt-db/src/type_inference.rs::tests::spread_on_non_list_emits_diagnostic` — `SELECT ...x FROM t` where `x` is `Expr<Integer>` emits `MetaSpreadOnNonList`; surrounding SELECT type-checks as if the spread were absent.

**Implementation shape.**

- `crates/smelt-db/src/lib.rs`: add the four diagnostic codes to `DiagnosticCode`. Render messages per `meta_language.md` §"Diagnostic codes" table.
- `crates/smelt-db/src/type_inference.rs`:
  - Convert the Phase 2 sentinel returns into actual diagnostic emissions.
  - Bidirectional rule: at every splice point that calls `infer_list_literal`, pass the expected sort (from the surrounding `TypeContext`); the inference function chooses meta-list vs Data-World array based on the rule in `meta_language.md` §"Per-phase semantic rules" Phase A rule 3. When both readings are admissible at a position, return the meta-list interpretation.
  - Spread expansion: a new `pub fn expand_spread_into_position(spread: &SpreadAst, ctx: &TypeContext, position: SpliceContext) -> ExpandedSplice` pure function. It (a) checks the position kind against the forbidden-position list and emits `MetaSpreadInForbiddenPosition` if invalid; (b) verifies the operand is `List<T>` and emits `MetaSpreadOnNonList` otherwise; (c) returns the per-element list with `Synthesized(SpreadFrom(span))` origin tags.
  - Empty-spread elision is a property of the expansion result (zero elements); the calling site (SELECT list type-check, function-arg type-check, etc.) must handle "spread contributed zero items, adjacent commas elide" by simply iterating over the empty slice.
- Wire `expand_spread_into_position` into:
  - SELECT-list type-check in `type_inference.rs` (or wherever `infer_select_column_types` lives).
  - Function-call argument type-check (positional argument list).
  - GROUP BY, ORDER BY, IN-list, VALUES-row type-check.
- Forbidden-position list per spec — Phase 3 enforces it; positions not in the allow-list emit `MetaSpreadInForbiddenPosition`.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/lib.rs` (DiagnosticCode + render)
- `crates/smelt-db/src/type_inference.rs` (rule wiring, spread expansion, sentinels → diagnostics)
- `crates/smelt-db/src/function_body_check.rs` (if argument-list type-check lives there)

**Docs touched.**

- None — Surface section in `meta_language.md` already enumerates the codes; this phase makes the implementation match.

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified.
- [ ] All four Phase A diagnostic codes anchor at the offending span and render the message format from `meta_language.md`.
- [ ] Bidirectional disambiguation honours rule 3 from `meta_language.md` §"Per-phase semantic rules" Phase A — meta wins when both admissible.
- [ ] Spread expansion stamps `Synthesized(SpreadFrom(span))` on every emitted item per `expansion.md` §"Provenance origin tags".
- [ ] Empty-list spread elides — no diagnostic, no synthetic empty item, surrounding position type-checks unchanged.
- [ ] Forbidden-position list matches spec exactly (WHERE, FROM-without-reducer, boolean, named-arg).
- [ ] `MetaSpreadOnNonList` drops the spread and continues type-checking the surrounding form (single diagnostic, no avalanche).
- [ ] `type_inference.rs` purity preserved.

**Commit.** `feat(types): meta-list diagnostics + bidirectional dispatch + spread expansion (meta-language Phase A)`

---

### Phase 4: LSP hover for list literal and spread

**Goal.** Implement hover for the two new CST nodes per `meta_language.md` §"LSP support required by Phase A". Goto-definition through list elements and rename are deferred to Phase G; this phase ships hover only.

**Pre-conditions.** Phases 1–3 done — parser, type-checker, diagnostics all in place.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-lsp/src/lib.rs::tests::hover_list_literal_homogeneous` — hover on `[1, 2, 3]` returns text containing `List<Expr<Integer>>`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_list_literal_empty_with_target` — hover on `[]` at a position expecting `List<Expr<Text>>` returns `List<Expr<Text>>`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_list_literal_unknown` — hover on `[1, 'hello']` returns `List<Unknown>`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_list_literal_dual_admissible` — hover on `[1, 2, 3]` at a position admitting both meta-list and Data-World array surfaces both readings (e.g. `List<Expr<Integer>>` (meta) — also valid as `Array<Integer>`); the user-facing text matches the spec note "literal accepted in two contexts".
- `crates/smelt-lsp/src/lib.rs::tests::hover_spread_returns_source_list_type` — hover on `...xs` where `xs: List<Expr<Numeric>>` returns text containing `List<Expr<Numeric>>`.

**Implementation shape.**

- `crates/smelt-lsp/src/lib.rs`: extend the existing hover handler to dispatch on `LIST_LITERAL` and `LIST_SPREAD` syntax kinds; render the inferred `SmeltType` via the existing Display / formatter.
- The dual-admissible hover text format is implementer's choice; the spec only requires that both readings are surfaced. Suggested format: `List<Expr<Integer>>` followed by a parenthetical "or `Array<Integer>` in array context".

**Critical files (allowed to touch in this phase).**

- `crates/smelt-lsp/src/lib.rs`

**Docs touched.**

- None.

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified.
- [ ] Hover never panics on a partially-parsed list literal (e.g. `[a,` mid-edit) — returns `List<Unknown>` or no hover, not a crash.
- [ ] Hover for spread reads through to the operand's type, not the spread node's own node-type.

**Commit.** `feat(lsp): hover for list literal + spread (meta-language Phase A)`

---

### Phase 5: Examples fixture + smelt-app-builder skill + /smelt-loop medium tier

**Goal.** Land `examples/meta_lists/`, a per-phase reference doc in the `smelt-app-builder` skill, and extend the `/smelt-loop` `medium` tier with at least one Phase A-specific ask (e.g. "use `[a, b, c]` for a dimension list" / "spread a column list into a SELECT"). The fixture is the integration test for the Phase A surface; the skill update gives downstream agents the workflow knowledge; the loop tier extension is the auto-validation harness per meta-plan §4 obligation #6.

**Pre-conditions.** Phases 1–4 done — every Phase A code path can run end-to-end.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_lists_clean_workspace` — running diagnostics over `examples/meta_lists/` produces zero errors and warnings.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_lists_broken_heterogeneous` — running diagnostics over the broken sub-fixture (or `examples/broken/meta_list_*`) produces exactly one `MetaListHeterogeneous` anchored at the literal.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_lists_broken_empty_unknown` — same shape for `MetaListEmptyTypeUnknown`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_lists_broken_spread_forbidden` — same shape for `MetaSpreadInForbiddenPosition`.

**Implementation shape.**

- `examples/meta_lists/`:
  - `smelt.yml` — minimal workspace config.
  - `models/select_with_spread.sql` — happy path: `SELECT id, ...[name, email] FROM users` (or equivalent over a synthesised source).
  - `models/coalesce_spread.sql` — happy path: spread into a variadic built-in (`coalesce(...[revenue, tax, shipping], 0)`).
  - `models/empty_spread.sql` — happy path: `SELECT id, ...[], created_at FROM users` produces a clean SELECT.
  - `models/nested_list.sql` — happy path: `SELECT array_construct(...[1, 2, 3])` or similar; exercises a single `List<Expr<Integer>>` value.
  - `sources.yml` — declares the source tables (`users`, `orders`).
- Negative cases — either as `examples/meta_lists/broken/*.sql` (preferred, keeps the fixture self-contained) or under `examples/broken/`. Each broken model triggers exactly one of the four Phase A codes.
- `.claude/skills/smelt-app-builder/references/20260509-meta-lists.md` — short workflow reference: "When to use a list literal vs SelectItems; what `...xs` does; where it's forbidden; how diagnostics look" — point at the user docs for syntax detail, capture only the workflow gotchas.
- `/smelt-loop` `medium` tier extension — the loop's existing fixture catalogue gets at least one new ask under the `medium` tier that requires Phase A surface to solve (e.g. "rewrite this hard-coded `SELECT a, b, c FROM t` into a list-literal + spread driven by a `dim_list`"). The ask must be small enough that an agent with skill access + this spec can solve it in one iteration; failure modes the agent might hit must be diagnostics the implementation now emits (not silent confusion).

**Critical files (allowed to touch in this phase).**

- `examples/meta_lists/**` (new tree)
- `examples/broken/meta_list_*.sql` if negative cases are placed there
- `crates/smelt-cli/tests/example_diagnostics.rs`
- `.claude/skills/smelt-app-builder/references/20260509-meta-lists.md`
- `.claude/commands/smelt-loop.md` (or wherever the medium-tier fixture catalogue lives) — add at least one Phase A ask

**Docs touched.**

- The skill reference is a docs touch by `meta_language.md` §References → User docs aspirational entries; the skill reference complements `docs-site/`.

**Review checklist** (material findings only):

- [ ] Examples build cleanly in `cargo test -p smelt-cli --test example_diagnostics`.
- [ ] Each broken sub-fixture triggers exactly the one diagnostic code it targets — no double-reporting, no incidental other-phase diagnostics.
- [ ] No use of HOFs / lambdas / pipe / reducers — Phase A surface only.
- [ ] Skill reference is short, points at user docs, captures only workflow advice (not syntax).
- [ ] `/smelt-loop` medium tier has at least one new Phase A ask; ask is solvable with the shipped surface; failure modes correspond to shipped diagnostics.

**Commit.** `feat(examples): meta_lists fixture + skill reference + smelt-loop medium tier (meta-language Phase A)`

---

### Phase 6: User docs

**Goal.** Ship `docs-site/docs/meta-language/{index,lists,reference}.md` per the spec's References → User docs section. Every shipped construct, every diagnostic code, every spec rule that has user-visible consequences is documented.

**Pre-conditions.** Phases 1–5 done — code, fixtures, skill all match the spec.

**TDD tests to write first.** Documentation phases are validated by `/smelt:validate`, not by `cargo test`. The validation gate is:

- `/smelt:validate meta_language` reports zero drift between Surface section and `docs-site/docs/meta-language/{index,lists,reference}.md`.

This phase has no Rust unit tests. The acceptance gate is content review (the reviewer checklist below) + `/smelt:validate` running clean.

**Implementation shape.**

- `docs-site/docs/meta-language/index.md`:
  - Concept overview: meta-world vs data-world, splice points, what the meta-language adds in v1, link to research doc for full design.
  - Phase coverage table (only Phase A has user-visible content; later phases land as they ship).
  - Cross-link: `lists.md`, `reference.md`.
- `docs-site/docs/meta-language/lists.md`:
  - The `List<T>` type, plain-language explanation.
  - List literal syntax, with worked examples drawn from `examples/meta_lists/`.
  - The bidirectional disambiguation rule (`[1, 2, 3]` at a meta-list vs runtime-array position) with an example of each.
  - Spread operator `...xs`, valid + forbidden positions, with an example of each.
  - Empty-list semantics (literal + spread elision).
  - Diagnostic codes — the four Phase A codes, what each means, what to fix.
- `docs-site/docs/meta-language/reference.md`:
  - Alphabetical reference; Phase A entries: `[…]` literal, `...xs` spread, `List<T>` type, four diagnostic codes. Each entry has a short type signature / syntax + a one-line worked example.
  - Reference is updated by every later phase; Phase A populates the initial entries.

**Critical files (allowed to touch in this phase).**

- `docs-site/docs/meta-language/index.md` (new)
- `docs-site/docs/meta-language/lists.md` (new)
- `docs-site/docs/meta-language/reference.md` (new)
- `docs-site/sidebars.ts` (or equivalent navigation file) — add the meta-language section

**Docs touched.**

- All three user-docs files above.

**Review checklist** (material findings only):

- [ ] Every Surface section in `meta_language.md` Phase A appears in user docs.
- [ ] Every diagnostic code is documented with a "what to fix" hint.
- [ ] Every spread forbidden-position is documented.
- [ ] No syntax appears in docs that is not speced.
- [ ] Reference page is alphabetical and complete.
- [ ] `/smelt:validate meta_language` reports zero drift.
- [ ] Worked examples are runnable — they correspond to the `examples/meta_lists/` fixture.

**Commit.** `docs(meta-language): user-docs index + lists + reference (meta-language Phase A)`

---

### Phase 7: Expert reviewer dispatch loop

**Goal.** Run each Phase A applicable expert reviewer from meta-plan §5 over the Phase A diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below. This phase is the realisation of the user's original ask: "Use expert reviews by subagents with specific context to help guide the implementation."

**Pre-conditions.** Phases 1–6 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase A subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **parser-expert** | sonnet | `crates/smelt-parser/src/{lexer,parser,ast,syntax_kind}.rs` | `DOT_DOT_DOT` lexer addition does not regress `..` (struct-spread); list-literal vs array-literal CST shape correct; spread accepted only in spec-listed positions; recursive-descent depth/recovery invariants intact. |
| **type-expert** | sonnet | `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/type_inference.rs` | `SmeltType::List<T>` addition is non-breaking (no missed exhaustive matches); LUB rules for homogeneous / Numeric-promotion / heterogeneous lists match `meta_language.md` Phase A semantics; `List<Unknown>` widening matches `gradual_typing.md`; `type_inference.rs` purity preserved (no Salsa imports inside analysis logic). |
| **lsp-expert** | sonnet | `crates/smelt-lsp/src/lib.rs` and any new LSP code paths from Phase 4 | Hover on a list literal returns `List<T>`; hover on a spread returns the operand list type; spans line up with CST; no regressions in goto-def or completion in adjacent positions. |
| **examples-curator** | haiku | `examples/meta_lists/` (and any broken sub-fixtures introduced for Phase 3 diagnostics) | Fixture is minimal-but-realistic (no contrived shapes); covers the happy path + at least one edge case (empty literal, spread of literal, heterogeneous error); passes `cargo test -p smelt-cli --test example_diagnostics`. |
| **docs-reviewer** | haiku | `docs-site/docs/meta-language/{index,lists,reference}.md` | Every Surface item from `meta_language.md` Phase A is documented; every diagnostic code has a "what to fix" hint; reference page is alphabetical and complete; no syntax appears in docs that is not speced. |

**Loop discipline.**

1. **Round 1.** Dispatch all five experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - The phase plan path and the spec sections that are the oracle (`docs/specs/meta_language.md` Phase A, plus `types.md` / `gradual_typing.md` cross-touches).
   - The exact file scope from the table above.
   - The diff range to review (commits since the start of Phase A — typically `git log --oneline <Phase-A-base>..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into other phases.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-A): address {expert-name} feedback` (e.g. `review(meta-language-A): address parser-expert feedback`).
   - Push after each commit (so the user sees progress on PR #117).

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were addressed, not the whole panel. Provide the same prompt as round 1 plus a diff of what changed since round N−1. If the expert returns "no material findings", that expert is **clean** and exits the loop.

4. **Repeat** step 2 → step 3 until **every** expert is clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of the following fires:
   - Same expert flags a material finding on round 3 (per-expert bound). The third repeat means the fix is wrong or the spec is wrong; the user must arbitrate.
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7). That's a design problem, not an implementation problem.
   - An expert's findings would force a spec change. Run `/smelt:spec meta_language` first; if the spec edit is non-trivial or contentious, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase A. Pause; the autonomy loop should not silently absorb pre-existing breakage.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus `docs/plans/20260509-meta-language-A.md` (to record the round count and final clean status).

**Docs touched.** None new — fixes may amend `docs-site/docs/meta-language/*` if the docs-reviewer flags a surface drift.

**Review checklist** (material findings only — applied to the expert-dispatch *process*, not to a code diff):

- [ ] All five experts were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below (see acceptance gate).
- [ ] No fix touched files outside the dispatching expert's scope (no scope creep).
- [ ] No expert ran more than 3 rounds; if any did, the autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 7 expert review: parser-expert clean (R{n}), type-expert clean (R{n}), lsp-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-A): address {expert-name} feedback`. If round 1 came back clean for an expert, no commit for that expert. The acceptance-gate summary line lands in the next commit naturally (or in a tiny `chore(meta-language-A): record Phase 7 review summary` if no other phase-7 commits were made).

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes.
- `cargo test -p smelt-cli --test example_diagnostics` passes — `examples/meta_lists/` clean, broken sub-fixtures report the exact Phase A diagnostic codes.
- `/smelt:validate meta_language` reports zero drift.
- Hover in the LSP shows `List<T>` on a list literal and the operand list type on a spread (manual or scripted LSP smoke test in `examples/meta_lists/`).
- Phase 7 acceptance gate met: every applicable expert reviewer (parser-expert, type-expert, lsp-expert, examples-curator, docs-reviewer) reported "no material findings" on its final dispatch, recorded in "Deferred during implementation" with round counts per expert. No stop-the-line condition fired.
