# Plan: Meta-Language Phase B — HOFs, lambdas, pipe, contextual reducers, `smelt.config.var`

**Date**: 2026-05-10
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md) §"Phase B — HOFs, lambdas, pipe, contextual reducers, `smelt.config.var`"; cross-touched in [`docs/specs/expansion.md`](../specs/expansion.md) §"Frame-stack invariants" (anonymous-frame form), [`docs/specs/scoping.md`](../specs/scoping.md) §"Resolution order" (lambda parameter scope), [`docs/specs/types.md`](../specs/types.md) §"smelt.define type annotations" (`Lambda<T, U>` entry)
**Spec diff**: commit `d4d4586` (`spec(meta-language): Phase B surface + semantics + design + invariants`) on branch `research/typed-meta-programming`
**Tracking PR / branch**: PR #117 — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this plan in full. Then read the spec at `docs/specs/meta_language.md` §"Phase B" and the cross-spec touches in `expansion.md` / `scoping.md` / `types.md` — they are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec` to revise the spec rather than encode the divergence in code.
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
- Real-fixture tests under `examples/meta_hofs/` — every phase from Phase 5 onward exercises its feature there; earlier phases have unit tests in `crates/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no reflection (`smelt.columns_of`, `smelt.models.*`), no records / `Map<K, V>`, no multi-model production, no parameterised reducers, no multi-arg lambdas — those are Phase C+.
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference.rs` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic).

---

## Context

The meta-language Phase B spec increment landed in commit `d4d4586`. The spec authors lambdas (`fn x => body`, single-arg, HOF-position-only), three higher-order functions (`map`, `filter`, `reduce` — exactly two positional args, zero named args, names reserved), the pipe operator `|>` (first-arg, meta-only, parser-level desugaring), a closed registry of seven contextual reducers (`comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`), and `smelt.config.var('x')` (literal-only argument, `Text` result, against `smelt.yml` `vars:`). This plan drives the implementation, examples, user docs, and skill update for that surface. It is the second of seven implementation phases (A–G); it must land cleanly because every later phase plugs into the HOF + reducer + lambda machinery Phase B introduces (reflection results in Phase C+ are `List<T>` values consumed via `map`/`filter`/`reduce`).

## Scope

### In scope (spec coverage)

- `meta_language.md` §"Phase B — Surface" — lambda syntax, three HOFs, pipe operator, closed reducer registry, `smelt.config.var`.
- `meta_language.md` §"Per-phase semantic rules" Phase B — thirteen normative rules covering lambda formation, parameter scoping, capture, HOF evaluation (length, ordering, dispatch), reducer evaluation (left-fold + identity), pipe desugaring, pipe binding, pipe RHS validation, `smelt.config.var` resolution + YAML scalar coercion, HOF inline-expansion frames, name reservation, termination.
- `meta_language.md` §"Per-phase design rationale" Phase B — captured in spec; no plan action.
- `meta_language.md` §"Phase B invariants" — preserved as architectural invariants policed by the implementation.
- Twelve new Phase B diagnostic codes (`LambdaInForbiddenPosition`, `LambdaArityNotSupported`, `LambdaResultTypeMismatch`, `HofExpectsLambda`, `HofExpectsReducer`, `HofNameShadowed`, `ReducerNameShadowed`, `PipeRhsNotCall`, `PipeInDataPosition`, `ReducerInputTypeMismatch`, `ReducerEmptyNoIdentity`, `ConfigVarNotFound`, `ConfigVarNameNotLiteral`, `ConfigVarNullCoercion`).
- `expansion.md` cross-spec touch — anonymous-frame form (`function = "<hof>"`, `fn_id = None`, optional `element_index`).
- `scoping.md` cross-spec touch — lambda parameters as a new scope kind, resolved before any wider scope inside the lambda body.
- `types.md` cross-spec touch — `SmeltType::Lambda(Box<SmeltType>, Box<SmeltType>)` runtime witness with the "meta-only, not user-writable as a parameter sort" note.
- LSP hover for lambdas, HOF calls, pipe expressions, reducer names, `smelt.config.var` calls.
- LSP goto-def for lambda parameters (binder + body uses) and `smelt.config.var` arguments (resolves to `vars.x:` line in `smelt.yml`).
- LSP completion for the bound lambda parameter inside body, and for the closed reducer registry at the second-argument position of `reduce` (filtered by inferred input type).
- Examples fixture `examples/meta_hofs/` covering happy path + at least one diagnostic edge case for each new Phase B code, gated by `crates/smelt-cli/tests/example_diagnostics.rs`.
- User docs at `docs-site/docs/meta-language/{hofs,lambdas,pipes,reducers,config-vars,reference}.md`.
- `smelt-app-builder` skill: per-phase reference doc.
- `/smelt-loop` `medium` tier: at least one Phase B-specific ask (e.g. "express this CASE chain via `fn` and `reduce(or_any)`").

### Explicitly deferred

- Reflection (`smelt.columns_of`, `smelt.models.*`, `smelt.sources.*`) — Phases C–D.
- Records, `Map<K,V>`, schema-typed config loaders (`smelt.config.load_yaml`, etc.) — Phase E1.
- Expression-valued `smelt.config.var(other_var)` — Phase E1.
- Richer-typed config-var reads (Boolean, Integer) — Phase E1.
- Multi-model production (`generates: models`, `ModelDef`, meta-`Text`-as-identifier lift) — Phase E2.
- Parameterised reducers (`concat_with(sep)`) — Phase F.
- Multi-argument lambdas (`fn (a, b) => body`) — Phase F.
- Meta-world ternary `if cond then a else b` — Phase F.
- `zip_with`, `flat_map`, `take`, `drop`, `length`, `index_of`, `any`, `all`, `find`, `partition` — speced as derivations; shipped only if examples force them (per meta-plan §3 theoretical-completeness ledger).
- LSP rename support for lambda parameters / config-var names — Phase G.
- The bridge rule between `List<Expr<T>>` and `SelectItems<Scalar>` beyond what `comma_sep` reducer provides — covered in this phase by `comma_sep`; broader interop deferred to Phase F if needed.
- Pipe-SQL extension (research §4.6 alternative b — `FROM t |> WHERE p |> SELECT cols`) — separate spec, out of plan.
- Wiring of `expand_spread_into_position` and forbidden-position spread coverage to the remaining splice positions (GROUP BY, ORDER BY, function args, IN-list, VALUES rows) deferred from Phase A — this Phase B Phase 3 picks up that wiring as natural side-effect of HOF + reducer integration into those positions.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 0307d79 | 2026-05-10 |
| 2     | done     | 3e0ea89 | 2026-05-10 |
| 3     | done     | 59a425a | 2026-05-10 |
| 4     | done     | 33821f5 | 2026-05-10 |
| 5     | done     |        | 2026-05-10 |
| 6     | pending  |        |      |
| 7     | pending  |        |      |

---

### Phase 1: Parser surface — `fn` keyword + `|>` token + lambda + pipe CST nodes

**Goal.** Lex and parse the new Phase B surface tokens (`fn` keyword, `|>` pipe arrow) and the new CST productions for lambdas and pipe expressions. The lexer must recognise `||` (existing SQL string concatenation) before `|>` to avoid mis-tokenisation. The parser must commit to lambda meaning at the `fn` keyword (no positional disambiguation): once `fn` is consumed, the next identifier is the lambda parameter and the next `=>` is the lambda arrow. Multi-arg lambdas `fn (a, b) => body` parse to a CST node tagged for Phase 3 to reject with `LambdaArityNotSupported`.

**Pre-conditions.** Phase A complete. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all pass on the Phase A baseline.

**TDD tests to write first.** Listed verbatim — write these as failing tests before any implementation:

