# Plan: Meta-Language Phase F — Polish (parameterised reducers, multi-arg lambdas, meta-world ternary)

**Date**: 2026-05-16
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md) §"Lambdas and higher-order functions" (Surface — Lambda syntax for `fn (a, b) => body`; Lambda + HOF diagnostic codes); §"Contextual reducers" (Surface — Bare reducers, Parameterised reducers, Reducer diagnostic codes); §"Meta-world ternary" (Surface, Diagnostic codes, LSP); §"Per-construct semantics — Lambdas and HOFs" rule 1–2; §"Per-construct semantics — Contextual reducers" rule 1; §"Per-construct semantics — Meta-world ternary" rules 1–7; §"Design — Lambdas and HOFs" multi-arg paragraphs; §"Design — Reducers" parameterised-reducer paragraphs; §"Design — Ternary"; §"Constraints — Lambda and HOF invariants", §"Reducer invariants", §"Ternary invariants".
**Spec diff**: uncommitted working-tree diff to `docs/specs/meta_language.md` (will commit together with this plan in Phase 0).
**Tracking PR / branch**: PR #117 (`feat: typed meta-programming`) — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this plan in full. Then read the spec at `docs/specs/meta_language.md` §"Lambdas and higher-order functions", §"Contextual reducers", §"Meta-world ternary", and the corresponding Semantics / Design / Constraints sections. The spec is the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec meta_language` to revise the spec rather than encode the divergence in code.
2. Confirm you are on branch `research/typed-meta-programming`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 7 is the expert-reviewer dispatch loop** — after Phases 1–6 commit, dispatch the meta-plan §5 expert reviewers applicable to F (type-expert, lsp-expert, examples-curator, docs-reviewer), address material findings, and re-dispatch each expert until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 7. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 7's acceptance gate is met.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` first to update).
- `cargo test` or `cargo clippy --all-targets` surfaces a pre-existing failure unrelated to the plan.
- Phase 7: an expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in the same round.
- The `SmeltType::Lambda` widening from `(Box<SmeltType>, Box<SmeltType>)` to `(Vec<SmeltType>, Box<SmeltType>)` forces non-trivial fallout in `types.md` invariants — meta-plan §7's "cross-feature impact wider than predicted" stop-the-line applies. Note: `types.md` is *not* listed as a Phase F cross-feature touch in the meta-plan §6 table; if the widening cascades there, pause.

**Conventions every phase:**

- Real-fixture test under `examples/meta_polish/` — Phase 4 exercises the full surface there; earlier phases have unit tests in `crates/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no `zip_with` (defer unless an example forces it); no LSP rename (Phase G); no `Array<U>(…)` runtime-array constructor; no user-defined reducers (post-plan); no `Optional<T>` (post-plan).
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference/` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic). Salsa queries call pure inference functions, not the other way around.
- Timeless-oracle rule: spec and user-doc edits read as if the feature has always existed. Phase vocabulary lives in this plan only — never inside `docs/specs/` or `docs-site/docs/` body sections.

---

## Context

The meta-language Phase F spec increment landed in this commit's working-tree edits to `docs/specs/meta_language.md`. Three polish surfaces are now normatively specified:

- **Multi-argument lambdas.** `fn (a, b) => body` is admitted alongside `fn x => body`. Arity is fixed at the literal and matched against the HOF call site's required arity. `LambdaArityNotSupported` is retired in favour of `LambdaArityMismatch` (wrong arity for the HOF), `LambdaZeroParameters` (`fn () => body`), and `LambdaDuplicateParameter` (`fn (a, a) => body`). The internal type witness widens from `Lambda<T, U>` to `Lambda<(T_1, …, T_k), U>` — i.e. `SmeltType::Lambda(Vec<SmeltType>, Box<SmeltType>)`.

- **Parameterised reducers.** The closed reducer registry now admits parameter-bearing entries. v1 ships exactly one: `concat_with(sep: Text)`, used as `reduce(xs, concat_with(' OR '))`. The call shape is parsed at the second argument of `reduce` (and only there); arguments are positional, compile-time-resolvable, and type-checked against the registry entry's declared parameter types. Four new diagnostic codes anchor the failure modes: `ReducerArityMismatch`, `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`, `ReducerNamedArgument`.

- **Meta-world ternary.** `if cond then a else b` is a new compile-time value expression. `if`, `then`, `else` are reserved meta-namespace keywords. The condition must be `Boolean`; the branches unify under LUB; evaluation is short-circuit (the unreached branch is type-checked but not evaluated, suppressing runtime-style diagnostics like `MapGetMissingKey`). Right-associative chaining gives `if c1 then a else if c2 then b else c` natural reading without an `elif` token. Seven new diagnostic codes cover the surface (`TernaryConditionNotBoolean`, `TernaryBranchTypeMismatch`, `TernaryKeywordShadowed`, `TernaryInDataPosition`, `TernaryDanglingThen`, `TernaryDanglingElse`).

This plan drives the implementation, the worked example, user docs, and skill update for that surface. Phase F is the seventh of eight implementation phases (A–G); it composes with Phase B's HOFs / reducers / lambdas and Phase E1's records / maps to deliver `examples/meta_polish/` — a model that exercises `concat_with(sep)`, multi-arg lambda, and `if cond then a else b` together.

`zip_with` remains deferred per meta-plan §3 ledger: ship only if a Phase F example forces it. The current `examples/meta_polish/` design does not require `zip_with`, so the example does not force it.

## Scope

### In scope (spec coverage)