- `crates/smelt-parser/src/lexer.rs::tests::tokenize_fn_keyword` — `fn` lexes as `FN_KW` keyword token, distinct from a bare `IDENT`. Verify `fnord` still lexes as a single `IDENT` (no over-eager keyword match).
- `crates/smelt-parser/src/lexer.rs::tests::tokenize_pipe_arrow` — `|>` lexes as a single `PIPE_ARROW` token.
- `crates/smelt-parser/src/lexer.rs::tests::pipe_arrow_disambiguates_from_double_pipe` — `||` lexes as `DOUBLE_PIPE` (existing SQL string concat) and `|>` lexes as `PIPE_ARROW`; `|||>` lexes as `DOUBLE_PIPE PIPE_ARROW`.
- `crates/smelt-parser/src/lexer.rs::tests::pipe_arrow_does_not_collide_with_named_arg` — `=>` (named-arg arrow) lexes as `FAT_ARROW` (or whatever the existing token is); `|>` does not lex as a sequence ending in `>`.
- `crates/smelt-parser/src/parser.rs::tests::parse_lambda_single_arg` — `map(xs, fn c => c)` parses to a function-call CST whose second argument is a `LAMBDA` node with one parameter (`c`) and a body that is an identifier reference.
- `crates/smelt-parser/src/parser.rs::tests::parse_lambda_with_complex_body` — `map(xs, fn c => CAST(c AS Text))` parses to a function call whose second argument is a `LAMBDA` whose body is a CAST expression.
- `crates/smelt-parser/src/parser.rs::tests::parse_lambda_multiarg_produces_cst_node_for_phase3_rejection` — `map(xs, fn (a, b) => a)` parses successfully (no parser error) producing a `LAMBDA` node with multi-arg parameter list; the reject is Phase 3's job (lexer/parser does not gate). The test asserts CST shape; it does not assert any diagnostic.
- `crates/smelt-parser/src/parser.rs::tests::parse_pipe_expression` — `xs |> filter(fn c => c)` parses to a `PIPE_EXPR` with LHS = `xs` and RHS = a function-call expression.
- `crates/smelt-parser/src/parser.rs::tests::parse_pipe_chain_left_associative` — `a |> b(p) |> c(q)` parses as `((a |> b(p)) |> c(q))`.
- `crates/smelt-parser/src/parser.rs::tests::parse_pipe_lowest_precedence` — `1 + 2 |> f()` parses as `(1 + 2) |> f()`, not `1 + (2 |> f())`. (Pipe is the lowest-precedence meta-language operator.)
- `crates/smelt-parser/src/parser.rs::tests::parse_pipe_does_not_cross_statement_boundary` — `a |> b(); c` parses as two statements (`a |> b()` and `c`); `a; |> b()` is a parser error at `|>`.
- `crates/smelt-parser/src/parser.rs::tests::parse_pipe_rhs_non_call_recovers` — `xs |> 3 + 4` parses to a `PIPE_EXPR` whose RHS is the bare expression node (Phase 3 emits `PipeRhsNotCall`); the parser does not crash and the surrounding statement continues to parse.
- `crates/smelt-parser/src/parser.rs::tests::parse_lambda_outside_call_recovers` — `let x = fn c => c` (or whatever the closest non-call surrounding form is) parses with a `LAMBDA` node in a position Phase 3 rejects via `LambdaInForbiddenPosition`; the parser does not crash.
- `crates/smelt-parser/src/parser.rs::tests::parse_named_arg_still_works_after_fn_keyword_addition` — existing named-arg syntax (`f(name => value)`) parses unchanged after the `fn`/`=>` parser interaction is added; pin via reused snapshot or assertion.

**Implementation shape.**

- `crates/smelt-parser/src/syntax_kind.rs`: add `FN_KW`, `PIPE_ARROW` tokens; add `LAMBDA`, `LAMBDA_PARAM_LIST`, `PIPE_EXPR` syntax kinds.
- `crates/smelt-parser/src/lexer.rs`:
  - Add `fn` to the keyword table. Verify that no other keyword starts `fn…`; if a future phase adds `for`, the same single-letter prefix issue won't apply.
  - Lex `|` lookahead: if the next char is `|`, emit `DOUBLE_PIPE`; if the next char is `>`, emit `PIPE_ARROW`; else emit single-`|`. The order must check `||` before `|>` to keep SQL string-concat compatibility.
- `crates/smelt-parser/src/parser.rs`:
  - Pratt-style precedence: pipe is lowest among meta-language operators, left-associative. Add `pipe` as the entry point of meta-language expression parsing wrapping the existing expression parse.
  - At the `fn` keyword, switch to a `parse_lambda` path: consume `fn`, parse a parameter list (identifier or parenthesised list), expect `=>`, parse the body expression. Single-arg lambdas omit parens; multi-arg lambdas require parens. The parser does not gate on arity — Phase 3 emits `LambdaArityNotSupported` for non-singleton parameter lists.
  - Pipe RHS validation: after parsing the RHS, mark the `PIPE_EXPR` node with a flag (or rely on Phase 3 walking the CST) indicating whether the RHS is a call expression. Parser does not emit the diagnostic; that is Phase 3's job.
- `crates/smelt-parser/src/ast.rs`: typed wrappers `Lambda`, `LambdaParamList`, `PipeExpr`. Each wrapper exposes accessors for the parameter list / body / LHS / RHS.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-parser/src/syntax_kind.rs`
- `crates/smelt-parser/src/lexer.rs`
- `crates/smelt-parser/src/parser.rs`
- `crates/smelt-parser/src/ast.rs`

**Docs touched.**

- None (parser surface is internal; the user-visible surface lands when type-checking + diagnostics fire in Phase 3).

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified, with red→green observed.
- [ ] `FN_KW` does not regress any existing identifier test (`fnord`, `function`, `fname` still lex as `IDENT`).
- [ ] `PIPE_ARROW` lexer ordering puts `||` before `|>` so SQL string-concat is preserved; verify with the dedicated disambiguation test.
- [ ] Parser commits to lambda meaning at `fn`; no positional disambiguation. The named-arg path `name => value` still parses unchanged when there is no preceding `fn`.
- [ ] Pipe is left-associative and the lowest-precedence meta-language operator. `1 + 2 |> f()` parses as `(1 + 2) |> f()`.
- [ ] Multi-arg lambdas parse to a `LAMBDA` CST node (Phase 3 will reject with `LambdaArityNotSupported`); parser does not gate.
- [ ] Pipe with a non-call RHS parses recovered (Phase 3 emits `PipeRhsNotCall`); parser does not crash.
- [ ] No analysis logic or Salsa imports added to `smelt-parser` (it remains the standalone parser per `CLAUDE.md` "Parser Architecture").
- [ ] `cargo fmt --all -- --check` and `cargo clippy --all-targets` pass.

**Commit.** `feat(parser): fn keyword + pipe-arrow + lambda/pipe CST (meta-language Phase B)`

---

### Phase 2: Type system — `SmeltType::Lambda<T, U>` + HOF inference + reducer registry (pure)

**Goal.** Add the `Lambda<T, U>` runtime witness and the pure inference rules for HOF dispatch (bidirectional binding of lambda parameter type from the HOF's `T`, body synthesis under that binding, result-type construction), the closed reducer registry (input-type validation, output-sort derivation, empty-list identity rule), and the pipe desugaring (parser-level / inference-level — pipe expression and the equivalent un-piped call have identical synthesised types). Diagnostic emission is wired in Phase 3; Phase 2 produces sentinel-tagged results where Phase 3 will emit codes. The cross-spec touch in `types.md` adds the `Lambda<T, U>` enumeration entry.

**Pre-conditions.** Phase 1 done — parser produces lambda + pipe CST nodes.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-types/src/signatures.rs::tests::lambda_type_round_trip` — `SmeltType::Lambda(Box<Expr<Integer>>, Box<Expr<Text>>)` parses-from / formats-to `Lambda<Expr<Integer>, Expr<Text>>` correctly.
- `crates/smelt-types/src/signatures.rs::tests::lambda_type_invariant` — `is_subtype_of(Lambda<Expr<Integer>, Expr<Text>>, Lambda<Expr<Numeric>, Expr<Text>>) == false`. Lambda is invariant (no contravariance, no covariance).
- `crates/smelt-types/src/signatures.rs::tests::lambda_type_equality_only_when_exact` — `is_subtype_of(L, L) == true` only for byte-equal `L`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_map_returns_list_of_body_type` — `map([1, 2, 3], fn c => c)` infers to `List<Expr<Integer>>` (HOF produces `List<U>` where `U` is the lambda body's synthesised type).
- `crates/smelt-db/src/type_inference.rs::tests::infer_map_with_typed_body` — `map([1, 2, 3], fn c => CAST(c AS Text))` infers to `List<Expr<Text>>`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_filter_returns_same_list_type` — `filter([1, 2, 3], fn c => c > 0)` infers to `List<Expr<Integer>>`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_filter_predicate_must_be_boolean_sentinel` — `filter([1, 2, 3], fn c => c)` (predicate body synthesises `Expr<Integer>` not `Expr<Boolean>`) returns a sentinel for `LambdaResultTypeMismatch`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_reduce_returns_reducer_output_sort` — `reduce([1, 2, 3], plus_chain)` infers to `Expr<Integer>`. `reduce(['a', 'b', 'c'], concat)` infers to `Expr<Text>`. `reduce([true, false], and_all)` infers to `Expr<Boolean>`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_reduce_comma_sep_yields_select_items` — `reduce([col1, col2, col3], comma_sep)` infers to `SelectItems<Scalar>` (output is select-list shape regardless of element `T`).
- `crates/smelt-db/src/type_inference.rs::tests::infer_reduce_empty_list_with_identity` — `reduce([], and_all)` with target `Expr<Boolean>` infers to `Expr<Boolean>` (TRUE identity); no sentinel.
- `crates/smelt-db/src/type_inference.rs::tests::infer_reduce_empty_list_no_identity_sentinel` — `reduce([], union_all)` returns a sentinel for `ReducerEmptyNoIdentity`.
- `crates/smelt-db/src/type_inference.rs::tests::infer_reduce_input_type_mismatch_sentinel` — `reduce([1, 2, 3], and_all)` returns a sentinel for `ReducerInputTypeMismatch` (input element type `Expr<Integer>` does not match `and_all`'s declared `Expr<Boolean>`).
- `crates/smelt-db/src/type_inference.rs::tests::infer_pipe_desugars_to_call` — `xs |> filter(fn c => c > 0)` and `filter(xs, fn c => c > 0)` infer to the same `SmeltType` for the same inputs.
- `crates/smelt-db/src/type_inference.rs::tests::infer_pipe_chain_associates_left` — `[1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)` infers to `List<Expr<Integer>>` matching the un-piped `map(filter([1,2,3], fn c => c > 0), fn c => c * 2)`.
- `crates/smelt-db/src/type_inference.rs::tests::lambda_parameter_binding_via_typecontext` — when checking the body of `map(xs: List<Expr<Integer>>, fn c => c)`, the lookup of `c` in the body returns `Expr<Integer>` (lambda parameter binding pushed onto `TypeContext`).
- `crates/smelt-db/src/type_inference.rs::tests::lambda_parameter_shadows_outer_binding` — when an enclosing `smelt.define` parameter named `c` is in scope, the lambda parameter `c` wins inside the lambda body (shadowing).
- `crates/smelt-db/src/type_inference.rs::tests::reducer_registry_lookup_closed_set` — every entry of the closed registry (`comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`) is recognised; an unknown identifier (e.g. `not_a_reducer`) is not in the registry.

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs`:
  - Add `SmeltType::Lambda(Box<SmeltType>, Box<SmeltType>)` variant. Update `SmeltTypeParseError::UnsupportedSort` paths; ensure existing exhaustive matches over `SmeltType` are extended (compiler-checked).
  - Lambda is invariant — `is_subtype_of(Lambda<S1, T1>, Lambda<S2, T2>)` is `true` only when `S1 = S2` and `T1 = T2`.
  - Update Display / formatter for `Lambda<T, U>` rendering.