- `meta_language.md` Surface for multi-arg lambdas: `fn (IDENT_1, …, IDENT_k) => EXPR` parenthesised-parameter-list form; equivalent `fn (x) => body` single-arg form; `LambdaArityMismatch`, `LambdaZeroParameters`, `LambdaDuplicateParameter` diagnostic codes; updated LSP hover/completion behaviour for multi-parameter lambdas.
- `meta_language.md` Surface for parameterised reducers: the `concat_with(sep: Text)` registry entry; the parameterised-reducer call shape at `reduce`'s second argument; `ReducerArityMismatch`, `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`, `ReducerNamedArgument` diagnostic codes; updated LSP hover/completion behaviour at parameterised-reducer call sites.
- `meta_language.md` Surface for the meta-world ternary: `if COND then THEN_EXPR else ELSE_EXPR` syntax; `if`/`then`/`else` keyword reservations; right-associative chaining; pipe-precedence rule (ternary lower than `|>`); `TernaryConditionNotBoolean`, `TernaryBranchTypeMismatch`, `TernaryKeywordShadowed`, `TernaryInDataPosition`, `TernaryDanglingThen`, `TernaryDanglingElse` diagnostic codes; LSP hover/completion/goto-def behaviour for the keywords.
- `meta_language.md` Semantics §"Lambdas and HOFs" rules 1–2 — k-arity lambda formation, coterminous parameter scoping, duplicate-parameter detection.
- `meta_language.md` Semantics §"Contextual reducers" rule 1 — parameterised-reducer argument evaluation, compile-time-resolvability, type matching, identity preservation.
- `meta_language.md` Semantics §"Meta-world ternary" rules 1–7 — condition typing, branch typing (LUB), short-circuit evaluation, `Unknown` propagation, no scope introduced, determinism, termination.
- `meta_language.md` Constraints §"Lambda and HOF invariants", §"Reducer invariants", §"Ternary invariants" — all polish-related rows.
- `meta_language.md` Known Divergences entry — replace the "Polish surfaces are normatively specified above; the implementation is pending" bullet with a recap of what shipped once Phase 5 ships, leaving `zip_with` deferred-by-default in the theoretical-completeness ledger.
- LSP support for every Phase F surface element: hover on multi-arg lambda parameter-list opening `(` (full `Lambda<…>` signature); hover on each lambda parameter (its bound type); hover on a parameterised-reducer call (parameter signature + statically-known argument values); hover on `if`/`then`/`else` keywords (ternary inferred type and per-branch types); completion at `reduce(xs, <cursor>)` offering parameterised entries as call snippets; completion at the start of a meta-evaluable position offering `if` as a snippet expanding to `if $cond then $then else $else`; goto-def on the keywords resolving to the reference page (URL hint, graceful no-op when the client lacks support).
- Example fixture `examples/meta_polish/` exercising all three polish surfaces in one model, gated by `crates/smelt-cli/tests/example_diagnostics.rs`.
- User docs at `docs-site/docs/meta-language/ternary.md` (new) and updates to `lambdas.md`, `reducers.md`, `reference.md` — every new construct documented with type signatures and a worked example.
- `smelt-app-builder` skill: per-phase reference doc at `.claude/skills/smelt-app-builder/references/20260516-meta-polish.md` covering workflow gotchas only (e.g. "ternary keywords cannot be `smelt.define` names"; "parameterised reducer arguments must be compile-time").

### Explicitly deferred

- `zip_with` and any other multi-list HOF — meta-plan §3 ledger says "ship only if an example forces it"; the `examples/meta_polish/` design does not force it. Spec mentions `zip_with` only as a future extension.
- User-defined reducers — post-plan; closed registry remains the v1 surface.
- `Optional<V>` / softer `m.get` returns — post-plan; the ternary + `m.has(k)` pattern is the v1 defaulting surface.
- LSP rename for new constructs (`if`/`then`/`else`, lambda parameters, parameterised reducer call) — Phase G.
- `/smelt-loop` `large` tier — Phase G.
- Performance optimisation — concrete profiling is a Phase G concern.
- Backwards-compatibility shims for pre-F workspaces — none required; `if`/`then`/`else` keywords are new and `concat_with` is a new bare identifier in the closed registry. Existing `fn x => body` lambdas continue to parse and check identically.
- `Array<U>(…)` runtime-array constructor — deferred per meta-language Known Divergences; does not interact with F.
- Path-component identifier lift, CTE-name lift, table-alias lift — out-of-scope per meta-language Out-of-scope list.

## Progress tracking

| Phase | Status   | Commit  | Date       |
|-------|----------|---------|------------|
| 0     | done     | 8b25f70 | 2026-05-16 |
| 1     | done     | 3b3e1a2 | 2026-05-16 |
| 2     | done     | 2942a70 | 2026-05-16 |
| 3     | done     | 58fdd2d | 2026-05-16 |
| 4     | done     | bbcde26 | 2026-05-17 |
| 5     | pending  |         |            |
| 6     | pending  |         |            |
| 7     | pending  |         |            |

---

## Phases

### Phase 0: Commit the spec increment

**Goal.** Land the working-tree spec edits to `docs/specs/meta_language.md` and this plan as a single atomic commit that opens Phase F.

**Pre-conditions.** Working tree contains the Phase F spec edits to `meta_language.md` (introducing multi-arg lambda surface, parameterised reducer surface, ternary surface, the corresponding Semantics / Design / Invariants entries, the diagnostic-code updates, the LSP additions, and the Known Divergences rewrite of the polish line). This plan file at `docs/plans/20260509-meta-language-F.md` is staged. No Phase F code changes anywhere in `crates/`. The overall plan's Phase F row still reads `pending`.

**TDD tests to write first.** None — Phase 0 is the spec + plan commit. Code TDD starts in Phase 1.

**Implementation shape.** Verify the spec diff via `git diff docs/specs/meta_language.md`; cross-check that all 14 new diagnostic codes listed in this plan's Context appear in §Surface tables under the appropriate construct; confirm no `Phase ` plan vocabulary leaked into the spec body sections; confirm the Known Divergences polish bullet now references this plan path; confirm References → Plans (history) lists this plan.

**Critical files (allowed to touch in this phase).**
- `docs/specs/meta_language.md` — Phase F spec increment (working tree).
- `docs/plans/20260509-meta-language-F.md` — this plan.

**Docs touched.**
- `docs/specs/meta_language.md` — already in working tree.
- `docs/plans/20260509-meta-language-overall.md` — leave the Phase F row at `pending` (it flips to `done` only after Phase 7 acceptance).