- `crates/smelt-db/src/type_inference.rs`:
  - Add `pub fn infer_hof_call(hof: HofKind, xs: &Expr, lambda: &Lambda, ctx: &TypeContext) -> HofInferResult` — pure function dispatching on the HOF (`Map`, `Filter`, `Reduce`). For `Map`/`Filter`: bind the lambda parameter to the HOF's `T` (extracted from `xs`'s inferred `List<T>`), synthesise the body type `U`, return the appropriate result type. For `Reduce`: see below.
  - Add `pub fn infer_reduce_call(xs: &Expr, reducer_name: &str, ctx: &TypeContext, expected: Option<&SmeltType>) -> ReduceInferResult` — pure function looking up the reducer in the closed registry, validating the input element type, deriving the output sort. Empty-list path consults the reducer's identity rule (`expected` may be needed for the `plus_chain` LUB-promoted identity case).
  - Add `pub static REDUCER_REGISTRY: &[ReducerSpec]` (or equivalent compile-time constant) — seven entries each with `name`, `input_element_type` (e.g. `Expr<Numeric>` template), `output_sort`, `empty_identity` (Some / None / parameterised-by-element-type).
  - Lambda parameter binding pushes onto `TypeContext::lambda_params: Vec<(String, SmeltType)>` (or extends `function_params` semantics — implementer's choice; the `scoping.md` spec touch will commit to which). The lookup in `lookup_identifier` consults lambda parameters before any wider scope.
  - Pipe desugaring: at AST level, an inference function `infer_pipe_expr(pipe: &PipeExpr, ctx: &TypeContext, expected: Option<&SmeltType>)` constructs a virtual call AST `CALL(LHS, args...)` and infers it. The expression's diagnostic anchoring uses original spans; only the inference output is shared.
- No diagnostic codes are emitted in this phase; the sentinel pattern lets Phase 3 add the wiring without restructuring.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-types/src/signatures.rs`
- `crates/smelt-db/src/type_inference.rs`

**Docs touched.**

- `docs/specs/types.md` — extend §"smelt.define type annotations" with the `Lambda<T, U>` entry per Phase B cross-spec touch obligation. The entry must say: "meta-only; not user-writable as a `smelt.define` parameter sort or return type; constructed only at HOF positional argument positions." Single concise paragraph following the `List<T>` entry.

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified.
- [ ] `Lambda<T, U>` is invariant — `is_subtype_of` rejects every non-equal pair.
- [ ] `type_inference.rs` and `signatures.rs` remain pure — `grep -n 'use.*salsa' crates/smelt-types/src/signatures.rs crates/smelt-db/src/type_inference.rs` returns nothing under analysis logic (existing acceptable exceptions per `CLAUDE.md` are not extended).
- [ ] HOF dispatch binds the lambda parameter type from `xs`'s element type and synthesises the body under that binding (bidirectional checking honoured).
- [ ] Reducer registry is closed (seven entries); registry lookup is by-name; output sort and input-type validation are in the registry, not scattered.
- [ ] `comma_sep` collapses any `List<Expr<T>>` to `SelectItems<Scalar>` per spec.
- [ ] Empty-list reducer evaluation uses the reducer's identity when present; returns a sentinel for `ReducerEmptyNoIdentity` otherwise.
- [ ] Pipe desugars at AST level — pipe expression and the equivalent un-piped call infer to byte-equal `SmeltType` results.
- [ ] Lambda parameter binding shadows outer scope correctly; lookup order is lambda → enclosing function params → wider SQL scope.
- [ ] No diagnostic codes wired yet — this phase records sentinels; Phase 3 emits.
- [ ] `types.md` cross-spec touch lands with `Lambda<T, U>` entry.
- [ ] Display / formatter renders `Lambda<T, U>` per `types.md` §"smelt.define type annotations".

**Commit.** `feat(types): SmeltType::Lambda + HOF inference + reducer registry (meta-language Phase B)`

---

### Phase 3: Diagnostics + cross-spec touches + `smelt.config.var` resolver + HOF expansion frames

**Goal.** Wire all Phase B diagnostic codes; implement the `smelt.config.var('x')` resolver against `smelt.yml` `vars:` (with YAML scalar coercion to `Text` and the four-code diagnostic surface); land the HOF inline-expansion frame stamping (anonymous frame form per `expansion.md` cross-spec touch); land the `scoping.md` cross-spec touch for lambda parameter scoping.

**Pre-conditions.** Phases 1–2 done — parser and type system both know about the new shapes; sentinel-tagged inference paths exist.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_lambda_in_forbidden_position` — `LambdaInForbiddenPosition` exists in the `DiagnosticCode` enum and renders the spec message format.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_lambda_arity_not_supported` — same, `LambdaArityNotSupported`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_lambda_result_type_mismatch` — same, `LambdaResultTypeMismatch`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_hof_expects_lambda` — same, `HofExpectsLambda`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_hof_expects_reducer` — same, `HofExpectsReducer`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_hof_name_shadowed` — same, `HofNameShadowed`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_reducer_name_shadowed` — same, `ReducerNameShadowed`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_pipe_rhs_not_call` — same, `PipeRhsNotCall`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_pipe_in_data_position` — same, `PipeInDataPosition`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_reducer_input_type_mismatch` — same, `ReducerInputTypeMismatch`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_reducer_empty_no_identity` — same, `ReducerEmptyNoIdentity`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_config_var_not_found` — same, `ConfigVarNotFound`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_config_var_name_not_literal` — same, `ConfigVarNameNotLiteral`.
- `crates/smelt-db/src/lib.rs::tests::diagnostic_code_config_var_null_coercion` — same, `ConfigVarNullCoercion` (Warning severity).
- `crates/smelt-db/src/type_inference.rs::tests::lambda_outside_hof_position_emits_diagnostic` — a `LAMBDA` CST node not in a HOF positional argument position emits `LambdaInForbiddenPosition` at the `fn` keyword span.
- `crates/smelt-db/src/type_inference.rs::tests::multi_arg_lambda_emits_arity_diagnostic` — `map(xs, fn (a, b) => a)` emits `LambdaArityNotSupported` at the parameter list span.
- `crates/smelt-db/src/type_inference.rs::tests::filter_predicate_non_boolean_emits_lambda_result_mismatch` — `filter([1,2,3], fn c => c)` emits `LambdaResultTypeMismatch` at the body expression.
- `crates/smelt-db/src/type_inference.rs::tests::map_with_non_lambda_second_arg_emits_hof_expects_lambda` — `map(xs, 42)` emits `HofExpectsLambda`.
- `crates/smelt-db/src/type_inference.rs::tests::reduce_with_non_reducer_second_arg_emits_hof_expects_reducer` — `reduce(xs, fn c => c)` (lambda where reducer expected) emits `HofExpectsReducer`. `reduce(xs, made_up_name)` emits `HofExpectsReducer`.
- `crates/smelt-db/src/type_inference.rs::tests::smelt_define_named_map_emits_hof_name_shadowed` — `smelt.define map(...)` declaration emits `HofNameShadowed` at the name token.
- `crates/smelt-db/src/type_inference.rs::tests::smelt_define_named_concat_emits_reducer_name_shadowed` — `smelt.define concat(...)` declaration emits `ReducerNameShadowed`.
- `crates/smelt-db/src/type_inference.rs::tests::pipe_rhs_not_call_emits_diagnostic` — `xs |> 3 + 4` emits `PipeRhsNotCall` at the RHS span.
- `crates/smelt-db/src/type_inference.rs::tests::pipe_in_where_clause_emits_diagnostic` — `WHERE x = 1 AND (a |> b())` emits `PipeInDataPosition` at the pipe span.
- `crates/smelt-db/src/type_inference.rs::tests::reduce_input_type_mismatch_emits_diagnostic` — `reduce([1, 2, 3], and_all)` emits `ReducerInputTypeMismatch` at the second-argument span.
- `crates/smelt-db/src/type_inference.rs::tests::reduce_empty_no_identity_emits_diagnostic` — `reduce([], union_all)` emits `ReducerEmptyNoIdentity` and the surrounding splice position re-validates as if the `reduce` call were absent.
- `crates/smelt-db/src/type_inference.rs::tests::config_var_not_found_emits_diagnostic` — `smelt.config.var('not_declared')` over a workspace whose `smelt.yml` `vars:` lacks `not_declared` emits `ConfigVarNotFound` at the call site.
- `crates/smelt-db/src/type_inference.rs::tests::config_var_resolves_string_scalar` — `smelt.config.var('region')` over a workspace with `vars: { region: us-west-2 }` resolves to a `Text` value `'us-west-2'`.
- `crates/smelt-db/src/type_inference.rs::tests::config_var_coerces_yaml_boolean` — `smelt.config.var('flag')` over `vars: { flag: true }` resolves to `'true'`; integer `42` resolves to `'42'`.
- `crates/smelt-db/src/type_inference.rs::tests::config_var_null_emits_warning` — `smelt.config.var('nullable')` over `vars: { nullable: ~ }` resolves to `''` and emits `ConfigVarNullCoercion` (Warning).
- `crates/smelt-db/src/type_inference.rs::tests::config_var_non_literal_arg_emits_diagnostic` — `smelt.config.var(some_var)` (non-literal) emits `ConfigVarNameNotLiteral` at the argument span.
- `crates/smelt-db/src/function_body_check.rs::tests::hof_lambda_body_diagnostic_carries_anonymous_frame` — a type error inside a HOF lambda body carries an `ExpansionFrames` payload whose innermost frame has `function = "map"`, `fn_id = None`, and `call_site_range = span_of(map_call)`. (The optional `element_index` may or may not be populated depending on whether the source list was a literal — implementer's choice for static-vs-runtime element indexing in v1; the spec allows both.)

**Implementation shape.**

- `crates/smelt-db/src/lib.rs`: add fourteen new Phase B diagnostic codes to `DiagnosticCode`. Render messages per `meta_language.md` §"Diagnostic codes (new in Phase B)".
- `crates/smelt-db/src/type_inference.rs`:
  - Convert Phase 2 sentinels into actual diagnostic emissions.
  - Lambda position check: walk the CST upward from each `LAMBDA` node; if the immediate parent is not a HOF positional-argument position, emit `LambdaInForbiddenPosition`.
  - Lambda arity check: a `LAMBDA_PARAM_LIST` with more than one identifier emits `LambdaArityNotSupported`.
  - Lambda body type check: synthesise the body's type under the bound parameter and compare against the HOF's required result shape (`Lambda<T, Boolean>` for `filter`, free `Lambda<T, U>` for `map`); mismatch emits `LambdaResultTypeMismatch`.
  - HOF arg validation: `map`/`filter` second arg must be a `LAMBDA` node — otherwise `HofExpectsLambda`. `reduce` second arg must be a bare identifier from the closed registry — otherwise `HofExpectsReducer`.
  - Name shadowing: at `smelt.define` body-check, intercept a name in `{map, filter, reduce, comma_sep, and_all, or_any, union_all, intersect_all, plus_chain, concat}` and emit `HofNameShadowed` (HOFs) or `ReducerNameShadowed` (reducers) at the name token.
  - Pipe RHS check: walk every `PIPE_EXPR` CST node; if the RHS is not syntactically a call expression, emit `PipeRhsNotCall`.
  - Pipe-in-Data-position check: a `PIPE_EXPR` appearing in a Data-World grammar slot (e.g. inside a `WHERE` predicate) emits `PipeInDataPosition`. The check is the same shape as `MetaSpreadInForbiddenPosition` from Phase A — extend the position-check infrastructure.
- `crates/smelt-db/src/lib.rs` (or a new module `config_vars.rs`):
  - `pub fn resolve_smelt_yml_vars(workspace_root: &Path) -> Result<BTreeMap<String, SerdeYamlValue>, ConfigVarLoadError>` — pure function reading `<workspace_root>/smelt.yml` and extracting the `vars:` block. Salsa-wrap in `lib.rs` as `smelt_yml_vars_query(db, workspace) -> Arc<Vars>` with input invalidation on `smelt.yml` change. **Verify whether `smelt.yml` parsing already supports `vars:`**; if it does, reuse it. If not, add the minimal additive parsing.
  - `pub fn coerce_yaml_scalar_to_text(v: &SerdeYamlValue, name: &str) -> CoerceResult` — pure function; returns the `Text` value plus an optional `ConfigVarNullCoercion` warning sentinel.
  - `smelt.config.var` call type-check: confirm argument is a `STRING_LITERAL` CST node (not an expression) — otherwise `ConfigVarNameNotLiteral`. Look up the name in the resolved `vars:` map; missing → `ConfigVarNotFound`.
- `crates/smelt-db/src/function_body_check.rs`:
  - At each HOF call site, push an anonymous frame onto the body-check's frame stack before walking the lambda body. The frame shape: `function = "<hof name>"`, `fn_id = None`, `decl_path = None`, `decl_range = None`, `call_site_range = span_of(hof_call)`, plus optional `element_index` (None in v1 unless the source is a literal at the call site, in which case the implementer may populate it).
  - Pop the frame on exit. Diagnostics surfaced inside the lambda body inherit the frame stack per `expansion.md` §"Frame-stack invariants".
- Extend `crates/smelt-types/src/signatures.rs::FrameInfo` with the optional `element_index: Option<usize>` field per the `expansion.md` cross-spec touch.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-db/src/lib.rs` (DiagnosticCode + render + Salsa wrappers for config-var resolver)
- `crates/smelt-db/src/type_inference.rs` (diagnostic wiring, position checks, sentinel→diagnostic conversion)
- `crates/smelt-db/src/function_body_check.rs` (anonymous-frame stamping, HOF body walk)
- `crates/smelt-db/src/config_vars.rs` (new module — pure `smelt.yml` `vars:` resolver) **or** extension of an existing config module if one already serves `smelt.yml`
- `crates/smelt-types/src/signatures.rs` (extend `FrameInfo` with `element_index: Option<usize>`)
- `docs/specs/expansion.md` (anonymous-frame form spec touch)
- `docs/specs/scoping.md` (lambda parameter scope spec touch)

**Docs touched.**

- `docs/specs/expansion.md` — extend §"`FrameInfo` shape" and §"Frame-stack invariants" to cover the anonymous-frame form (`fn_id = None`, optional `element_index`). One short paragraph + a row addition to the `FrameInfo` field table.
- `docs/specs/scoping.md` — extend §"Resolution order" with lambda parameters as a new step inserted *before* "Function parameters" (lambda params resolve first inside the lambda body). Single short subsection on lambda parameter scoping. Per the spec rule, lambda parameters shadow outer-function parameters (lexical shadowing).

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified.
- [ ] All fourteen Phase B diagnostic codes anchor at the offending span and render the message format from `meta_language.md`.
- [ ] `LambdaInForbiddenPosition` covers every non-HOF position listed in spec (top-level expression, named-arg value, list element, splice point, `smelt.define` argument).
- [ ] `LambdaArityNotSupported` fires only for non-singleton parameter lists; single-arg lambdas pass.
- [ ] `HofNameShadowed` / `ReducerNameShadowed` fire at `smelt.define` declarations of the reserved names.
- [ ] `PipeRhsNotCall` and `PipeInDataPosition` work as spec'd; non-call RHS evaluates as if pipe were absent.
- [ ] `smelt.config.var` resolver reads `smelt.yml` `vars:` correctly; YAML scalar coercion handles strings, booleans, integers, floats, null per spec.
- [ ] `ConfigVarNullCoercion` is Warning severity (not Error).
- [ ] HOF inline-expansion frames carry the anonymous-frame shape (`fn_id = None`, `function = "<hof>"`, optional `element_index`); diagnostic round-trip preserves the frame stack.
- [ ] `expansion.md` cross-spec touch lands with the anonymous-frame form.
- [ ] `scoping.md` cross-spec touch lands with the lambda parameter scope rule.
- [ ] `type_inference.rs` purity preserved.
- [ ] Salsa wrappers around `resolve_smelt_yml_vars` correctly invalidate on `smelt.yml` change.

**Commit.** `feat(types): Phase B diagnostics + smelt.config.var + HOF expansion frames (meta-language Phase B)`

---

### Phase 4: LSP — hover, goto-def, completion for Phase B constructs

**Goal.** Implement LSP support for Phase B per `meta_language.md` §"LSP support required by Phase B": hover for lambdas (parameter type), HOF calls (result type), pipe expressions (un-piped result type), reducer names (input element type, output sort, identity), `smelt.config.var` calls (`Text` + resolved value); goto-def for lambda parameters (binder + body uses) and `smelt.config.var` arguments (resolves to `vars.x:` line in `smelt.yml`); completion for the bound lambda parameter inside body and for the closed reducer registry at the second-arg position of `reduce`. Rename support is deferred to Phase G.

**Pre-conditions.** Phases 1–3 done — parser, type-checker, diagnostics, smelt.config.var resolver all in place.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-lsp/src/lib.rs::tests::hover_lambda_parameter_in_body` — hover on `c` inside `map([1, 2, 3], fn c => c)` returns text containing the parameter type (e.g. `Expr<Integer>`).
- `crates/smelt-lsp/src/lib.rs::tests::hover_hof_call_returns_result_type` — hover on the `map(...)` call expression returns `List<U>` where `U` is the lambda body's synthesised type.
- `crates/smelt-lsp/src/lib.rs::tests::hover_pipe_expression_returns_unpiped_type` — hover on `xs |> filter(fn c => c > 0)` returns the same type as hover on `filter(xs, fn c => c > 0)`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_reducer_name_in_reduce_position` — hover on `union_all` in `reduce(xs, union_all)` returns text containing the input element type (`TableExpr`), output sort (`TableExpr`), and identity rule (`no identity`).
- `crates/smelt-lsp/src/lib.rs::tests::hover_reducer_name_with_identity` — hover on `and_all` returns identity `TRUE`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_smelt_config_var_resolved` — hover on `smelt.config.var('region')` over a workspace with `vars: { region: us-west-2 }` returns text containing `Text` and the resolved value `'us-west-2'`.
- `crates/smelt-lsp/src/lib.rs::tests::hover_smelt_config_var_unresolved` — hover on `smelt.config.var('not_declared')` returns `Text` and a hint that the variable is not declared (no crash; diagnostic anchoring is Phase 3's job).
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_lambda_parameter_resolves_to_binder` — goto-def on `c` inside the body of `map(xs, fn c => c)` resolves to the `c` token in the lambda parameter list.
- `crates/smelt-lsp/src/lib.rs::tests::goto_def_smelt_config_var_resolves_to_yml_line` — goto-def on the argument `'region'` of `smelt.config.var('region')` returns a Location pointing at the `vars.region:` line in `smelt.yml`.
- `crates/smelt-lsp/src/lib.rs::tests::completion_in_lambda_body_includes_parameter_first` — at a completion request inside the body of `fn c => |`, the completion list includes `c` as the first identifier completion.
- `crates/smelt-lsp/src/lib.rs::tests::completion_in_reduce_second_arg_offers_registry` — at a completion request at the second-arg position of `reduce(xs, |)` where `xs: List<Expr<Integer>>`, the completion list includes the reducers whose declared input is compatible with `Expr<Integer>` (i.e. `plus_chain`, `comma_sep`); reducers with incompatible input (e.g. `union_all` for `TableExpr`) are filtered out.
- `crates/smelt-lsp/src/lib.rs::tests::hover_does_not_panic_on_partial_lambda` — hover inside `map(xs, fn c =` (mid-edit, no body yet) does not crash; returns `Lambda<T, ?>` or no hover.

**Implementation shape.**

- `crates/smelt-lsp/src/lib.rs`:
  - Extend the existing hover handler to dispatch on `LAMBDA`, `LAMBDA_PARAM_LIST`, `PIPE_EXPR`, and the function-call dispatch to recognise HOF calls and `smelt.config.var` calls. For reducer-name hover, dispatch on the second-argument identifier of a `reduce` call.
  - Extend the goto-def handler:
    - For an identifier inside a lambda body that resolves (per the body's `TypeContext`) to a lambda parameter, return the parameter's binder span as the destination.
    - For the argument of `smelt.config.var('x')`, return a `Location` pointing at `<workspace_root>/smelt.yml` at the `vars.<x>:` line. Use the YAML parser's source-mapping if available; otherwise reasonable line-level resolution.
  - Extend the completion handler:
    - At a position inside a lambda body, prepend the bound parameter to the completion list.
    - At the second-arg position of a `reduce` call (after the first comma), offer the closed reducer registry, filtering by input-element compatibility when the first arg's type is inferable.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-lsp/src/lib.rs`

**Docs touched.**

- None.

**Review checklist** (material findings only):

- [ ] All TDD tests above exist and assert what's specified.
- [ ] Hover on a lambda parameter in body returns the bound type (the HOF's `T`); hover on a HOF call returns the result type; hover on a pipe expression returns the un-piped equivalent type; hover on a reducer name returns the registry entry's data; hover on `smelt.config.var` returns `Text` + resolved value.
- [ ] Goto-def on a lambda parameter resolves to its binder span.
- [ ] Goto-def on a `smelt.config.var('x')` argument resolves to the `vars.x:` line in `smelt.yml`.
- [ ] Completion inside a lambda body offers the bound parameter first.
- [ ] Completion at the second-arg position of `reduce` offers the closed registry, filtered by input-element compatibility when inferable.
- [ ] Hover never panics on a partially-parsed lambda or pipe expression — returns a placeholder type or no hover, not a crash.
- [ ] Existing hover/goto-def/completion paths (Phase A list literal + spread; `smelt.<path>` resolution) regress none of their tests.

**Commit.** `feat(lsp): hover/goto-def/completion for HOFs + lambdas + pipe + config-var (meta-language Phase B)`

---

### Phase 5: Examples fixture + smelt-app-builder skill + /smelt-loop medium tier

**Goal.** Land `examples/meta_hofs/`, a per-phase reference doc in the `smelt-app-builder` skill, and extend the `/smelt-loop` `medium` tier with at least one Phase B-specific ask. The fixture is the integration test for the Phase B surface; the skill update gives downstream agents the workflow knowledge; the loop tier extension is the auto-validation harness per meta-plan §4 obligation #6.

**Pre-conditions.** Phases 1–4 done — every Phase B code path can run end-to-end.

**TDD tests to write first.** Listed verbatim:

- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_clean_workspace` — running diagnostics over `examples/meta_hofs/` produces zero errors and warnings (excluding intentional `ConfigVarNullCoercion` warnings if a fixture exercises it).
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_lambda_in_forbidden_position` — running diagnostics over the broken sub-fixture produces exactly one `LambdaInForbiddenPosition`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_lambda_arity_not_supported` — same shape for `LambdaArityNotSupported`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_lambda_result_type_mismatch` — same shape for `LambdaResultTypeMismatch`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_hof_expects_lambda` — same shape for `HofExpectsLambda`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_hof_expects_reducer` — same shape for `HofExpectsReducer`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_hof_name_shadowed` — same shape for `HofNameShadowed`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_pipe_rhs_not_call` — same shape for `PipeRhsNotCall`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_reducer_input_type_mismatch` — same shape for `ReducerInputTypeMismatch`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_reducer_empty_no_identity` — same shape for `ReducerEmptyNoIdentity`.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_hofs_broken_config_var_not_found` — same shape for `ConfigVarNotFound`.

(Diagnostics with deferred test wiring — `ReducerNameShadowed`, `PipeInDataPosition`, `ConfigVarNameNotLiteral`, `ConfigVarNullCoercion` — may also be added; minimum is the eleven above. The acceptance-gate is "every Phase B code is covered by at least one test path — unit test in Phase 3 OR fixture test in Phase 5".)

**Implementation shape.**

- `examples/meta_hofs/`:
  - `smelt.yml` — workspace config with a `vars:` block (e.g. `region: us-west-2`, `min_revenue: 100`, `flag: true`, `nullable: ~`) for `smelt.config.var` exercises.
  - `models/surrogate_key.sql` — happy path: surrogate-key generic over `[a, b, c]` via `reduce(map(cols, fn c => CAST(c AS Text)), concat_with_sep)` — note: `concat_with_sep` is Phase F (parameterised reducer); for Phase B use `concat` over a hand-formatted `[CAST(a AS Text), '|', CAST(b AS Text), '|', CAST(c AS Text)]` list, OR use `comma_sep` in a SELECT-list context. Pick whichever shape lands a clean Phase B-only example.
  - `models/pipe_rewrite.sql` — happy path: a chain `[1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)` rendered into a SELECT.
  - `models/check_cols_with_config_var.sql` — happy path: uses `smelt.config.var('min_revenue')` to drive a threshold.
  - `models/and_all_predicates.sql` — happy path: `reduce([is_active, is_paid, is_validated], and_all)` rendered into a WHERE clause via splice.
  - `models/comma_sep_select_list.sql` — happy path: `SELECT [name, email, region] |> map(fn c => CAST(c AS Text)) |> reduce(comma_sep) FROM users` (or the syntactically compliant equivalent — pick the form that the spec's bidirectional-disambiguation rules accept).
  - `sources.yml` — declares the source tables used (`users`, `transactions`).
- Negative cases — under `examples/meta_hofs_broken_*/` (preferred to mirror Phase A's three-workspace layout for surgical per-code assertions). Each broken workspace exercises one Phase B code.
- `.claude/skills/smelt-app-builder/references/20260510-meta-hofs.md` — short workflow reference: "When to reach for `map`/`filter`/`reduce` vs spread; how `fn x => body` differs from named-args; closed reducer registry; `smelt.config.var` for env-conditional values; how to read HOF expansion-frame diagnostics" — point at user docs for syntax detail, capture only workflow gotchas.
- `/smelt-loop` `medium` tier extension — at least one new ask requiring Phase B surface (e.g. "rewrite this hand-coded `CASE x WHEN 'a' THEN ... WHEN 'b' THEN ...` chain via a list of pairs + `reduce(or_any)`"). The ask must be solvable with the shipped surface (no reflection, no records); failure modes correspond to Phase B diagnostics.

**Critical files (allowed to touch in this phase).**

- `examples/meta_hofs/**` (new tree)
- `examples/meta_hofs_broken_*/` for negative cases
- `crates/smelt-cli/tests/example_diagnostics.rs`
- `.claude/skills/smelt-app-builder/references/20260510-meta-hofs.md`
- `.claude/commands/smelt-loop.md` (or wherever the medium-tier fixture catalogue lives) — add at least one Phase B ask

**Docs touched.**

- The skill reference is a docs touch by `meta_language.md` §References → User docs aspirational entries; the skill reference complements `docs-site/`.

**Review checklist** (material findings only):

- [ ] Examples build cleanly in `cargo test -p smelt-cli --test example_diagnostics`.
- [ ] Each broken sub-fixture triggers exactly the one diagnostic code it targets — no double-reporting, no incidental other-phase diagnostics.
- [ ] No use of reflection (Phase C+), records (E1+), multi-model production (E2), parameterised reducers (F), or multi-arg lambdas (F) — Phase B surface only.
- [ ] Skill reference is short, points at user docs, captures only workflow advice (not syntax).
- [ ] `/smelt-loop` medium tier has at least one new Phase B ask; ask is solvable with the shipped surface; failure modes correspond to shipped diagnostics.

**Commit.** `feat(examples): meta_hofs fixture + skill reference + smelt-loop medium tier (meta-language Phase B)`

---

### Phase 6: User docs

**Goal.** Ship `docs-site/docs/meta-language/{hofs,lambdas,pipes,reducers,config-vars}.md` and extend `reference.md` with every Phase B construct, per the spec's References → User docs section. Every shipped construct, every diagnostic code, every spec rule that has user-visible consequences is documented.

**Pre-conditions.** Phases 1–5 done — code, fixtures, skill all match the spec.

**TDD tests to write first.** Documentation phases are validated by `/smelt:validate`, not by `cargo test`. The validation gate is:

- `/smelt:validate meta_language` reports zero drift between Surface section and `docs-site/docs/meta-language/{hofs,lambdas,pipes,reducers,config-vars,reference}.md`.

This phase has no Rust unit tests. The acceptance gate is content review (the reviewer checklist below) + `/smelt:validate` running clean.

**Implementation shape.**

- `docs-site/docs/meta-language/lambdas.md`:
  - The `fn x => body` syntax, plain-language explanation.
  - Single-arg only (multi-arg deferred to Phase F).
  - Lambda parameter scoping (lambda param wins over outer `smelt.define` param inside body).
  - Lambda capture rules (captures meta-only outer scope; cannot capture runtime SQL columns).
  - Forbidden positions (must be a HOF positional argument).
  - Diagnostic codes — `LambdaInForbiddenPosition`, `LambdaArityNotSupported`, `LambdaResultTypeMismatch`. Each entry has a "what it means" + "what to fix" hint.
- `docs-site/docs/meta-language/hofs.md`:
  - The three HOFs (`map`, `filter`, `reduce`) with type signatures and one worked example each, drawn from `examples/meta_hofs/`.
  - Reserved names (no `smelt.define` may shadow `map`/`filter`/`reduce`).
  - Diagnostic codes — `HofExpectsLambda`, `HofExpectsReducer`, `HofNameShadowed`.
- `docs-site/docs/meta-language/pipes.md`:
  - The `|>` operator, first-arg semantics, left-associative.
  - Pipe is meta-only and parser-level desugaring.
  - Diagnostic codes — `PipeRhsNotCall`, `PipeInDataPosition`.
- `docs-site/docs/meta-language/reducers.md`:
  - The closed registry of seven reducers (`comma_sep`, `and_all`, `or_any`, `union_all`, `intersect_all`, `plus_chain`, `concat`).
  - Each entry: input element type, output sort, empty-list identity (or "no identity"), one worked example.
  - Reserved names (no `smelt.define` may shadow a reducer name).
  - Diagnostic codes — `ReducerInputTypeMismatch`, `ReducerEmptyNoIdentity`, `ReducerNameShadowed`.
- `docs-site/docs/meta-language/config-vars.md`:
  - `smelt.config.var(name)` syntax and signature.
  - Literal-only argument in v1 (expression-valued reserved for E1).
  - YAML scalar coercion rules (string round-trip, boolean/integer/float to text, null + warning).
  - Worked examples drawn from `examples/meta_hofs/`.
  - Diagnostic codes — `ConfigVarNotFound`, `ConfigVarNameNotLiteral`, `ConfigVarNullCoercion`.
- `docs-site/docs/meta-language/reference.md`:
  - Append entries (alphabetical insertion) for: `and_all`, `comma_sep`, `concat`, `filter`, `fn` (lambda keyword), `intersect_all`, `map`, `or_any`, `plus_chain`, `reduce`, `smelt.config.var`, `union_all`, `|>` (pipe). Each entry: short type signature / syntax + one-line example.
  - Phase A entries remain alphabetised correctly.
- `docs-site/docs/meta-language/index.md`:
  - Update the phase-coverage table: Phase B is now "user-visible content shipped".
  - Add cross-links to the new pages.

**Critical files (allowed to touch in this phase).**

- `docs-site/docs/meta-language/index.md` (update)
- `docs-site/docs/meta-language/hofs.md` (new)
- `docs-site/docs/meta-language/lambdas.md` (new)
- `docs-site/docs/meta-language/pipes.md` (new)
- `docs-site/docs/meta-language/reducers.md` (new)
- `docs-site/docs/meta-language/config-vars.md` (new)
- `docs-site/docs/meta-language/reference.md` (extend)
- `docs-site/sidebars.ts` (or equivalent navigation file) — add the new pages to the meta-language section

**Docs touched.**

- All seven user-docs files above.

**Review checklist** (material findings only):

- [ ] Every Surface item from `meta_language.md` Phase B appears in user docs.
- [ ] Every Phase B diagnostic code is documented with a "what to fix" hint.
- [ ] Every reducer's empty-list identity is documented (or its absence).
- [ ] No syntax appears in docs that is not speced.
- [ ] Reference page is alphabetical and complete (Phase A + Phase B entries).
- [ ] `/smelt:validate meta_language` reports zero drift.
- [ ] Worked examples are runnable — they correspond to the `examples/meta_hofs/` fixture.

**Commit.** `docs(meta-language): user-docs hofs + lambdas + pipes + reducers + config-vars + reference (meta-language Phase B)`

---

### Phase 7: Expert reviewer dispatch loop

**Goal.** Run each Phase B applicable expert reviewer from meta-plan §5 over the Phase B diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below. This phase is the realisation of the user's original ask: "Use expert reviews by subagents with specific context to help guide the implementation."

**Pre-conditions.** Phases 1–6 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase B subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **parser-expert** | sonnet | `crates/smelt-parser/src/{lexer,parser,ast,syntax_kind}.rs` | `FN_KW` and `PIPE_ARROW` lexer additions do not regress identifier or `||` (string-concat) tokenisation; lambda CST shape correct; pipe is left-associative and lowest-precedence; multi-arg lambdas parse to a CST node for Phase 3 rejection (parser does not gate); recursive-descent depth/recovery invariants intact. |
| **type-expert** | sonnet | `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/type_inference.rs` | `SmeltType::Lambda` addition is non-breaking (no missed exhaustive matches); HOF dispatch binds lambda parameter type from `xs`'s element type (bidirectional); reducer registry is closed and registry lookup is by-name; pipe desugars at AST level; `Lambda<T, U>` is invariant; `type_inference.rs` purity preserved. |
| **expansion-expert** | sonnet | `crates/smelt-db/src/function_body_check.rs`, `docs/specs/expansion.md` | HOF inline-expansion frames carry the anonymous-frame shape (`fn_id = None`, `function = "<hof>"`); frame-stack innermost-first ordering preserved; `expansion.md` Phase B touch is normative for the new frame form; multi-frame chains crossing a HOF preserve `Caller`/`Callee`/anonymous-frame provenance correctly. |
| **lsp-expert** | sonnet | `crates/smelt-lsp/src/lib.rs` | Hover on lambda parameter / HOF call / pipe expression / reducer name / `smelt.config.var` returns the spec'd content; goto-def on lambda parameter binder + body uses + `smelt.config.var` arg works; completion in lambda body offers the bound parameter; completion at `reduce` second-arg offers the registry; spans line up with CST; no panics on partial parses; no regressions in Phase A hover paths. |
| **examples-curator** | haiku | `examples/meta_hofs/` (and broken sub-fixtures) | Fixture is minimal-but-realistic; covers the happy path (HOF chain, pipe, reducer, `smelt.config.var`) + at least one diagnostic edge case for each new Phase B code; passes `cargo test -p smelt-cli --test example_diagnostics`. |
| **docs-reviewer** | haiku | `docs-site/docs/meta-language/{index,hofs,lambdas,pipes,reducers,config-vars,reference}.md` | Every Surface item from `meta_language.md` Phase B is documented; every Phase B diagnostic code has a "what to fix" hint; reference page is alphabetical and complete (Phase A + Phase B); no syntax appears in docs that is not speced. |

**Loop discipline.**

1. **Round 1.** Dispatch all six experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - The phase plan path (`docs/plans/20260509-meta-language-B.md`) and the spec sections that are the oracle (`docs/specs/meta_language.md` Phase B, plus `expansion.md` / `scoping.md` / `types.md` cross-touches).
   - The exact file scope from the table above.
   - The diff range to review (commits since the start of Phase B — typically `git log --oneline d4d4586..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into other phases.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-B): address {expert-name} feedback` (e.g. `review(meta-language-B): address parser-expert feedback`).
   - Push after each commit (so the user sees progress on PR #117).

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were addressed, not the whole panel. Provide the same prompt as round 1 plus a diff of what changed since round N−1. If the expert returns "no material findings", that expert is **clean** and exits the loop.

4. **Repeat** step 2 → step 3 until **every** expert is clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of the following fires:
   - Same expert flags a material finding on round 3 (per-expert bound). The third repeat means the fix is wrong or the spec is wrong; the user must arbitrate.
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7). That's a design problem, not an implementation problem.
   - An expert's findings would force a spec change. Run `/smelt:spec meta_language` first; if the spec edit is non-trivial or contentious, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase B. Pause; the autonomy loop should not silently absorb pre-existing breakage.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus `docs/plans/20260509-meta-language-B.md` (to record the round count and final clean status).

**Docs touched.** None new — fixes may amend `docs-site/docs/meta-language/*` if the docs-reviewer flags a surface drift; or `docs/specs/expansion.md` / `docs/specs/scoping.md` / `docs/specs/types.md` if an expert flags a cross-spec touch drift.

**Review checklist** (material findings only — applied to the expert-dispatch *process*, not to a code diff):

- [ ] All six experts were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below (see acceptance gate).
- [ ] No fix touched files outside the dispatching expert's scope (no scope creep).
- [ ] No expert ran more than 3 rounds; if any did, the autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 7 expert review: parser-expert clean (R{n}), type-expert clean (R{n}), expansion-expert clean (R{n}), lsp-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-B): address {expert-name} feedback`. If round 1 came back clean for an expert, no commit for that expert. The acceptance-gate summary line lands in the next commit naturally (or in a tiny `chore(meta-language-B): record Phase 7 review summary` if no other phase-7 commits were made).

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 1 `LambdaArityNotSupported` is latent in Phase B.** Round 2 reviewer flagged that `is_fn_lambda_start()` originally matched LPAREN, mis-parsing `SELECT fn(x, y) FROM t` as a multi-arg lambda. Fix restricted detection to `Some(IDENT)` only — multi-arg lambda CST nodes are no longer produced in Phase B, so the `LambdaArityNotSupported` diagnostic from the spec cannot fire in Phase B. The diagnostic remains in the spec for Phase F when multi-arg lambdas land. User-facing impact in Phase B: `fn (a, b) => body` produces a generic parse error rather than the targeted `LambdaArityNotSupported` diagnostic. Reviewer judged this acceptable; orchestrator deferred to Phase F.
- **Phase 2 round 1 reviewer surfaced 3 MUST FIX findings** — `Box::leak` in production HOF inference (LSP server memory leak risk); `comma_sep` empty-identity scattered outside `REDUCER_REGISTRY` (registry-as-single-source-of-truth invariant violated); `types.md` Lambda entry missing the required prohibition phrase. Round 2 fixed all three: `HofSecondArg::Lambda` now owns the `Lambda` (no leak); `EmptyIdentity::EmptySelectItems` added to enum, `comma_sep` registry entry uses it, `reducer_name == "comma_sep"` special-case removed; `types.md` entry now includes the meta-only-not-user-writable prohibition with `LambdaInForbiddenPosition` enforcement note. Grep evidence confirmed: zero `Box::leak`, zero `reducer_name == "comma_sep"` references in `type_inference.rs`. Two round-1 OBSERVATIONs accepted as-is (`or_any`/`and_all` share `EmptyIdentity::Boolean` — type is correct, identity value is codegen concern; FILTER_KW fallback is safe in practice — no other HOF name is a reserved SQL keyword in the lexer).
- **Phase 5 surfaced four Phase 3 wiring gaps that landed in the Phase 5 commit** rather than as a Phase 3 amendment. Reviewer judged `accept-as-is` because each was causally required for a Phase 5 broken-fixture test to pass and each was minimal (no new behaviour, just orchestration wiring Phase 3's unit-tested pure helpers). The four: (1) `check_define_name_shadowing` was inside the `if let Some(ast) = AstFile::cast(syntax)` block that runs after the `parse_model.is_none()` early return, so function-only files (like `meta_hofs_broken_hof_name_shadowed/functions/shadowed_hof.sql` with no SELECT) never reached it — moved unconditionally before the early return. (2) `check_config_var_call_diagnostics` (new pure function in `type_inference.rs`) walks the syntax tree to wire `ConfigVarNotFound` / `ConfigVarNameNotLiteral` / `ConfigVarNullCoercion` from real workspaces — Phase 3's unit tests called pure helpers but the Salsa orchestration in `check_file_diagnostics` was missing. (3) HOF names exempted from the `UnrecognizedFunction` warning in `check_expression_types` so that `map(...)` calls don't co-fire with the HOF-specific diagnostic. (4) `smelt_fn_call_diagnostics_for_file` skips `smelt.config.var(...)` so the generic smelt-function checker doesn't co-fire with `ConfigVarNotFound`/etc. These together preserve the "exactly one Phase B code" invariant in every broken fixture.

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes.
- `cargo test -p smelt-cli --test example_diagnostics` passes — `examples/meta_hofs/` clean, broken sub-fixtures report the exact Phase B diagnostic codes.
- `/smelt:validate meta_language` reports zero drift.
- LSP smoke test in `examples/meta_hofs/`: hover, goto-def, completion all work for HOFs / lambdas / pipe / reducers / `smelt.config.var` per spec.
- Phase 7 acceptance gate met: every applicable expert reviewer (parser-expert, type-expert, expansion-expert, lsp-expert, examples-curator, docs-reviewer) reported "no material findings" on its final dispatch, recorded in "Deferred during implementation" with round counts per expert. No stop-the-line condition fired.