**Review checklist** (material findings only):
- [ ] Spec changes match the scope listed in this plan's Scope §"In scope" precisely.
- [ ] No phase vocabulary (`### Phase F …`, `(Phase F)`, `[deferred to Phase F]`) leaked into spec body sections.
- [ ] The 14 new diagnostic codes are listed in §Surface tables under their respective constructs.
- [ ] The Known Divergences polish entry references this plan file.
- [ ] References → Plans (history) lists this plan.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test --quiet` all pass (no code change, must be green).

**Commit.** `spec(meta-language-F): author polish surface (multi-arg lambdas, parameterised reducers, ternary)`

---

### Phase 1: Parser + lexer + CST nodes

**Goal.** Add lexer tokens (`IF_KW`, `THEN_KW`, `ELSE_KW`), CST nodes (`TERNARY_EXPR`, `REDUCER_CALL`, multi-arg `LAMBDA`), and parser productions for all three polish surfaces. Pure parser work; no type-inference rules yet.

**Pre-conditions.** Phase 0 committed. The lexer currently recognises `FN` as the lambda keyword; adding three more keywords is mechanical. The parser currently has a `LAMBDA` production accepting `fn IDENT => EXPR`; the multi-arg form `fn ( IDENT ( , IDENT )* ) => EXPR` is a small extension at the same production. `REDUCER_CALL` is a new CST node parsed only at the second argument of `reduce`. `TERNARY_EXPR` is a new low-precedence expression form.

**TDD tests to write first.**

- `crates/smelt-parser/src/lexer.rs::tests::lex_ternary_keywords` — `if`, `then`, `else` lex as `IF_KW`, `THEN_KW`, `ELSE_KW`; they are distinct from identifiers; case-sensitive.
- `crates/smelt-parser/src/lexer.rs::tests::lex_keywords_not_in_strings` — `'if'`, `'then'`, `'else'` inside string literals continue to lex as `STRING_LITERAL`.
- `crates/smelt-parser/src/parser/tests.rs::parse_multi_arg_lambda` — `fn (a, b) => a + b` parses as `LAMBDA` with two `LAMBDA_PARAM` children.
- `crates/smelt-parser/src/parser/tests.rs::parse_multi_arg_lambda_trailing_comma` — `fn (a, b,) => body` accepted.
- `crates/smelt-parser/src/parser/tests.rs::parse_single_arg_lambda_parenthesised` — `fn (x) => x` accepted (equivalent surface to `fn x => x`).
- `crates/smelt-parser/src/parser/tests.rs::parse_lambda_zero_params_rejected` — `fn () => body` produces a parse-recovered `LAMBDA` flagged for the downstream `LambdaZeroParameters` diagnostic (the parser does not crash).
- `crates/smelt-parser/src/parser/tests.rs::parse_lambda_no_parens_multi_arg_rejected` — `fn a, b => body` is a parse error at the comma (recovers to a `LAMBDA` with one param plus an `ERROR` token).
- `crates/smelt-parser/src/parser/tests.rs::parse_ternary_basic` — `if cond then a else b` parses as `TERNARY_EXPR` with three sub-expressions.
- `crates/smelt-parser/src/parser/tests.rs::parse_ternary_nested_right_associative` — `if c1 then a else if c2 then b else c` parses as `TERNARY_EXPR(c1, a, TERNARY_EXPR(c2, b, c))`.
- `crates/smelt-parser/src/parser/tests.rs::parse_ternary_in_lambda_body` — `fn x => if x > 0 then 'pos' else 'neg'` parses with the ternary as the lambda's body.
- `crates/smelt-parser/src/parser/tests.rs::parse_ternary_in_pipe_chain` — `xs |> filter(fn x => x > 0) |> reduce(plus_chain)` followed by `if c then a else b` parses correctly under the pipe-lower-than-ternary precedence rule.
- `crates/smelt-parser/src/parser/tests.rs::parse_ternary_dangling_then_recovery` — `then x else y` (no `if`) recovers as an `ERROR` at `then` without consuming the surrounding expression.
- `crates/smelt-parser/src/parser/tests.rs::parse_ternary_dangling_else_recovery` — `if c then x` followed by no `else` recovers as an incomplete `TERNARY_EXPR` with the `else` slot missing (flagged for the downstream `TernaryDanglingElse` diagnostic).
- `crates/smelt-parser/src/parser/tests.rs::parse_reducer_call` — `reduce(xs, concat_with(' OR '))` parses with a `REDUCER_CALL` node containing the identifier `concat_with` plus the argument list `(' OR ',)`.
- `crates/smelt-parser/src/parser/tests.rs::parse_reducer_call_bare_identifier_still_works` — `reduce(xs, and_all)` continues to parse as a bare-identifier reducer second-argument (no `REDUCER_CALL` node).
- `crates/smelt-parser/src/parser/tests.rs::parse_reducer_call_at_non_reduce_position_rejected` — `concat_with('|')` at a top-level expression position emits `UnknownIdentifier`-style recovery; the parser does not produce a `REDUCER_CALL` node outside `reduce`'s second-argument context.

**Implementation shape.**

- `crates/smelt-parser/src/lexer.rs` — add `IF_KW`, `THEN_KW`, `ELSE_KW` token kinds and the corresponding `Logos`-style match patterns. Keep `||` before `|>` ordering rule unchanged.
- `crates/smelt-parser/src/syntax_kind.rs` — add `IF_KW`, `THEN_KW`, `ELSE_KW`, `TERNARY_EXPR`, `REDUCER_CALL`. Confirm `LAMBDA` continues to be the node for both single- and multi-arg surfaces (no new node — the multi-arg parameter list just produces more `LAMBDA_PARAM` children inside the existing `LAMBDA` node).
- `crates/smelt-parser/src/parser/expr.rs` — extend the lambda production to accept the parenthesised parameter list; emit `LAMBDA_PARAM` for each identifier; track parameter-list arity in the CST as the count of children (no special node).
- `crates/smelt-parser/src/parser/expr.rs` — add `parse_ternary` at lowest meta-language precedence: `if EXPR then EXPR else EXPR`. Each `EXPR` slot parses with the standard expression parser (not the ternary-only path). The `else` branch invokes `parse_ternary` recursively (right-associative chaining without `elif`).
- `crates/smelt-parser/src/parser/expr.rs` — keep `parse_pipe` higher-precedence than `parse_ternary` per the precedence rule; this means after `parse_pipe` returns, the parser checks for `if` and folds the just-parsed expression into the ternary's `COND` slot. Actually a cleaner shape: `parse_ternary` is the top-level expression entry point, dispatching to `parse_pipe` for the slots — see the existing pattern for `parse_or`-then-pipe chains.
- `crates/smelt-parser/src/parser/expr.rs` — recovery: a `then` keyword without a leading `if` emits `ERROR(TernaryDanglingThen)` and skips the token; a missing `else` slot completes the `TERNARY_EXPR` with no `else` child, flagged for the downstream `TernaryDanglingElse` diagnostic.
- `crates/smelt-parser/src/parser/expr.rs` — at the second argument of `reduce`, parse either a bare reducer identifier (existing behaviour) or a reducer-call form (`IDENT (` lookahead, followed by argument-list parsing and `RPAREN`). Produce `REDUCER_CALL` only in this context; in any other context the same syntax is parsed as a generic function call.
- `crates/smelt-parser/src/ast.rs` — typed AST wrappers: `Lambda::params() -> Vec<LambdaParam>` (returning the full vector, not just `Option<LambdaParam>`); `TernaryExpr::condition() / then_branch() / else_branch()`; `ReducerCall::name() / args()`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/lexer.rs`
- `crates/smelt-parser/src/syntax_kind.rs`
- `crates/smelt-parser/src/parser/{mod,expr}.rs`
- `crates/smelt-parser/src/ast.rs`
- `crates/smelt-parser/src/parser/tests.rs`
- `crates/smelt-parser/src/lexer.rs::tests`

**Docs touched.** None in this phase. Phase 5 lands the user docs.

**Review checklist** (material findings only):
- [ ] The 15 TDD tests listed above exist and pass.
- [ ] Existing single-arg lambda tests still pass (no regression).
- [ ] Existing bare-reducer-identifier tests still pass.
- [ ] Existing pipe-chain tests still pass with the new ternary added.
- [ ] Lexer changes do not alter the relative ordering of `||` vs `|>`.
- [ ] CST node count (`TERNARY_EXPR`, `REDUCER_CALL`) and tokens (`IF_KW`, `THEN_KW`, `ELSE_KW`) match the spec's References → Code entries.
- [ ] No diagnostic emission yet — Phase 3 wires the parse-error-flagged nodes into `file_diagnostics`.
- [ ] No scope creep into type inference (no Lambda `Vec<SmeltType>` widening — that's Phase 2).
- [ ] Pure-parser invariant preserved (no `smelt-db` or Salsa imports in `crates/smelt-parser/`).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test -p smelt-parser` all green.

**Commit.** `feat(meta-language-F): parser support for multi-arg lambdas, parameterised reducer calls, meta-world ternary`

---

### Phase 2: Type inference

**Goal.** Pure type-inference rules for multi-arg lambdas, parameterised reducer calls, and the ternary. Widen `SmeltType::Lambda` from `(Box<SmeltType>, Box<SmeltType>)` to `(Vec<SmeltType>, Box<SmeltType>)`. Add per-construct registries (`PARAMETERISED_REDUCER_REGISTRY`). All logic lives in `crates/smelt-db/src/type_inference/` as pure functions; Salsa queries are unchanged.

**Pre-conditions.** Phase 1 committed; the CST nodes and AST wrappers exist. `SmeltType::Lambda(Box<SmeltType>, Box<SmeltType>)` is the current shape in `crates/smelt-types/src/signatures.rs`. The bare-reducer registry lives in `crates/smelt-db/src/type_inference/hof.rs`. HOF dispatch in `hof.rs` binds the lambda parameter type from the HOF's `T`.

**TDD tests to write first.**

- `crates/smelt-types/src/signatures.rs::tests::lambda_vec_arity` — `SmeltType::Lambda(vec![Integer, Integer], Box::new(Text))` is distinct from `SmeltType::Lambda(vec![Integer], Box::new(Text))` under equality; `assignable_from` returns false for mismatched arities.
- `crates/smelt-types/src/signatures.rs::tests::lambda_vec_display` — multi-arg lambda renders as `Lambda<(Integer, Integer), Text>` (or equivalent agreed format); single-arg renders as `Lambda<Integer, Text>` for backward readability (no `(…)` wrapper for arity-1).
- `crates/smelt-db/src/type_inference/hof.rs::tests::map_rejects_multi_arg_lambda` — `map(xs, fn (a, b) => a + b)` over `List<Integer>` emits `LambdaArityMismatch` ("map expects a lambda of arity 1; found arity 2").
- `crates/smelt-db/src/type_inference/hof.rs::tests::filter_rejects_multi_arg_lambda` — same pattern for `filter`.
- `crates/smelt-db/src/type_inference/hof.rs::tests::lambda_zero_params_diagnostic` — `map(xs, fn () => 1)` (if parser produces a zero-param lambda for `fn () =>`) emits `LambdaZeroParameters`.
- `crates/smelt-db/src/type_inference/hof.rs::tests::lambda_duplicate_parameter_diagnostic` — Hypothetical surface (no HOF in v1 uses multi-arg, so duplicate-param check is at the lambda formation site itself, not the HOF). Test that `fn (a, a) => a` formed in any HOF positional argument position emits `LambdaDuplicateParameter` at the second `a`'s span.
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_concat_with_text_separator` — `reduce(xs: List<Expr<Text>>, concat_with(' OR '))` synthesises `Expr<Text>` with empty-list identity `''`.
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_concat_with_arity_mismatch` — `reduce(xs, concat_with())` emits `ReducerArityMismatch` ("reducer concat_with expects 1 argument; found 0").
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_concat_with_too_many_args` — `reduce(xs, concat_with(' OR ', ' AND '))` emits `ReducerArityMismatch` ("reducer concat_with expects 1 argument; found 2").
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_concat_with_wrong_arg_type` — `reduce(xs, concat_with(42))` emits `ReducerArgTypeMismatch` ("reducer concat_with's argument `sep` expects Text; found Integer").
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_concat_with_named_arg_rejected` — `reduce(xs, concat_with(sep => ' OR '))` emits `ReducerNamedArgument`.
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_concat_with_runtime_arg_rejected` — `reduce(xs, concat_with(UPPER('|')))` emits `ReducerArgNotCompileTime` (a runtime `Expr<Text>` cannot supply a compile-time separator).
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_concat_with_config_var_arg_accepted` — `reduce(xs, concat_with(smelt.config.var('sep')))` accepts the config-var-resolved compile-time `Text`.
- `crates/smelt-db/src/type_inference/hof.rs::tests::reducer_call_only_at_reduce_second_arg` — `concat_with(' OR ')` at any non-`reduce`-second-arg position emits `UnknownIdentifier`.
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_basic_boolean_cond` — `if TRUE then 1 else 2` synthesises `Integer`.
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_lub_branches` — `if cond then 1 else 1.5` synthesises `Number` (or the LUB per `types.md` numeric promotion).
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_non_boolean_cond` — `if 42 then a else b` emits `TernaryConditionNotBoolean`.
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_branch_type_mismatch` — `if cond then 1 else 'hello'` emits `TernaryBranchTypeMismatch` (Integer vs Text do not unify).
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_keyword_shadowed_smelt_define` — a `smelt.define if(x: Boolean) -> Boolean = x` emits `TernaryKeywordShadowed` at the `if` token of the declaration.
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_short_circuit_suppresses_unreached_evaluation_diagnostic` — `if FALSE then m.get('missing') else 'default'` does NOT emit `MapGetMissingKey` (the `m.get` is in the unreached branch).
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_both_branches_typecheck_even_when_one_unreached` — `if FALSE then (1 + 'oops') else 'default'` DOES emit a type-mismatch diagnostic on the unreached branch (type-checking happens regardless of evaluation).
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_unknown_cond_propagates` — `if smelt.config.var('missing_var') == 'on' then a else b` with a non-existent `missing_var` propagates `Unknown` through the ternary's result.
- `crates/smelt-db/src/type_inference/ternary.rs::tests::ternary_nested_right_associative` — `if c1 then a else if c2 then b else c` checks under the right-associative parse; result type is the LUB of `a`, `b`, `c`.

**Implementation shape.**

- `crates/smelt-types/src/signatures.rs` — widen `SmeltType::Lambda(Box<SmeltType>, Box<SmeltType>)` to `SmeltType::Lambda(Vec<SmeltType>, Box<SmeltType>)`. Update every match arm across the crate (mechanical fan-out — count call sites first). Update `Display`, `assignable_from`, `unifies_with`, `fmt`, `kind_label`. Update the parser-facing `parse_type_annotation` to admit the new shape (if it admits `Lambda<…>` at all — likely not, since `Lambda<…>` is not user-writable).
- `crates/smelt-db/src/type_inference/hof.rs` — update `infer_lambda_in_hof_position` to:
  - Detect arity from the lambda's parameter-list children count; reject 0 with `LambdaZeroParameters`; reject duplicate parameter names with `LambdaDuplicateParameter`.
  - Compare arity against the HOF's required arity (1 for `map`/`filter`, undefined for `reduce` since it takes a reducer not a lambda); mismatch emits `LambdaArityMismatch`.
  - Bind each parameter to its declared type (single type repeated for arity 1 from HOF's `T`; placeholder for future multi-list HOFs).
- `crates/smelt-db/src/type_inference/hof.rs` — add `PARAMETERISED_REDUCER_REGISTRY: &[ParameterisedReducer]` with one entry: `concat_with` with parameter list `[("sep", SmeltType::Text)]`, input element type `Expr<Text>`, output `Expr<Text>`, identity `Some(empty_string_literal)`.
- `crates/smelt-db/src/type_inference/hof.rs` — extend `infer_reducer_in_reduce_position` to recognise a `REDUCER_CALL` CST node:
  - Resolve the identifier against `PARAMETERISED_REDUCER_REGISTRY`; unknown name → `UnknownIdentifier` (same as bare reducers).
  - Validate arity: positional-argument count must match the registry entry's parameter count; mismatch → `ReducerArityMismatch`.
  - Reject named arguments → `ReducerNamedArgument`.
  - For each argument: synthesise the type; ensure it's compile-time-resolvable (literal, `smelt.config.var` result, statically-known field projection); non-compile-time → `ReducerArgNotCompileTime`; type-mismatch with declared parameter type → `ReducerArgTypeMismatch`.
  - Return a `ReducerWitness::Parameterised { name, args }` that the surrounding `reduce` call uses for input-list type-checking and (in expansion) for rendering the binary operation with the resolved separator.
- `crates/smelt-db/src/type_inference/ternary.rs` (new file) — pure function `infer_ternary_type(ternary_ast: &TernaryExpr, ctx: &TypeContext) -> InferResult<SmeltType>`:
  - Resolve `if`/`then`/`else` keyword tokens (no special check here — the parser produces the `TERNARY_EXPR` node only when the keywords are in correct shape; `TernaryDanglingThen` / `TernaryDanglingElse` are flagged by the parser, surfaced by Phase 3's `file_diagnostics`).
  - Type-check the condition; if not assignable to `Boolean`, emit `TernaryConditionNotBoolean` and treat the cond as `Unknown`.
  - Type-check both branches under the surrounding target type.
  - Compute the LUB; if the branches do not unify, emit `TernaryBranchTypeMismatch` at the `else` keyword and return `Unknown`.
  - If the condition's resolved value is `Unknown`, return `Unknown` (both branches still type-checked but neither evaluated for the purposes of value resolution).
  - Apply short-circuit semantics in any evaluation pass (the surrounding HOF / record / generator evaluator inspects which branch to actually evaluate); the inference pass itself is straight LUB.
- `crates/smelt-db/src/type_inference/dispatch.rs` — route `TERNARY_EXPR` CST nodes to `infer_ternary_type`. Route `REDUCER_CALL` CST nodes (only at `reduce`'s second-argument position) to the parameterised-reducer path.
- `crates/smelt-db/src/type_inference/mod.rs` — keyword-shadowing check: when a `smelt.define`, `smelt.record`, or lambda parameter is declared with name `if`, `then`, or `else`, emit `TernaryKeywordShadowed` at the offending name token. (Same shape as `HofNameShadowed` / `ReducerNameShadowed`.)
- `crates/smelt-db/src/diagnostics_types.rs::DiagnosticCode` — add the 12 new variants: `LambdaArityMismatch`, `LambdaZeroParameters`, `LambdaDuplicateParameter`, `ReducerArityMismatch`, `ReducerArgTypeMismatch`, `ReducerArgNotCompileTime`, `ReducerNamedArgument`, `TernaryConditionNotBoolean`, `TernaryBranchTypeMismatch`, `TernaryKeywordShadowed`, `TernaryInDataPosition`, `TernaryDanglingThen`, `TernaryDanglingElse`. Remove `LambdaArityNotSupported` (replaced by `LambdaArityMismatch`). Add the `Display` impl per the message shapes listed in §Surface.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs`
- `crates/smelt-db/src/type_inference/{mod,dispatch,hof,ternary}.rs`
- `crates/smelt-db/src/diagnostics_types.rs`
- `crates/smelt-db/src/type_inference/tests.rs` and any per-file `tests::` modules.

**Docs touched.** None in this phase (the spec was authored in Phase 0). The `smelt-types` and `smelt-db` doc-comments on changed signatures may need brief updates — those are not user-facing.

**Review checklist** (material findings only):
- [ ] All TDD tests listed above exist and pass.
- [ ] `SmeltType::Lambda` widening preserves every existing single-arg lambda equality / unification / display behaviour.
- [ ] `PARAMETERISED_REDUCER_REGISTRY` is the entire v1 parameterised set (exactly `concat_with`); adding entries is a one-line registry edit (closed-registry invariant).
- [ ] Short-circuit ternary semantics: unreached-branch *evaluation* diagnostics suppressed; unreached-branch *type-checking* diagnostics still emitted.
- [ ] `LambdaArityNotSupported` is fully removed (no orphan references in code or tests).
- [ ] Pure-function rule preserved: no Salsa imports in `type_inference/`. `infer_ternary_type` and `infer_parameterised_reducer_call` take AST + `TypeContext` parameters, never call `db.<query>()`.
- [ ] No new exhaustive-match panics on `SmeltType::Lambda` (every match arm covers `Vec<SmeltType>` cleanly).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test -p smelt-db` all green.

**Commit.** `feat(meta-language-F): type inference for multi-arg lambdas, parameterised reducers, ternary`

---

### Phase 3: Diagnostic wiring + LSP

**Goal.** Wire the new CST nodes into `file_diagnostics` so the new diagnostic codes surface to the LSP. Add LSP hover / completion / goto-def for the Phase F surface elements.

**Pre-conditions.** Phases 1–2 committed. Parser produces `TERNARY_EXPR`, `REDUCER_CALL`, multi-arg `LAMBDA` nodes; type inference returns the right diagnostics for them. `file_diagnostics` orchestrates Salsa queries that gather inputs for the pure check functions; today it walks the existing meta nodes (lists, lambdas, pipes, reducers, records, maps, ModelDef). The LSP `Backend::hover` / `Backend::completion` / `Backend::goto_definition` dispatch on cursor position and the surrounding AST.

**TDD tests to write first.**

- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_ternary_condition_not_boolean` — opening a file containing `if 42 then a else b` produces one diagnostic with code `TernaryConditionNotBoolean` anchored at the `42` span.
- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_ternary_branch_type_mismatch` — opening a file containing `if cond then 1 else 'x'` produces one diagnostic with code `TernaryBranchTypeMismatch` anchored at the `else` keyword's range.
- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_ternary_dangling_then` — opening a file containing a stray `then x` outside any `if` produces one diagnostic with code `TernaryDanglingThen`.
- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_ternary_dangling_else` — `if c then x` followed by no `else` produces one diagnostic with code `TernaryDanglingElse`.
- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_reducer_call_diagnostics` — opening a file containing `reduce(xs, concat_with())` produces one `ReducerArityMismatch` diagnostic.
- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_lambda_arity_mismatch` — opening a file containing `map(xs, fn (a, b) => a + b)` produces one `LambdaArityMismatch` diagnostic at the lambda's parameter list.
- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_lambda_duplicate_parameter` — opening a file containing `map(xs, fn (a, a) => a)` produces one `LambdaDuplicateParameter` diagnostic at the second `a`.
- `crates/smelt-db/src/queries/check_types.rs::tests::file_diagnostics_emits_ternary_keyword_shadowed` — opening a file containing `smelt.define if(x: Boolean) -> Boolean = x` produces one `TernaryKeywordShadowed` diagnostic at the `if` token of the declaration.
- `crates/smelt-lsp/src/hover.rs::tests::hover_on_if_keyword_shows_ternary_type` — cursor on the `if` keyword of `if cond then 1 else 1.5` returns hover text containing the full inferred type `if cond:Boolean then Integer else Decimal -> Number` (or equivalent format).
- `crates/smelt-lsp/src/hover.rs::tests::hover_on_then_keyword_shows_then_branch_type` — cursor on the `then` keyword returns the then-branch's synthesised type.
- `crates/smelt-lsp/src/hover.rs::tests::hover_on_else_keyword_shows_else_branch_type` — cursor on the `else` keyword returns the else-branch's synthesised type.
- `crates/smelt-lsp/src/hover.rs::tests::hover_on_multi_arg_lambda_open_paren_shows_signature` — cursor on the `(` of `fn (a, b) => …` returns the full `Lambda<(T_1, T_2), U>` signature.
- `crates/smelt-lsp/src/hover.rs::tests::hover_on_lambda_param_in_multi_arg_shows_bound_type` — cursor on the `b` parameter of `fn (a, b) => …` (in a HOF context that supplies arity-2 lambda types — placeholder; arity-1 HOFs in v1 do not produce this, so test with a stub HOF for the helper layer).
- `crates/smelt-lsp/src/hover.rs::tests::hover_on_concat_with_call_shows_parameter_signature` — cursor on `concat_with` in `reduce(xs, concat_with(' OR '))` returns hover text containing `concat_with(sep: Text)` plus the resolved `sep` value `' OR '`.
- `crates/smelt-lsp/src/completion.rs::tests::completion_at_reduce_second_arg_offers_concat_with_snippet` — at `reduce(xs, <cursor>)` with `xs: List<Expr<Text>>`, completion offers `concat_with($sep)` (or equivalent snippet form) alongside the bare reducers.
- `crates/smelt-lsp/src/completion.rs::tests::completion_at_meta_expression_position_offers_if_snippet` — at the start of a meta-evaluable position (e.g. inside a lambda body), completion offers `if` as a snippet expanding to `if $cond then $then_expr else $else_expr`.
- `crates/smelt-lsp/src/goto_definition.rs::tests::goto_def_on_if_keyword_returns_reference_page_url_hint` — cursor on `if` returns a graceful URL hint (or `None` if the client doesn't support it; matches existing HOF goto-def behaviour).

**Implementation shape.**

- `crates/smelt-db/src/queries/check_types.rs::check_file_diagnostics` — walk `TERNARY_EXPR` and `REDUCER_CALL` nodes; call the pure inference functions from Phase 2; append resulting diagnostics to the file's diagnostic list. Walk `LAMBDA` nodes' parameter lists to detect zero-param / duplicate-param / arity-mismatch at the surrounding HOF dispatch.
- `crates/smelt-db/src/queries/check_types.rs` — keyword-shadowing check: scan workspace-level `smelt.define` / `smelt.record` / lambda parameter declarations for `if` / `then` / `else` names; emit `TernaryKeywordShadowed` at the declaration token.
- `crates/smelt-lsp/src/hover.rs` — pure helpers `hover_text_for_ternary_keyword`, `hover_text_for_multi_arg_lambda_signature`, `hover_text_for_parameterised_reducer_call`. Backend dispatch in `Backend::hover` routes the cursor's enclosing CST node to the correct helper.
- `crates/smelt-lsp/src/completion.rs` — pure helpers `completion_items_for_reduce_second_arg` (extended to include parameterised entries as snippets) and `completion_items_for_meta_expression_position` (extended to include `if` snippet). Backend dispatch in `Backend::completion` routes the cursor position to the correct helper.
- `crates/smelt-lsp/src/goto_definition.rs` — pure helpers `goto_def_for_ternary_keyword`, `goto_def_for_parameterised_reducer_name`. URL-hint shape; graceful no-op when the client lacks support (matches existing HOF / reducer behaviour).
- `crates/smelt-lsp/src/backend.rs` — `Backend::hover`, `Backend::completion`, `Backend::goto_definition` dispatch additions. Keep the production-wiring discipline matching Phases E1/E2 (pure helpers with unit tests at this layer; Backend integration covered by LSP smoke tests in the relevant integration suite).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/check_types.rs`
- `crates/smelt-lsp/src/{hover,completion,goto_definition,backend}.rs`
- The corresponding `tests` modules.

**Docs touched.** None in this phase. The spec already lists the LSP obligations under each construct.

**Review checklist** (material findings only):
- [ ] All TDD tests listed above exist and pass.
- [ ] Every new diagnostic code is reached by at least one `file_diagnostics`-level test.
- [ ] Hover / completion / goto-def helpers are pure (no Salsa imports) and Backend dispatch is the thin wrapper.
- [ ] No regression in existing hover / completion / goto-def for single-arg lambdas, bare reducers, or non-ternary expressions.
- [ ] `Backend::completion` returns deduplicated entries (no duplicate snippet for `if` when the position offers it as part of a wider completion set).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` all green.

**Commit.** `feat(meta-language-F): file_diagnostics + LSP for polish surfaces`

---

### Phase 4: Example fixture — `examples/meta_polish/`

**Goal.** Real-fixture model exercising all three polish surfaces (`concat_with(sep)`, multi-arg lambda candidate, ternary) end-to-end. Passes `cargo test -p smelt-cli --test example_diagnostics` with zero diagnostics.

**Pre-conditions.** Phases 1–3 committed; the polish surfaces are fully working at the inference + LSP layer. The example must use surfaces available in v1 — no `zip_with`, no user-defined reducers, no `Optional<V>`.

**TDD tests to write first.**

- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_polish_passes_diagnostics_gate` — opening `examples/meta_polish/` workspace produces zero LSP diagnostics on every model file.
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_polish_concat_with_renders_separator_in_codegen` — building `examples/meta_polish/<model that uses concat_with>` produces SQL containing the expected separator-joined fragment (e.g. `tag IN ('a' OR 'b' OR 'c')` if `concat_with(' OR ')` is the chosen test, **or** `WHERE a = 1 OR a = 2` style if the example uses `or_any` for the cleaner shape and `concat_with` is exercised elsewhere on a `Text` chain).
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_polish_ternary_short_circuit_visible_in_codegen` — building a model with `if smelt.config.var('env') == 'prod' then strict_filter else permissive_filter` produces SQL containing only the chosen branch's filter (compile-time-resolved per the workspace's `smelt.yml` `vars:`).
- `crates/smelt-cli/tests/example_diagnostics.rs::tests::meta_polish_with_broken_subfixture_emits_expected_diagnostics` — opening `examples/meta_polish/broken/` (a sub-fixture with a deliberately-broken polish surface — e.g. `if 42 then a else b`) produces diagnostics matching the expected codes (`TernaryConditionNotBoolean`, etc.).

**Implementation shape.**

- `examples/meta_polish/smelt.yml` — workspace config with at least one `vars:` entry the ternary example consumes (`env: 'prod'` or similar) and DuckDB backend.
- `examples/meta_polish/models/concat_with_separator.sql` — model demonstrating `concat_with(sep)`: a list of text fragments reduced with `concat_with(' OR ')` to produce a composite predicate or a comma-style join. Keep the model under ~30 lines.
- `examples/meta_polish/models/ternary_env_branch.sql` — model demonstrating ternary: `WHERE if smelt.config.var('env') == 'prod' then strict_predicate else permissive_predicate`. Uses `smelt.config.var` to resolve at compile time; the SQL output depends on the workspace `vars:`.
- `examples/meta_polish/models/multi_arg_lambda_placeholder.sql` — **deferred behaviour**: in v1 no shipped HOF takes a multi-arg lambda (`map`/`filter` are arity-1, `reduce` takes a reducer). The example demonstrates that the multi-arg surface parses, type-checks, and emits the right diagnostic when used at an arity-1 position (a positive test for `LambdaArityMismatch` in a real workspace, not the broken sub-fixture). If forcing a multi-arg lambda for a passing test requires `zip_with`, defer to Deferred During Implementation; the broken-sub-fixture path is sufficient.
- `examples/meta_polish/broken/models/ternary_non_boolean_cond.sql` — `if 42 then 1 else 2` (broken sub-fixture).
- `examples/meta_polish/broken/models/ternary_branch_mismatch.sql` — `if cond then 1 else 'oops'` (broken sub-fixture).
- `examples/meta_polish/broken/models/reducer_arity.sql` — `reduce(xs, concat_with())` (broken sub-fixture).
- `examples/meta_polish/broken/.smelt-broken` — marker file telling `example_diagnostics` to expect diagnostics rather than zero.
- `examples/meta_polish/README.md` — brief description of what the fixture exercises and which surface elements appear in which models.

**Critical files (allowed to touch in this phase).**
- `examples/meta_polish/**`
- `crates/smelt-cli/tests/example_diagnostics.rs` — only to register the new fixture path if the test harness requires explicit registration (otherwise the fixture is auto-discovered).

**Docs touched.** Phase 5 lands the user docs that *reference* this example.

**Review checklist** (material findings only):
- [ ] All TDD tests listed above exist and pass.
- [ ] Every model in `examples/meta_polish/` (excluding `broken/`) passes `example_diagnostics` with zero diagnostics.
- [ ] Every model in `examples/meta_polish/broken/` produces the diagnostics enumerated in its filename / README.
- [ ] At least one Phase F surface (parameterised reducer, multi-arg lambda surface, ternary) appears in each non-broken model.
- [ ] The fixture is minimal — no contrived data, no unused tables, no models that don't exercise Phase F surfaces.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` green.

**Commit.** `feat(meta-language-F): examples/meta_polish/ fixture`

---

### Phase 5: User docs

**Goal.** Author `docs-site/docs/meta-language/ternary.md` (new) and update `lambdas.md`, `reducers.md`, `reference.md` to cover the polish surfaces. Promote the Known Divergences polish bullet in `meta_language.md` from "specified but pending" to "shipped".

**Pre-conditions.** Phases 1–4 committed; the surfaces are working and the example fixture exists for the docs to link to.

**TDD tests to write first.** Documentation has no TDD tests in the unit-test sense. The docs-site builds via `mkdocs`; a build failure (missing link, broken anchor) is the verification gate. Add:

- `docs-site/.mkdocs-links-check` (or equivalent) — verify the new `ternary.md` is reachable from `index.md` and `reference.md`.
- Manual verification: every code snippet in the new docs paste-compiles into `examples/meta_polish/` or into a fresh test workspace.

**Implementation shape.**

- `docs-site/docs/meta-language/ternary.md` (new) — concept overview, syntax (`if cond then a else b`), evaluation rules (compile-time, short-circuit), the `m.has(k) |> if then m.get(k) else default` defaulting pattern, the right-associative chaining note, type rule (LUB), worked example linking to `examples/meta_polish/models/ternary_env_branch.sql`. Keep under ~150 lines.
- `docs-site/docs/meta-language/lambdas.md` — extend the existing single-arg lambda content with a "Multiple parameters" subsection covering `fn (a, b) => body` syntax, the arity-checking rule, the duplicate-parameter rule, the closed list of currently-arity-1 HOFs and the deferred multi-list HOF (`zip_with`).
- `docs-site/docs/meta-language/reducers.md` — extend with a "Parameterised reducers" subsection covering `concat_with(sep)`, the compile-time argument rule, the call shape at `reduce`'s second argument, the identity-on-empty-list rule.
- `docs-site/docs/meta-language/reference.md` — alphabetical reference additions: `concat_with(sep: Text) -> Reducer<Text, Text>` entry; `if-then-else` entry (with type signature in pseudo-syntax: `(Boolean, T, T) -> T`); update the `Lambda<…>` entry to reflect the multi-arg parameter-vector shape.
- `docs-site/docs/meta-language/index.md` — add a "Meta-world ternary" navigation entry.
- `docs/specs/meta_language.md` — Known Divergences polish bullet: replace "specified but pending" with a recap of what shipped (the 14 diagnostic codes, the multi-arg lambda surface, `concat_with(sep)`, the ternary). `zip_with` remains deferred per the theoretical-completeness ledger.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/meta-language/{ternary,lambdas,reducers,reference,index}.md`
- `docs/specs/meta_language.md` — Known Divergences polish bullet only (no Surface / Semantics / Design / Constraints changes; the spec is settled from Phase 0).

**Docs touched.** All of the above are the docs themselves.

**Review checklist** (material findings only):
- [ ] Every new section reads as if the feature has always existed — no `Phase F` headings, no `(Phase F)` labels, no plan-vocabulary callouts.
- [ ] Every code snippet either (a) matches a model in `examples/meta_polish/`, or (b) compiles standalone in a `smelt-cli` doctest.
- [ ] `reference.md` entries are alphabetical and cover the full Phase F surface (`concat_with`, `if-then-else`, multi-arg `Lambda<…>` form).
- [ ] The Known Divergences polish bullet in `meta_language.md` is rewritten in past tense (shipped) rather than future tense.
- [ ] Docs-site builds cleanly: `cd docs-site && mkdocs build --strict` (or equivalent CI command) produces no broken links or anchors.
- [ ] No new `docs/specs/` changes beyond the Known Divergences rewrite.

**Commit.** `docs(meta-language-F): user docs for polish surfaces (ternary, multi-arg lambdas, parameterised reducers)`

---

### Phase 6: `smelt-app-builder` skill update

**Goal.** Add a dated reference doc under the `smelt-app-builder` skill documenting workflow gotchas for the Phase F surfaces. Skill body stays short — point at the user docs for syntax, capture only workflow-level discipline.

**Pre-conditions.** Phases 1–5 committed; user docs are published and the example fixture is the canonical citation target.

**TDD tests to write first.** The skill is unstructured markdown; verification is by `smelt-loop` runs (Phase G) rather than per-skill tests. Add a smoke step:

- Confirm `.claude/skills/smelt-app-builder/SKILL.md` references the new reference doc.
- Confirm the new reference doc renders under the smelt-app-builder skill's `references/` directory.

**Implementation shape.**

- `.claude/skills/smelt-app-builder/references/20260516-meta-polish.md` (new) — short reference doc (~50 lines) covering:
  - `if`/`then`/`else` are reserved meta-namespace keywords; `smelt.define if(...)` is a diagnostic, not silently shadowing.
  - Parameterised reducer arguments must be compile-time-resolvable; `concat_with(UPPER('|'))` is a diagnostic.
  - Multi-arg lambda parens are mandatory: `fn (a, b) => body`, not `fn a, b => body`.
  - The `m.has(k) |> if then m.get(k) else default` defaulting pattern.
  - Common authoring mistakes and their diagnostic codes.
- `.claude/skills/smelt-app-builder/SKILL.md` — one-line addition under the references section pointing at the new file.

**Critical files (allowed to touch in this phase).**
- `.claude/skills/smelt-app-builder/SKILL.md`
- `.claude/skills/smelt-app-builder/references/20260516-meta-polish.md`

**Docs touched.** The skill is its own docs; no further changes here.

**Review checklist** (material findings only):
- [ ] The reference doc is short (workflow gotchas only — no syntax tutorials; point at `docs-site/docs/meta-language/`).
- [ ] The SKILL.md update is one line.
- [ ] The reference doc reads as if the Phase F surface has always been part of the language — no `Phase F` callouts.
- [ ] All cargo checks remain green (no code changes in this phase).

**Commit.** `skill(meta-language-F): smelt-app-builder workflow gotchas for polish surfaces`

---

### Phase 7: Expert reviewer dispatch loop

**Goal.** Run each Phase F applicable expert reviewer from meta-plan §5 over the Phase F diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below.

**Pre-conditions.** Phases 0–6 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase F subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **type-expert** | sonnet | `crates/smelt-types/src/signatures.rs` + `crates/smelt-db/src/type_inference/` (especially `hof.rs`, `ternary.rs`, `dispatch.rs`, `mod.rs`). | Type inference purity preserved (no Salsa imports inside analysis logic); `SmeltType::Lambda` widening to `Vec<SmeltType>` is non-breaking (no exhaustive-match panics, every match arm covers the vector cleanly); bidirectional checking still terminates for ternary branches (LUB rule produces a fixed point); `PARAMETERISED_REDUCER_REGISTRY` follows the closed-registry discipline (no dynamic dispatch, no user-extension surface). |
| **lsp-expert** | sonnet | `crates/smelt-lsp/src/{hover,completion,goto_definition,backend}.rs` + any new helper module. | Hover types correct for the polish surfaces (multi-arg lambda parameter-list `(`, `if`/`then`/`else` keywords, parameterised reducer call sites); completion at `reduce(xs, <cursor>)` offers `concat_with($sep)` snippet; `if` snippet appears at meta-expression positions; goto-def on the keywords resolves to URL hint gracefully; no regression in existing LSP paths. |
| **examples-curator** | haiku | `examples/meta_polish/`. | Minimal-but-realistic; not contrived; passes `example_diagnostics` with zero non-broken diagnostics and the expected broken-sub-fixture diagnostics; every model exercises at least one Phase F surface; the README correctly describes which surface each model exercises. |
| **docs-reviewer** | haiku | `docs-site/docs/meta-language/{ternary,lambdas,reducers,reference,index}.md` deltas; `docs/specs/meta_language.md` Known Divergences polish bullet. | User docs match the spec's §Surface exactly (no syntax in docs that isn't speced); reference page is alphabetical and complete (every shipped Phase F construct has an entry with type signature + small example); the Known Divergences polish bullet reads as shipped (past tense) and references this plan and the `zip_with` deferral correctly. |

**Loop discipline.**

1. **Round 1.** Dispatch all four experts in parallel — single message, four Agent tool calls. Each prompt MUST include:
   - The phase plan path (`docs/plans/20260509-meta-language-F.md`) and the spec sections that are the oracle (§"Lambdas and HOFs", §"Contextual reducers", §"Meta-world ternary").
   - The exact file scope from the table above.
   - The diff range to review (`git log --oneline <phase-0-commit>..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: numbered list of findings with file:line refs, or "no material findings".
2. **Address findings.** For each expert that returns material findings:
   - Mechanical fix (≤~30 lines, single concern) → edit directly.
   - Non-trivial fix → dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into other phases.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-F): address {expert-name} feedback`. Push immediately.
3. **Re-dispatch** only the expert whose findings were addressed, providing the round-1 prompt plus a diff of what changed. "No material findings" → that expert is **clean** and exits the loop.
4. **Repeat** step 2 → step 3 until every expert is clean.
5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of:
   - Same expert flags a material finding on round 3 (per-expert bound).
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7).
   - An expert's findings would force a non-trivial spec change — pause for the user.
   - A fix surfaces a pre-existing failure unrelated to this phase.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus this plan file (to record round counts).

**Review checklist** (material findings only — applied to the expert-dispatch *process*):

- [ ] All four applicable experts were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded under "Deferred during implementation".
- [ ] No fix touched files outside the dispatching expert's scope.
- [ ] No expert ran more than 3 rounds; if any did, autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] All cargo checks green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation":

> Phase 7 expert review: type-expert clean (R{n}), lsp-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-F): address {expert-name} feedback`. If round 1 came back clean, no commit for that expert. The acceptance-gate summary lands in the next commit (or in `chore(meta-language-F): record Phase 7 review summary` if no other phase-7 commits were made).

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

**Phase 4 — codegen acceptance tests skipped.** The plan listed two codegen tests
(`meta_polish_concat_with_renders_separator_in_codegen` and
`meta_polish_ternary_short_circuit_visible_in_codegen`) that would require `smelt build`
+ DuckDB execution, analogous to `crates/smelt-cli/tests/cohort_count_acceptance.rs`.
These were deferred because the diagnostic gates already prove the surface compiles
cleanly at the LSP / type-inference layer, and adding build + execution tests requires
DuckDB round-tripping that goes beyond the scope of verifying type correctness. If
codegen fidelity of the separator string or short-circuit branch selection becomes a
correctness concern, a future plan should add a `concat_with` acceptance test alongside
the existing DuckDB backend tests.

**Phase 4 — multi-arg lambda clean-workspace model deferred.** The plan called for a
`models/multi_arg_lambda_placeholder.sql` that demonstrates the multi-arg surface parsing
and type-checking at an arity-1 call site (positive test for `LambdaArityMismatch`). In
practice, exercising this at the LSP layer in a clean workspace produces a diagnostic
(the mismatch IS the point), so it cannot live in the zero-diagnostic clean fixture.
The broken-sibling approach is the right home for a `LambdaArityMismatch` example; a
dedicated `meta_polish_broken_lambda_arity_mismatch/` workspace was not added to keep
Phase 4 scope tight — `meta_hofs_broken_lambda_arity_not_supported/` already covers
that diagnostic end-to-end (tests are green). If the examples-curator wants a fixture
that is named after the polish workspace family, add it in Phase 5 or a follow-on.

---

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets`
- `cargo test --quiet 2>&1 | tail -40`
- `cargo test -p smelt-cli --test example_diagnostics` — `examples/meta_polish/` and every earlier-phase example continue to pass.
- `cargo build` produces no warnings from the new Phase F code paths.
- `/smelt:validate meta_language` reports zero drift.
- The overall plan file `docs/plans/20260509-meta-language-overall.md` row for Phase F flips from `pending` to `done` with the commit SHA of the Phase 7 acceptance-gate commit and today's date.
