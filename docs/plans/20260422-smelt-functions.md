# Smelt Functions Implementation Plan

**Date:** 2026-04-22 (Steps 1 & 2) / extended 2026-04-23 (Steps 3–8)
**Research:** [`docs/research/20260413-smelt-functions.md`](../research/20260413-smelt-functions.md) (full paper, but especially §3, §6, §7, §8, §9, §10, §11, §12, §13, §16, §19, §21)
**Tracking PR:** #108 (branch `worktree-review`)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion autonomously.

**Before touching any code:**

1. Read this entire plan file. Then read the research doc `docs/research/20260413-smelt-functions.md` — at minimum §2, §3, §8, §13, §16 (all 24 decisions), §19, and §21; Steps 3+ additionally depend on §6, §7, §9, §10, §11, §12. The plan assumes those decisions are settled — do not re-open them.
2. Confirm you are on branch `worktree-review` (PR #108): `git rev-parse --abbrev-ref HEAD`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the "Progress tracking" table below. That is your starting point. If every phase is `done`, run the post-Phase-38 verification under "Verification" and stop.

**For each phase, loop:**

1. **Implement.** Spawn a fresh `general-purpose` subagent with a self-contained brief built from the phase's own section: goal, pre-conditions, TDD tests to write first (list them verbatim), implementation shape, and the files under "Critical files" that it is allowed to touch. The implementer writes failing tests first, makes them pass, and must leave the tree passing: `cargo fmt --all -- --check`, `cargo clippy --all-targets` with zero warnings, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics`.
2. **Review.** Spawn a fresh `general-purpose` subagent as reviewer. Hand it the phase's review checklist and the `git diff` since the last phase's commit (or since the plan commit for Phase 1). Ask it to report only material issues — ignore trivial style nits. Material = correctness, architectural invariants (pure functions, Salsa purity rule), test coverage gaps against the listed TDD tests, or scope creep beyond the phase's stated goal.
3. **Iterate.** If the reviewer reports material findings, dispatch the implementer again with those findings. Repeat until the reviewer comes back clean. Do not move on with open material findings.
4. **Record + commit.** Update the "Progress tracking" row for this phase: status → `done`, fill the commit sha once known, date → today (`date -I`). If the phase surfaced anything worth deferring, append it to "Deferred during implementation". Then commit (including the plan-file updates) with the exact commit message under the phase's "Commit." line, and push to `worktree-review`.
5. **Advance.** Proceed to the next `pending` phase immediately. Do not stop between phases.

**When to pause and ask the user:**

- A reviewer keeps surfacing the same material finding across two implementer passes.
- A phase's TDD tests cannot be made green without violating a cross-phase design choice in this file.
- A fundamental assumption in the research or the plan turns out to be wrong (e.g., a decision in §16 is found to be self-contradictory in practice).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure on `worktree-review` that is not caused by your changes — diagnose before continuing.

**Conventions that apply every phase:**

- Never skip hooks, never `--no-verify`, never force-push, never rebase the tracking PR.
- Never amend a prior phase's commit. If a later phase needs to revise earlier work, add a new commit and note the revision under "Deferred during implementation".
- Keep `type_inference.rs` pure (no Salsa DB access) — architectural invariant in `CLAUDE.md`.
- Don't widen scope: the Step N phase you are on must not reach into Step N+1's scope. TableExpr (Step 3), context bindings (Step 4), Tier 2/3 (Step 5), PASSING (Step 6), planner visibility (Step 7), struct row vars (Step 8) each have their own phases — if you think your phase "needs" one of them to pass its tests, re-read the phase — the tests are scoped deliberately.
- Commit messages are the phase's `Commit.` line verbatim; include the standard Claude Code co-author trailer.

You may now begin. Start by reading the files listed above, then proceed with the first `pending` phase.

---

## Context

The smelt-functions research has converged — §16 records 24 settled decisions and §21's pre-implementation checklist is complete. Step 1 (`smelt.define` for `Expr<T>` functions with Tier 1 checking, `safe_divide` end-to-end) and Step 2 (canonical built-in signature registry, generics, variadics, `smelt.extern`, `backends:` frontmatter) are the first two rungs of the experimentation roadmap. Together they establish the fragment-sort expansion model, the canonical signature vocabulary, and the Tier 1 error-tracing infrastructure every later step reuses.

This plan decomposes Steps 1 & 2 into twelve phases executed via red-green TDD. Each phase ships independently, commits atomically, and is reviewed by a subagent before the next begins.

## Scope

### In scope (roadmap coverage)

- **Steps 1 & 2** (Phases 1–12, **complete**) — `smelt.define` / `smelt.fn.*` for `Expr<T>` functions with Tier 1 checking, canonical signature registry with generics + variadics (§16 #13/#14/#15), `smelt.extern` (§16 #21), per-declaration frontmatter (§16 #22), `backends:` inference and backend namespace sugar (§16 #23), multi-level frame rendering (§16 #16), CAST-enforcement flag on canonical returns (§16 #9).
- **Step 3** (Phases 13–18) — `TableExpr` sort parameter, `WindowExpr<T>` sort, `SelectItems<K, ctx>` kind ceiling (§16 #24), parameters-first bare-column resolution over `TableExpr` schemas (§16 #7), shadow warnings (§16 #1), `TableExpr<{…}>` row-requirement annotations, `add_margin` and `sessionize` end-to-end.
- **Step 4** (Phases 19–22) — Context binding syntax (`Expr<T, ctx>`, `SelectItems<K, ctx>`), CTE schema extraction, context inference from splice points and multi-splice intersection (§16 #5), explicit-annotation validation, `session_rollup` end-to-end.
- **Step 5** (Phases 23–27) — Tier 2 (parameters annotated) body check in isolation, Tier 3 (return annotated) LSP hover, call-site bidirectional pre-expansion checking, Tier 2 → Tier 1 inline expansion (§16 #17), bidirectional interaction with built-in generics (§16 #14).
- **Step 6** (Phases 28–29) — `PASSING name AS (…)` trailing clauses, context-sensitive after `smelt.fn.*` and user-defined call closings (§16 #18); binding PASSING fragments to fragment-sort parameters; basic LSP completion inside clause bodies.
- **Step 7** (Phases 30–34) — Functions as first-class nodes in the logical plan (§12), declared-property propagation, planner-rule API (Level 1 hooks), first transparent-boundary rewrite (filter pushdown), join-elimination example.
- **Step 8** (Phases 35–38) — Row variables on `Struct<…>` types (§11), value-level spread `..event`, row unification at the call site with erasure at expansion, `smelt.as_struct(source EXCEPT …)` revisit (§16 #19).

### Explicitly deferred (out of scope after Phase 38)

- Generics in `smelt.define` (user polymorphic functions) — §16 #14 defers past v1.
- Variadics in `smelt.define` — §16 #15 defers past v1.
- Nullability tracking (§16 #10), Decimal precision/scale arithmetic (§16 #9), Text collation (§16 #13).
- Union contexts (replaced by the no-overlap rule, §16 #4) — not reopened.
- Multi-row struct variables (`merge(a: {..r}, b: {..s}) -> {..r, ..s}`) — §11 keeps the single-variable constraint for v1.
- Disjoint-union row merging in struct return types; defaults on row-polymorphic parameters — §11.
- Higher-kinded constraints (`Array<T>: Ordered where T: Ordered`) — §16 #14 deferred.
- Structured planner properties beyond booleans/lists — §12/§18. Phase 34's join-elimination example ships with hand-declared structured properties under an unstable-schema flag.
- Automatic derivation of provenance / join graphs — §12 v1 requires explicit declarations.
- Cross-backend execution orchestration (Spark writes Parquet, DuckDB reads) — handled by the separate multi-engine plan.
- Expansion caching, span-based diagnostic deduplication, deep-expansion truncation (§16 #12, §16 #16 remaining deferred items).
- Local/nested `smelt.define` (§16 #11).
- Runtime schema validation of `smelt.extern` return types (§20L).
- Function-test workflow as a first-class concept (§18) — functions remain tested through models.
- CAST *emission* in generated SQL (Step 2 only records the flag; codegen integrates with planner lowering in Step 7 Phase 32).

A "Deferred during implementation" section at the end of this file records anything additional that surfaces during the work.

## Execution model

The plan is executed autonomously after approval:

1. **Plan commit (before Phase 1).** This file is committed on its own and pushed to `worktree-review` (PR #108).
2. **Per-phase loop.** For each phase the main session:
   1. Spawns an implementer subagent with the phase's self-contained brief (goal, pre-conditions, tests to write first, implementation shape).
   2. Spawns a reviewer subagent with the phase's review checklist plus the diff produced by the implementer.
   3. Re-dispatches the implementer with any material review findings. Iterates until the reviewer reports zero material issues. Trivial style nits may be skipped.
   4. Once clean: main session updates this plan file (phase status + any new deferrals), commits + pushes to `worktree-review` (PR #108) with the phase commit message.
3. **Course corrections.** If a phase surfaces an unresolvable problem, the main session pauses and asks the user. The user may interrupt at any phase boundary.
4. **CI gates per phase (implementer must ensure all pass before handoff to reviewer):** `cargo fmt --all`, `cargo clippy --all-targets` (zero warnings), `cargo test`, `cargo test -p smelt-cli --test example_diagnostics`.

## Example project strategy

Every phase must exercise its feature in real SQL fixtures, not just unit-test ASTs. Two workspaces back this:

- **`examples/functions_demo/`** (new, created in Phase 1) — the green end-to-end workspace. Each phase extends it with fixtures that demonstrate the feature just landed. The workspace must stay diagnostic-clean under `cargo test -p smelt-cli --test example_diagnostics` — which is the standing CI gate, so regressions fail the build automatically. Layout mirrors `examples/test_workspace/`: `smelt.yml`, `sources.yml`, `models/`, plus a new `functions/` directory holding `smelt.define` and `smelt.extern` files.
- **`examples/broken/models/`** (existing) — negative fixtures. Each phase that introduces a new diagnostic code adds a file here named `fn_<what-is-broken>.sql`. Because `example_diagnostics` skips `broken/`, these fixtures need a companion assertion: a new integration test `crates/smelt-cli/tests/broken_function_diagnostics.rs` (introduced in Phase 6 alongside the first user-facing function diagnostic) asserts the expected `DiagnosticCode` + message substring for each broken fixture. New phases append rows, not new test files.

A phase's review must confirm both (a) `functions_demo` is still green and (b) every broken fixture the phase added is matched by an assertion in `broken_function_diagnostics.rs` (or, for Phases 1–5 which predate it, a dedicated integration test in the corresponding crate).

## Cross-phase design choices

| Decision | Choice | Rationale |
|---|---|---|
| Frame-stack rendering shape | Extend `DiagnosticData` with an `ExpansionFrames(Vec<FrameInfo>)` variant; LSP renders via `DiagnosticRelatedInformation` | Existing consumers ignore unknown variants cleanly; no second accumulator touching the LSP publish path. Step 1 populates 0–1 frames, Step 2 populates full stack — no schema change between them. |
| Signature registry location | `smelt-types` (new `signatures` module sibling to `functions`) | `smelt-types` is dependency-free; registry is pure data. Keeps property tests and unit tests DB-free. `smelt-db` already depends on `smelt-types`. |
| Parser-first sequencing | Parser lands before type-system wiring (Phases 1–2 parser, 3+ type system) | Type-system tests need syntax to feed them; hand-constructed CSTs are brittle. Keep parser Phase 1 strictly grammar-only so type-system phases don't re-touch it. |
| Salsa signature/body split | `function_signature(path, name)` and `function_body(path, name)` as separate queries from day one (§20H) | Body-only edits must not invalidate callers. Introducing this in Phase 3 prevents LSP latency regression once Step 2's fanout lands. |
| Frame stack always populated | Phase 6 builds the full stack; Phase 12 upgrades the renderer only (§16 #16) | Avoids a schema migration mid-plan. "Single-level" is a renderer property, not a data-structure property. |
| TableExpr schema representation (Step 3+) | Reuse `smelt-types::ModelSchema` for both call-site concrete schemas and `TableExpr<{…}>` annotations; annotation is a `SchemaRequirement` with optional row-variable tail | Single schema datum feeds both structural resolution (§7) and annotation checking. Row-variable tail lives on the requirement, not on the runtime schema. |
| WindowExpr sort threading (Step 3+) | Add `ExprKind { Scalar, Agg, Window }` on every `Expr<T>`-typed AST node in Phase 14; subtype check is `kind <= expected_kind` | Makes the check O(1) at every splice point. Kind is populated during type inference, same pass as `DataType`. |
| SelectItems kind ceiling (Step 3+) | Compute `K` as `max(item.kind for item in items)` when the `SelectItems` value is synthesised; no per-item tag in the CST | Matches §16 #24 exactly. Per-item tagging would force the parser to care about kinds, which it must not. |
| Context binding inference (Step 4) | Infer a fragment parameter's context from its splice point(s) before recording the parameter's final type (§16 #5); explicit annotations validate against the inferred context; multi-splice uses intersection | Keeps annotations optional. The inferred context is what the Tier 1 body check actually needs; explicit context is a checked comment. |
| CTE forward reference (Step 4) | Two-pass body analyser inside each `smelt.define`: Pass 1 extracts CTE schemas (DFS with colour-based cycle detection); Pass 2 runs the type checker with full CTE schemas visible. Each pass is pure; Salsa wraps only the outer boundary | Solves §21's CTE forward-reference item without reintroducing Salsa into the pure checker. |
| Check-mode discriminator (Step 5) | Introduce `CheckMode { Tier1Expansion(arg_types), Tier2Isolated, Tier2CallSite(expected_ret) }` | Keeps the pure checker a single function. Tier 1 is already one of these modes after Phase 5; Phase 23 adds the other two. |
| Tier 2 → Tier 1 inline expansion (Step 5) | On reaching a Tier 1 call, switch to `Tier1Expansion(param_types)` with the Tier 2 context's parameter types as concrete arg types (§16 #17); frame stack rooted at Tier 2 body; Phase 12's multi-level renderer handles output unchanged | Reuses existing mechanisms. Signature stability for Tier 2's callers holds because errors are reported against the Tier 2 body. |
| Generics ↔ bidirectional interaction (Step 5) | Phase 27 extends `unify_call` with `expected_return: Option<DataType>`; expected return is another "position" for any `T` in the return (§16 #14) — already anticipated as the Phase 8 "inert hook" | Decision 14's inference rule handles this at spec level; no new algorithm, just plumbing. |
| PASSING context-sensitivity (Step 6) | Parser peeks `PASSING` only immediately after the `)` closing a `smelt.fn.*` or user-defined function call; non-call `PASSING` stays an identifier (§16 #18) | Matches the decision exactly. Keeps the parser independent of the type checker. |
| Planner plan representation (Step 7) | Logical plan is an `Arc<LogicalNode>` tree; `LogicalNode::FunctionCall { fn_id, args, transparent, provenance, properties }` carries the enriched typed interface (§12 Level 1); expansion happens at Level 2 | This is where §19 Step 7 lives. Data-structure-only change in Phase 30; rules fill in through Phase 34. |
| Planner rule API (Step 7) | A `PlannerRule` trait with one `apply(&mut Plan, &Context) -> RuleResult`; fixed-point loop; rules live in new `smelt-planner` crate (create in Phase 30) | Matches `docs/planner_rule_api_design.md`. Fixed-point execution is the usual logical-planner shape. |
| Struct row vars — one named var in v1 (Step 8) | `..r` is the single named row variable per signature (§11); a second named `..s` is a diagnostic; anonymous `..` creates a fresh variable per parameter that cannot be referenced | Exactly §11's constraint. Multi-row unification deferred. |
| `smelt.as_struct()` revisit (Step 8) | Phase 38 lands `smelt.as_struct(<alias> [EXCEPT <col-list>])` in expression position, dispatching on the backend's `struct-literal` capability flag; backends without a struct literal error at planner time | Matches §16 #19. Row-var machinery from Phase 35 backs the erasure. |

## Cross-phase risks

- **`smelt.define` vs. identifier ambiguity.** `smelt` is not a reserved word. `smelt.define` is only special at top-level statement position (§16 #11). Phase 1 must encode this trigger; regressions would break models that happen to reference a column called `define`.
- **Frontmatter model change (§16 #22).** The current `strip_frontmatter` assumes a single file-level block. Phase 11 moves to per-declaration. All prior phases must use the legacy single-block rule unchanged so fixtures don't break mid-plan.
- **`UnrecognizedFunction` collision.** Phases 6 and 9 both touch function lookup. New `DiagnosticCode::UnknownSmeltFn` keeps `smelt.fn.*` misses distinct from plain SQL function misses.
- **Registry coverage gap (Phase 9).** Rewiring `infer_function_type` through the registry must preserve current property-test behaviour. Spike first: confirm every `SqlFunction` variant removed from the legacy match has a registry entry.
- **AggExpr collapse temptation (Step 3).** §18 flags "keep or collapse `AggExpr<T>`." **Keep** — the linear subtyping chain (§16 #8) gives it a clear role once `WindowExpr<T>` lands in Phase 14. Regressions would re-open the question.
- **Row variable scope leaks (Step 8).** Phase 35's row-variable binding must be scoped to the signature; `..r` in one function's signature must not be visible in another's. Use per-function-declaration fresh var IDs.
- **Planner-rule fixed point on transparent functions (Step 7).** Phase 33's first rewrite pushes filters across `LogicalNode::FunctionCall { transparent: true, .. }`. If the rule doesn't terminate (pushing the same filter repeatedly), the planner loop loops. Termination is guaranteed by the `pushed_filter.is_some()` field on the `ExpandedCall` node (not a separate `Context` field as originally planned), tested explicitly. _(Phase 53 audit: stale "marker in `Context`" reference corrected — the implementation uses `pushed_filter.is_some()` on the node itself, `RuleContext` is `#[derive(Default)]` with no fields.)_
- **Tier 2 → Tier 1 expansion caching (Step 5).** Phase 26 may expand the same Tier 1 body many times under one Tier 2 body check. Salsa caches per `(callee_fn_id, arg_types)` — but `arg_types` is not hashable by default. Define a canonical `DataTypeHash` in Phase 26, not retrofitted.
- **`PASSING` vs. future post-call syntax (Step 6).** Phase 28's lookahead after `)` is currently one token. If future syntax ever adds another post-call form (e.g. `.chain_method()`), the lookahead grows. Keep the check centralised so the growth is one edit.
- **Struct literal backend divergence (Step 8).** Phase 36 emits struct literals on DuckDB (`{'f': v}`); Spark (`struct(v AS f)`) and Postgres (row constructor / composite type) differ. The backend printer (Phase 11's infrastructure) is the only place this should vary.

## §21 pre-implementation checklist closure

Steps 1 & 2 cleared every "Must resolve" item plus the "Should resolve" items: `smelt.define` grammar, expansion mechanics, Tier 1 error tracing MVP + full rendering, `Ordered` constraint, generics syntax/inference, variadics, `smelt.extern` full syntax, unified frontmatter, engine-agnostic bodies.

The remaining "Can defer" items close during Steps 3–5 as follows:

- **Function file discovery** — closes in Phase 13 (first phase that requires recursing into subdirectories under `functions/` for `TableExpr`-based fixtures).
- **`AggExpr<T>` — keep or collapse?** — closes in Phase 14. Decision: **keep** as a distinct sort; the linear subtyping chain (§16 #8) gives it a clear role once `WindowExpr<T>` is formalised.
- **CTE forward reference / cycle detection** — closes in Phase 22 (two-pass body analyser inside `smelt.define`; pure function, Salsa wraps only the outer boundary).
- **Tier 1 → Tier 2 upgrade-path breaking changes** — closes in Phase 27 with a migration-story doc (`docs/smelt-functions-upgrade-story.md`) and a call-site diagnostic when a Tier 2 declaration rejects a previously-accepted Tier 1 expansion.

---

## Step 1 — Phases 1 to 6

### Phase 1 — Parser: `smelt.define` top-level grammar

**Goal.** Parse `smelt.define name(params) [-> Type] AS (body)` into a CST; `parse_file` accepts a sequence of top-level items.

**Pre-conditions.** None.

**TDD tests** (all in `crates/smelt-parser/src/parser.rs` under `#[cfg(test)] mod tests`):
1. `parses_minimal_smelt_define` — `smelt.define foo(x) AS (x + 1)` produces one `SMELT_DEFINE` with `DEFINE_NAME`, `PARAM_LIST(PARAM(x))`, `DEFINE_BODY(expr)`. Errors empty.
2. `parses_typed_params` — `smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double> AS (...)`.
3. `parses_default_values` — `smelt.define foo(x: Expr<Integer> = 0) AS (x)`.
4. `parses_file_with_define_and_model` — file with one define + bare SELECT; `File::defines()` length 1, `File::select_stmt()` `Some`.
5. `parses_multiple_defines` — three defines in one file.
6. `error_recovery_missing_as` — malformed define emits diagnostic but parser continues to next top-level item.
7. `error_recovery_unbalanced_body` — unbalanced `(` in body errors and syncs on next `smelt.define` / EOF.
8. `smelt_define_in_expression_position_is_not_special` — `SELECT smelt.define FROM t` parses as column reference.

**Implementation.**
- `crates/smelt-parser/src/syntax_kind.rs`: add `SMELT_DEFINE`, `PARAM_LIST`, `PARAM`, `DEFINE_BODY`, `TYPE_REF`, `DEFINE_NAME`, `RETURN_ARROW`, `DEFAULT_VALUE`.
- `crates/smelt-parser/src/parser.rs::parse_file`: loop over top-level items, dispatch on `smelt.define` vs SELECT/WITH/VALUES.
- New `parse_smelt_define`, `parse_param_list`, `parse_type_ref`. TypeRef is a flat tree in this phase; structured parsing is Phase 4.
- `crates/smelt-parser/src/ast.rs`: wrappers `SmeltDefine`, `Param`, `TypeRef`, `DefineBody`, `File::defines()`.

**Example fixtures.** Create `examples/functions_demo/` with `smelt.yml`, `sources.yml`, `models/` (one trivial passthrough model), and `functions/trivial.sql` containing a single `smelt.define trivial(x) AS (x + 1)`. The directory must register in the example-diagnostics test harness so CI exercises it from Phase 1 onward.

**Review checklist.**
- `parse_file` handles mixed top-level items without panics.
- `smelt.define` in expression/column position is NOT a declaration.
- Error recovery uses existing `sync_to`.
- All new SyntaxKind variants appear in the debug printer.
- AST wrappers follow the `cast`/`syntax` pattern.
- No existing parser tests regress.
- `examples/functions_demo/` is registered with `cargo test -p smelt-cli --test example_diagnostics` and stays clean.

**Commit.** `parser: add smelt.define top-level grammar (Phase 1, smelt-functions Step 1)`

### Phase 2 — Parser: `smelt.fn.*` call syntax

**Goal.** Parse `smelt.fn.namespace.name(arg, named => value)` as a new CST node distinct from plain function calls.

**Pre-conditions.** Phase 1.

**TDD tests** (parser unit tests):
1. `parses_smelt_fn_call_simple` — `SELECT smelt.fn.safe_divide(a, b) FROM t` → `SMELT_FN_CALL`.
2. `parses_smelt_fn_call_named_args` — reuses `NAMED_PARAM`.
3. `parses_smelt_fn_call_nested_namespace` — `smelt.fn.core.safe_divide(...)`.
4. `smelt_fn_without_parens_is_error`.
5. `smelt_fn_inside_where` — predicate position works.

**Implementation.** `parse_smelt_fn_call` branching from `parse_primary_expression`; peek `smelt.fn.` prefix. CST node `SMELT_FN_CALL` with `CALL_PATH` + `ARG_LIST`.

**Example fixtures.** Add `examples/functions_demo/models/uses_trivial.sql` that selects `smelt.fn.trivial(1)` to prove the call parses (no type-checking yet — that's Phases 5–6). The model's FROM clause should still target a source so downstream phases can extend it.

**Review checklist.** Only recognised in expression position. `FROM smelt` still works if user has a table called `smelt`. Reuses `parse_argument`. No existing function-call regressions. `uses_trivial.sql` parses without error in the demo workspace.

**Commit.** `parser: add smelt.fn.* call syntax (Phase 2, Step 1)`

### Phase 3 — Salsa function signature index

**Goal.** `functions_in_file(path) -> Arc<Vec<FunctionSig>>` and `resolve_function(fq_name) -> Option<FunctionSig>` queries. Separate `function_signature` and `function_body` queries per §20H. Bodies not yet type-checked.

**Pre-conditions.** Phases 1–2.

**TDD tests** (`crates/smelt-db/tests/function_registry.rs`):
1. `function_declarations_indexed` — workspace with `functions/safe_divide.sql`; query returns one sig.
2. `duplicate_function_name_across_files_diagnostic` — emits at the second declaration.
3. `function_body_invalidation_separate_from_signature` — body edit leaves `function_signature` output unchanged (use Salsa event counters).

**Implementation.**
- `smelt-types::signatures::{FunctionSig, ParamSpec, Tier}`.
- `smelt-db::lib`: queries `functions_in_file`, `function_signature`, `function_body`, `resolve_function`. Collision detection at workspace scope.
- New `DiagnosticCode::DuplicateFunctionDefinition`.

**Example fixtures.** Add `examples/functions_demo/functions/identity.sql` (a second `smelt.define`) so the registry sees more than one entry. Create a negative fixture `examples/broken/models/fn_duplicate_define.sql` + sibling file that share a name, exercising `DuplicateFunctionDefinition`. Since the broken integration-test harness arrives in Phase 6, Phase 3 asserts the diagnostic via a temporary `crates/smelt-db/tests/function_registry.rs` workspace fixture — the Phase 6 harness later absorbs the assertion.

**Review checklist.** Body edits don't invalidate `function_signature` (prove with event counter). Duplicate diagnostic points only at the second file. PathBuf used consistently. `functions_demo` stays clean; `broken/models/fn_duplicate_define.sql` produces exactly the expected diagnostic.

**Commit.** `db: index smelt.define signatures with split signature/body queries (Phase 3)`

### Phase 4 — `Expr<T>` type-reference resolution

**Goal.** Parse `Expr<Integer>`, `Expr<Numeric>`, `Expr<Boolean>` into a structured `SmeltType`. No generics yet.

**Pre-conditions.** Phase 3.

**TDD tests** (`smelt-types/src/signatures.rs`):
1. `parses_expr_of_concrete_type` — `Expr<Integer>` → `SmeltType::Expr(DataType::Integer)`.
2. `parses_expr_of_numeric_constraint` — `Expr<Numeric>` → `SmeltType::Expr(TypeConstraint::Numeric)`.
3. `rejects_unknown_sort` — `TableExpr<T>` → `Err(UnsupportedSort)` (deferred to Step 3).
4. `rejects_nested_expr` — `Expr<Expr<Integer>>` errors.
5. `numeric_constraint_accepts_integer` / `rejects_text` — helper `satisfies(DataType, TypeConstraint)`.

**Implementation.** `smelt-types/src/signatures.rs` with `SmeltType`, `TypeConstraint { Numeric, Any }` (Ordered in Phase 7). Pure `parse_smelt_type`. Wire into Phase 3's `FunctionSig` construction.

**Example fixtures.** Upgrade `examples/functions_demo/functions/trivial.sql` to `smelt.define add_one(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)` and add `functions/abs_numeric.sql` with a `Expr<Numeric>` parameter. Add a broken fixture `examples/broken/models/fn_bad_type_ref.sql` declaring a function with `TableExpr<T>` to exercise the "unsupported sort" diagnostic (asserted via a targeted unit test in Phase 4 until the Phase 6 harness arrives).

**Review checklist.** `smelt-types` stays dependency-free. Numeric membership matches §16 #9 exactly (SmallInt, Integer, BigInt, Float, Double, Decimal). Everything pure and unit-testable. `functions_demo` still diagnostic-clean after signature tightening.

**Commit.** `types: parse Expr<T> signatures with numeric constraint (Phase 4)`

### Phase 5 — Tier 1 body check with parameter binding

**Goal.** Type-check `smelt.define` bodies by binding param names into a seeded `TypeContext`. Re-use `infer_expression_type`. Diagnostics fire inside the body.

**Pre-conditions.** Phases 3, 4.

**TDD tests** (`crates/smelt-db/tests/function_body_check.rs`):
1. `safe_divide_body_checks_ok`.
2. `body_type_error_reported` — `numerator + "text"` → TypeMismatch at inner expression.
3. `body_references_unknown_param` → `UnknownIdentifier`.
4. `duplicate_param_name_is_error`.

**Implementation.**
- Extend `TypeContext` with `function_params: HashMap<String, TypedColumn>` and a `lookup_identifier` that checks params first (§16 #1).
- New Salsa query `check_function_body(path, name)`; `file_diagnostics` aggregates.
- New codes: `FunctionBodyTypeMismatch`, `UnknownIdentifier`.

**Example fixtures.** Add `examples/functions_demo/functions/safe_divide.sql` (the canonical end-to-end example from §2) with a correct body — body now checks clean. Add broken fixtures `examples/broken/models/fn_body_type_mismatch.sql` (adding Text to Integer inside the body) and `examples/broken/models/fn_unknown_param.sql` (body references `z` when only `x, y` declared). These are asserted via a temporary fixture-driven test in `function_body_check.rs`; Phase 6 migrates them into the unified `broken_function_diagnostics.rs` harness.

**Review checklist.** `type_inference.rs` stays pure. Params resolve before SQL FROM scope. Test 2's diagnostic range is the *inner* bad subexpression. Zero-error bodies don't allocate an empty frame stack. `functions_demo` remains clean with the new `safe_divide.sql`.

**Commit.** `db: Tier 1 body type-check with parameter binding (Phase 5)`

### Phase 6 — Call-site expansion + single-level frame trace (Step 1 complete)

**Goal.** `smelt.fn.safe_divide(x, y)` binds `numerator → typeof(x)`, `denominator → typeof(y)`, re-checks the body, surfaces errors at the call site with `FrameInfo`. `safe_divide` example ships green end-to-end.

**Pre-conditions.** Phases 2, 3, 4, 5.

**TDD tests** (`crates/smelt-db/tests/smelt_fn_call_check.rs`):
1. `safe_divide_call_types_correctly`.
2. `wrong_arg_type_error_at_call_site` — Text passed to `Expr<Numeric>` → diagnostic at arg span with `FrameInfo` for `safe_divide.numerator`.
3. `named_args_bind_correctly`.
4. `missing_required_arg_error`.
5. `default_value_fills_missing_arg`.
6. `unknown_smelt_fn_error` → `UnknownSmeltFn`.
7. `e2e_example_diagnostics_clean` — fixture added under `examples/` stays clean under `cargo test -p smelt-cli --test example_diagnostics`.
8. `frame_stack_only_innermost_rendered` — nested `a(b(wrong))` renders innermost frame only (§16 #16).

**Implementation.**
- Route `smelt.fn.*` through the Phase 3 registry inside `infer_function_type`.
- Thread a `Vec<FrameInfo>` frame stack through the pure checker — always populated, regardless of nesting depth.
- Extend `DiagnosticData` with `ExpansionFrames(Vec<FrameInfo>)`.
- LSP `to_lsp_diagnostic` appends a single-line "in expansion of `X`, `p` was bound to <type>" using the innermost frame.
- New `DiagnosticCode::UnknownSmeltFn`.

**Example fixtures.** Update `examples/functions_demo/models/uses_trivial.sql` → `models/uses_safe_divide.sql` that successfully calls `smelt.fn.safe_divide(revenue, clicks)` against a source. Add broken fixtures `examples/broken/models/fn_call_wrong_arg_type.sql` (pass Text to `Expr<Numeric>`), `fn_call_missing_arg.sql`, and `fn_call_unknown.sql` (call `smelt.fn.does_not_exist`). **Introduce** `crates/smelt-cli/tests/broken_function_diagnostics.rs` as the unified harness: it iterates the `examples/broken/models/fn_*.sql` files, runs `file_diagnostics`, and asserts each fixture produces its expected `DiagnosticCode` + message substring. Migrate the Phase 3/4/5 ad-hoc assertions into this harness so there is one place to extend from Phase 7 onward.

**Review checklist.** Frame stack populated for arbitrary depth (assert via unit test). `DiagnosticData` change is backward-compatible. `safe_divide` example fixture registered with CI. LSP e2e test confirms diagnostic reaches client. `broken_function_diagnostics.rs` covers every `fn_*.sql` in `examples/broken/models/` — no orphan fixtures.

**Commit.** `db+lsp: smelt.fn.* call checking with single-level frame trace (Phase 6, Step 1 complete)`

---

## Step 2 — Phases 7 to 12

### Phase 7 — `Ordered` constraint + canonical registry skeleton

**Goal.** Add `TypeConstraint::Ordered` per §16 #13. Define registry data shape in `smelt-types` with seed monomorphic entries (LOWER, UPPER, ABS scalar, LENGTH).

**Pre-conditions.** Phase 4.

**TDD tests** (`smelt-types/src/signatures.rs`):
1. `ordered_members_match_decision_13` — exhaustive enumeration.
2. `ordered_excludes_composites` — Array/Struct/Map not members.
3. `numeric_is_subset_of_ordered`.
4. `registry_lookup_by_name` — `BuiltinRegistry::resolve("LOWER")`.
5. `registry_lookup_case_insensitive`.

**Implementation.** `smelt-types::signatures::{BuiltinRegistry, Signature, TypeConstraint::Ordered}`. Registry is `once_cell::Lazy<HashMap<&str, Signature>>` or `phf_map`. No generics here.

**Example fixtures.** Extend `examples/functions_demo/models/uses_safe_divide.sql` with `LOWER(name)`, `UPPER(code)`, `LENGTH(description)`, `ABS(balance)` calls to exercise registry lookup. No broken fixtures this phase — Phase 7 adds data only.

**Review checklist.** Registry is pure, no Salsa dep. Ordered list exhaustive. ASCII-lowercase case-insensitivity (SQL convention). `functions_demo` models typecheck against the new registry entries.

**Commit.** `types: Ordered constraint and canonical built-in registry skeleton (Phase 7)`

### Phase 8 — Generics + variadics

**Goal.** Extend `Signature` with type parameters, constraints, and trailing variadic. Inference: LUB for promotion-chain constraints (Numeric), unification otherwise (§16 #14 + #15).

**Pre-conditions.** Phase 7.

**TDD tests** (signatures unit):
1. `min_generic_preserves_input_type` — `MIN<T: Ordered>(T) → T` with Integer returns Integer.
2. `coalesce_lub_of_numeric_args` — `COALESCE(Integer, BigInt, Double)` → Double.
3. `coalesce_text_int_rejects` — error points at arg 2.
4. `greatest_variadic_allows_single_arg`.
5. `concat_zero_args_returns_text` — `CONCAT(Text...) → Text` at zero-arity.
6. `generic_inference_error_cites_positions` — message format per §16 #14.

**Implementation.** `Signature { type_params, params: Vec<ParamSpec>, return: TypeExpr }` where `ParamSpec` is `Concrete | Var | Variadic(Box)`. `unify_call` routine. Seed ~30 most-used built-ins (SUM, AVG, MIN, MAX, COUNT, COALESCE, GREATEST, LEAST, ABS, POWER, SQRT, LOG, LN, LOWER, UPPER, LENGTH, SUBSTRING, TRIM, CONCAT, IS NULL, NULLIF, CAST, date/time basics).

**Example fixtures.** Add `examples/functions_demo/models/uses_generics.sql` calling `MIN(event_time)`, `COALESCE(revenue, 0)`, `GREATEST(a, b, c)`, `CONCAT(first_name, ' ', last_name)` — each exercising a different generic/variadic form. Add broken fixtures `examples/broken/models/fn_coalesce_text_int.sql` (mixed Text/Integer args) and `fn_greatest_no_args.sql` (zero-arity variadic). Append rows to `broken_function_diagnostics.rs`.

**Review checklist.** LUB reuses `promote_types`. Variadic zero-args error is local (§16 #15). No silent coercion between concrete numeric types (§16 #9). Inert hook for bidirectional expected-return (Step 5 hook). `functions_demo/models/uses_generics.sql` types clean; both broken fixtures asserted.

**Commit.** `types: generics and variadics for built-in signatures (Phase 8)`

### Phase 9 — Rewire built-in inference through the registry

**Goal.** Replace the hand-written match at `type_inference.rs:541` with a registry lookup. Preserve current behaviour.

**Pre-conditions.** Phases 7, 8.

**TDD tests.**
1. Existing `smelt-db` type-inference tests still pass.
2. Property test `type_property_tests.rs` still passes (or divergences are updated with justification).
3. `unrecognized_function_uses_existing_code` — untouched surface.
4. `sum_of_decimal_returns_decimal` — matches §16 #9.
5. `min_of_timestamp_returns_timestamp` — type-preserving via generics.

**Implementation.** `infer_function_type` becomes a thin wrapper: registry first, fall back to legacy match for anything not yet migrated. Delete hardcoded entries as they migrate. Spike first to confirm coverage before starting the rewrite.

**Example fixtures.** No new fixtures — this phase's test is that every existing `examples/functions_demo/` model still types cleanly after rewire, and that `examples/timeseries/` and `examples/retail_analytics/` (which exercise many built-ins) remain diagnostic-clean. This is the phase where the existing example workspaces become load-bearing regression coverage. If any of them regress, the rewrite is incomplete.

**Review checklist.** Property tests pass. Any `SqlFunction` variant removed from the legacy match has a registry entry. Diagnostic messages at least as good as before. `cargo test -p smelt-cli --test example_diagnostics` passes across all non-broken workspaces.

**Commit.** `db: route SQL built-in inference through canonical registry (Phase 9)`

### Phase 10 — `smelt.extern` declarations

**Goal.** Parse and index `smelt.extern name(params) -> ReturnType` per §16 #21. Collision diagnostic against built-ins and other externs.

**Pre-conditions.** Phases 1 (parser top-level dispatch), 9 (registry is canonical).

**TDD tests.**
1. `parses_smelt_extern_minimal`.
2. `extern_with_frontmatter_backends` — per-decl frontmatter attaches (still using legacy frontmatter rule; Phase 11 upgrades).
3. `extern_call_typed_like_builtin`.
4. `extern_collision_with_builtin_is_error`.
5. `extern_duplicate_declaration_is_error`.

**Implementation.**
- Parser: `parse_smelt_extern` analogous to `parse_smelt_define`, no body.
- Extend Phase 3's `functions_in_file` to collect externs. Unified resolver `builtin || user_extern`.
- Collision enforced at index time.

**Example fixtures.** Add `examples/functions_demo/functions/externs.sql` declaring a `smelt.extern regex_match(text: Expr<Text>, pattern: Expr<Text>) -> Expr<Boolean>` and call it from a new model. Add broken fixtures `examples/broken/models/fn_extern_collides_with_builtin.sql` (extern named `LOWER`) and `fn_extern_duplicate.sql`. Append rows to `broken_function_diagnostics.rs`.

**Review checklist.** Externs reuse Phase 3 Salsa. Collision spans built-ins AND user externs. Frontmatter in this phase uses legacy single-block rule (Phase 11 upgrade point). `functions_demo` model calling the extern types clean.

**Commit.** `parser+db: smelt.extern declarations and signature indexing (Phase 10)`

### Phase 11 — Per-declaration frontmatter + `backends:` inference + backend namespace

**Goal.** Move to per-declaration frontmatter (§16 #22). Parse `backends:`. Infer a function's `backends` set from body calls (narrow-only, §16 #23). Backend namespace `duckdb.*` resolves via the registry.

**Pre-conditions.** Phase 10.

**TDD tests.**
1. `frontmatter_attaches_to_next_decl` — file with two `---`/`---` blocks, one per decl.
2. `backends_inferred_from_calls` — canonical body → `all`; body with `duckdb.read_parquet` → `[duckdb]`.
3. `declared_backends_narrows`.
4. `declared_backends_widening_is_error`.
5. `duckdb_namespace_sugar_equivalent_to_frontmatter` — `smelt.extern duckdb.foo(...)` equivalent to explicit `backends: { duckdb: { emit: foo } }`.
6. `old_file_level_frontmatter_on_lone_model_still_works` — backwards compat.

**Implementation.**
- `smelt-parser/src/lib.rs::strip_frontmatter` → `find_frontmatter_blocks` returning all blocks with byte ranges. Preserve comment-replacement trick for line-number stability.
- Parser attaches frontmatter to the following declaration.
- `smelt-db`: `function_backends(path, name)` query. Narrow-only check.
- New `DiagnosticCode::BackendsWideningNotAllowed`.

**Example fixtures.** Add `examples/functions_demo/functions/multi_decl.sql` with two declarations each preceded by their own `---/---` frontmatter block (one `smelt.define` with `backends: [duckdb]`, one `smelt.extern duckdb.read_parquet(...)` demonstrating the namespace-sugar form). Upgrade `functions_demo/functions/safe_divide.sql` to include a per-decl frontmatter block. Add broken fixture `examples/broken/models/fn_backends_widening.sql` that declares a caller with broader `backends` than its body allows. Append row to `broken_function_diagnostics.rs`. Keep an existing single-block fixture (e.g. `examples/timeseries/` models) untouched to prove backwards compat.

**Review checklist.** LSP line numbers still correct post multi-block frontmatter. Existing single-block fixtures unaffected. `backends:` schema strict but extensible. Backend namespace resolution is syntactic — no special type-checker codepath. `functions_demo/functions/multi_decl.sql` both types and resolves backends correctly.

**Commit.** `parser+db: per-declaration frontmatter, backends inference, backend namespace (Phase 11)`

### Phase 12 — Multi-level frame rendering + CAST-enforcement flag (Step 2 complete)

**Goal.** Upgrade Step 1's single-frame renderer to full-stack (§16 #16 Step 2 deliverable). Record `needs_cast` on registry return types where canonical differs from engine-native (§16 #9) — recording only; SQL emission is Step 7+.

**Pre-conditions.** Phase 6 (stack already populated), Phase 9 (registry has return metadata).

**TDD tests.**
1. `nested_call_error_renders_all_frames` — `a(b(c(wrong)))` produces 3 frames, outer-to-inner.
2. `single_level_call_unchanged` — Phase 6 tests still pass.
3. `cast_flag_set_when_canonical_differs_from_engine` — `SUM(Integer)` on DuckDB: registry `canonical = BigInt`, engine native = `HUGEINT` → `needs_cast = true` via unit hook.
4. `lsp_diagnostic_formats_frames_as_related_information` — LSP e2e: nested-call error includes `relatedInformation` per frame.

**Implementation.**
- `smelt-lsp::to_lsp_diagnostic` + `smelt-cli` printer iterate full `ExpansionFrames` vec.
- LSP uses `DiagnosticRelatedInformation` for per-frame links.
- `Signature` gains `canonical_return: DataType` plus `engine_native: HashMap<BackendId, DataType>`. Checker flags `needs_cast` on divergence.
- CAST emission itself deferred — documented.

**Example fixtures.** Add `examples/functions_demo/functions/nested_helpers.sql` with a chain `smelt.define outer(x) AS (smelt.fn.middle(x))`, `middle(x) AS (smelt.fn.safe_divide(x, 0))` to exercise multi-level expansion on the happy path. Add broken fixture `examples/broken/models/fn_nested_call_error.sql` that calls `outer("bad_text")` — asserts the full multi-frame renderer and is appended to `broken_function_diagnostics.rs` with a check that the diagnostic's rendered message includes all three frame names (outer-to-inner). This is the final fixture and the full-stack renderer's canonical test.

**Review checklist.** Single-level cases unchanged. `relatedInformation` URIs resolve in LSP client. `needs_cast` computed but not emitted — clearly documented. All example projects still diagnostic-clean. Nested broken fixture's rendered output matches §16 #16's Step 2 example in the research.

**Commit.** `lsp+db: multi-level frame rendering and CAST canonical-return tracking (Phase 12, Step 2 complete)`

---

## Step 3 — Phases 13 to 18

### Phase 13 — Parser: TableExpr / WindowExpr / SelectItems<K, ctx> in type refs

**Goal.** Parser accepts `TableExpr`, `TableExpr<{col: Type [, ..]}>`, `AggExpr<T>`, `WindowExpr<T>`, and `SelectItems<K, ctx>` / `SelectItems<K>` / `SelectItems<ctx>` in parameter type positions. CST only; no type-system wiring (that lands Phase 14).

**Pre-conditions.** Phases 1–12.

**TDD tests** (`crates/smelt-parser/src/parser.rs`):
1. `parses_tableexpr_bare` — `source: TableExpr` parses; AST wrapper exposes `kind() == TableExpr`.
2. `parses_tableexpr_with_row_requirement` — `source: TableExpr<{revenue: Numeric, cost: Numeric}>` carries a `ROW_REQUIREMENT` node with two fields.
3. `parses_tableexpr_with_row_tail` — `source: TableExpr<{revenue: Numeric, ..r}>` carries `ROW_TAIL_NAMED(r)`; bare `..` is `ROW_TAIL_ANON`.
4. `parses_aggexpr_and_windowexpr` — `Expr<T>`, `AggExpr<T>`, `WindowExpr<T>` each produce a distinct `EXPR_KIND` sibling to `DATA_TYPE`.
5. `parses_selectitems_kind_only` — `SelectItems<Agg>`.
6. `parses_selectitems_kind_and_ctx` — `SelectItems<Agg, sessionized>` — both children in declared order.
7. `parses_selectitems_ctx_only` — `SelectItems<sessionized>` (no kind).
8. `rejects_unknown_expr_kind` — `FooExpr<T>` still errors with the Phase 4 unknown-sort diagnostic.
9. `tableexpr_in_expression_position_is_not_special` — `SELECT TableExpr FROM t` parses as a column reference.

**Implementation.**
- `crates/smelt-parser/src/syntax_kind.rs`: `EXPR_KIND`, `ROW_REQUIREMENT`, `ROW_FIELD`, `ROW_TAIL_NAMED`, `ROW_TAIL_ANON`, `SELECTITEMS_KIND`, `SELECTITEMS_CTX`.
- `parse_type_ref` recognises the additional head keywords and dispatches.
- Pure parser; no type-check interaction.

**Example fixtures.** Add `examples/functions_demo/functions/add_margin.sql` with `smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (SELECT source.*, revenue - cost AS margin FROM source)` (body type-check fires in Phases 14–15; this phase only verifies parsing). Function-file discovery now recurses into subdirectories — add `functions/patterns/pred_demo_stub.sql` to cover directory-namespace matching and close §21's "function file discovery" item.

**Review checklist.** All new kinds in the debug printer. No regressions in Phase 1–12 parser tests. AST wrappers follow the `cast`/`syntax` pattern.

**Commit.** `parser: TableExpr/WindowExpr/SelectItems<K,ctx> type-ref grammar (Phase 13, Step 3 opens)`

### Phase 14 — Types: WindowExpr sort and SelectItems<K> kind ceiling

**Goal.** Add `ExprKind { Scalar, Agg, Window }` in `smelt-types`. Every typed node in the checker synthesises a kind alongside its `DataType`. Implement the linear subtype check. `SelectItems<K, ctx>` value synthesis computes `K` as the ceiling of its contents.

**Pre-conditions.** Phase 13.

**TDD tests** (`smelt-types` + `smelt-db`):
1. `window_func_synthesises_window_kind` — `ROW_NUMBER() OVER (...)` → `(DataType::BigInt, ExprKind::Window)`.
2. `aggregate_in_select_synthesises_agg_kind` — `SUM(revenue)` → `(..., Agg)`; `revenue` alone → `(..., Scalar)`.
3. `kind_subtype_chain` — `Scalar <= Agg <= Window`; non-comparable pairs error.
4. `selectitems_kind_ceiling` — `[user_id, COUNT(*)]` has ceiling `Agg`; `[COUNT(*) OVER (...)]` has ceiling `Window`.
5. `where_clause_rejects_window_kind` — `WHERE ROW_NUMBER() OVER (...) > 1` → `WindowInScalarContext`.
6. `groupby_rejects_window_kind` — same diagnostic.

**Implementation.**
- `smelt-types::signatures::ExprKind` plus `fn subkind_of(a, b) -> bool`.
- `smelt-db::type_inference` threads `kind` through every `infer_expression_type` return (tuple or small `TypedValue { data_type, kind, needs_cast }`).
- `SelectItems` value is `(Vec<TypedValue>, ctx)`; kind derived on demand.
- New `DiagnosticCode::WindowInScalarContext`.

**Example fixtures.** Extend `examples/functions_demo/functions/nested_helpers.sql` with a window-function call. Add broken fixture `examples/broken/models/fn_window_in_where.sql`. Append to `broken_function_diagnostics.rs`.

**Review checklist.** `type_inference.rs` stays pure. Every existing signature-registry entry still compiles (seed returns kinds explicitly). No property-test regressions. `AggExpr<T>` kept as a distinct sort — close §21's keep-or-collapse item under "Deferred during implementation".

**Commit.** `types+db: ExprKind with linear subtyping and SelectItems<K> ceiling (Phase 14)`

### Phase 15 — TableExpr parameters: bare-column row polymorphism + shadow warnings

**Goal.** A `TableExpr` parameter introduces the caller-supplied table's schema into the body's SQL FROM scope. Bare column references resolve through standard SQL column resolution when no parameter name matches (§16 #7). Shadow warnings fire when a parameter name matches a column in scope (§16 #1).

**Pre-conditions.** Phase 14.

**TDD tests** (`crates/smelt-db/tests/tableexpr_body_check.rs`):
1. `add_margin_body_checks_ok` — call `add_margin` with a concrete table `{revenue, cost}`.
2. `bare_column_resolves_from_tableexpr_schema`.
3. `missing_column_at_call_site` — call with a table lacking `revenue` → `UnknownIdentifier` at the bare reference, with frame stack rooted at the call site.
4. `param_shadows_column_emits_warning` — `smelt.define f(user_id: Expr<Text>, source: TableExpr) AS (SELECT user_id FROM source)` called with a `user_id` column in `source` → `Severity::Warning` on the parameter decl; body still typechecks with parameter as `Expr<Text>`.
5. `qualified_access_escapes_shadow` — `source.user_id` resolves to the column.

**Implementation.**
- Extend `TypeContext` with a FROM-scope layer fed from `TableExpr` parameters at call-site expansion.
- New `DiagnosticCode::ParameterShadowsColumn` (warning severity).
- Reuse Phase 5's parameters-first lookup; extend second-tier lookup to cover `TableExpr`-derived scope.

**Example fixtures.** Upgrade `functions/add_margin.sql` to call-site test via a model `models/margin_report.sql` using `smelt.ref('orders')`. Add broken fixtures `fn_tableexpr_missing_col.sql` and `fn_tableexpr_shadow_warning.sql`.

**Review checklist.** `type_inference.rs` still pure. Warning-severity plumbing works end-to-end to LSP. Frame stack populated for call-site errors.

**Commit.** `db: TableExpr row polymorphism with parameters-first scoping (Phase 15)`

### Phase 16 — Row-requirement annotations: TableExpr<{…}> pre-expansion checking

**Goal.** `TableExpr<{col: Type, ..r}>` verifies at the call site that the caller's table has at least the declared columns with compatible types, before expansion. Missing or wrong-type columns produce a call-site error with frame stack.

**Pre-conditions.** Phase 15.

**TDD tests.**
1. `row_requirement_satisfied_by_superset_schema`.
2. `row_requirement_missing_column_errors_at_call_site` — error cites the requirement's span; no expansion runs.
3. `row_requirement_wrong_type_errors_at_call_site`.
4. `row_tail_allows_extra_columns` — `TableExpr<{revenue: Numeric, ..r}>` binds `r` to the remainder; bare `r` is not yet usable in the return (that's Phase 17 / Phase 37).
5. `row_tail_anonymous_allows_any_extras`.
6. `row_requirement_at_tier1_function_body_still_checks_on_expansion`.

**Implementation.**
- `SchemaRequirement { required: Vec<(String, DataTypeReq)>, tail: RowTail }` where `RowTail = None | Anon | Named(String)`.
- Call-site checker runs the requirement against the argument's schema before invoking the body checker.
- Named tails bind into a per-call `row_var_env` carried on the frame stack.

**Example fixtures.** Tighten `functions/add_margin.sql` to `TableExpr<{revenue: Numeric, cost: Numeric}>`. Add broken fixture `fn_row_requirement_missing.sql`.

**Review checklist.** Requirement errors fire at the call site, not inside the body. `row_var_env` observable via a unit-test-only accessor.

**Commit.** `db: TableExpr row-requirement annotations with call-site checking (Phase 16)`

### Phase 17 — `sessionize` end-to-end: TableExpr + WindowExpr in body

**Goal.** Full end-to-end working of the `sessionize` example (research §3). Parameters `(source: TableExpr, user_col: Expr<Text>, ts_col: Expr<Timestamp>, gap: Expr<Interval> = …)`, body uses `LAG()` and `SUM() OVER (…)`, return is `TableExpr`. Demonstrates TableExpr output-schema inference.

**Pre-conditions.** Phase 16.

**TDD tests.**
1. `sessionize_body_types_clean`.
2. `sessionize_output_schema_inferred` — returned `TableExpr` schema is `source schema ∪ {session_id: BigInt}`.
3. `sessionize_windowexpr_in_body_accepted`.
4. `sessionize_missing_ts_col_on_source_errors`.
5. `sessionize_default_gap_applied_when_omitted`.

**Implementation.**
- `TableExpr` return-type inference walks the body's SELECT list to synthesise the output schema; `source.*` expands from the caller's bound schema.
- Default-value expansion (parsed back in Phase 1) finally runs end-to-end at call resolution.

**Example fixtures.** Add `examples/functions_demo/functions/sessionize.sql` (verbatim §3 example). Add `models/sessions_report.sql` using it against `sources.yml` data.

**Review checklist.** Output-schema correctness asserted in a unit test and manually via LSP hover. Default-value provenance (`Synthesized`) attached correctly.

**Commit.** `db: sessionize TableExpr function with window-function body (Phase 17)`

### Phase 18 — LSP polish + pipeline example (Step 3 complete)

**Goal.** LSP hover shows TableExpr schemas (columns and types). `functions_demo` grows an end-to-end pipeline chaining `add_margin` → `sessionize`. `docs/ROADMAP.md` marks Step 3 complete.

**Pre-conditions.** Phase 17.

**TDD tests.**
1. `lsp_hover_tableexpr_shows_schema` — LSP e2e: hover on a `TableExpr` parameter shows `{revenue: Numeric, cost: Numeric, ..}`.
2. `lsp_hover_bare_column_shows_resolved_type`.
3. `e2e_margin_to_sessions_pipeline_clean`.

**Implementation.**
- `smelt-lsp::hover`: format `TableExpr<{...}>` and `Expr<...>` using the current schema from the frame-stack binding.
- No new tests for function-file discovery — Phase 13 already covers it; this phase just confirms no regression.

**Example fixtures.** `functions_demo/models/margin_by_session.sql` chains `add_margin` → `sessionize`. No broken fixtures this phase.

**Review checklist.** Step 3 closure note added to `docs/ROADMAP.md`. LSP visual test updated. `broken_function_diagnostics.rs` coverage intact.

**Commit.** `lsp+examples: TableExpr hover and end-to-end pipeline (Phase 18, Step 3 complete)`

---

## Step 4 — Phases 19 to 22

### Phase 19 — Parser + types: context-binding syntax

**Goal.** Parser already accepts `Expr<T, ctx>` / `SelectItems<K, ctx>` from Phase 13. Phase 19 wires the context name into the signature's type representation. Type-level only; inference wires in Phase 20.

**Pre-conditions.** Phase 13.

**TDD tests.**
1. `context_binding_parsed_into_signature` — `filters: Expr<Boolean, source>` stores `context_name = "source"`.
2. `context_name_resolves_to_tableexpr_param` — if `source` is a `TableExpr` param in the same signature, `context_name` links to it.
3. `context_name_resolves_to_cte_by_name` — deferred resolution: Phase 20 hooks it up.
4. `invalid_context_name_errors_at_definition_time` — context name that matches neither a parameter nor a CTE in the body → `UnknownContext`.

**Implementation.**
- `ParamSpec` gains `context: Option<ContextRef>`. `ContextRef` is a pre-resolution string + a post-resolution pointer filled in Phase 20.
- New `DiagnosticCode::UnknownContext`.

**Example fixtures.** Add `functions/session_rollup_stub.sql` with `filters: Expr<Boolean, source>`; full example lands in Phase 22.

**Review checklist.** Decision 4 (no-overlap) still honoured — context-binding syntax does not let an author reference multiple tables at once.

**Commit.** `parser+types: context-binding parsing and resolution stub (Phase 19, Step 4 opens)`

### Phase 20 — CTE schema extraction + splice-point context inference

**Goal.** Extract CTE schemas from a function body; infer an unlabelled fragment parameter's context from its splice point (§16 #5). Closes §21's "CTE forward reference" item.

**Pre-conditions.** Phase 19, Phase 17 (schema inference machinery).

**TDD tests.**
1. `cte_schema_extracted_from_body` — body `WITH s AS (SELECT …)` produces `ctx "s" = schema`.
2. `context_inferred_from_splice_point` — `filters` spliced into `WHERE filters` on `source` scope → inferred context `source`.
3. `context_matches_explicit_annotation` — if both inferred and declared, they must agree; mismatch → `ContextMismatch`.
4. `intersection_over_multi_splice` — `pred` spliced into `WHERE pred` (over `{id, name, amount}`) and `HAVING pred` (over `{id, total}`) → inferred context = `{id}`.
5. `cte_cycle_detected` — A → B → A → `CteCycle`.

**Implementation.**
- Two-pass body analyser (see Cross-phase design choices). Pass 1: CTE schema DFS with colour-based cycle detection. Pass 2: type inference.
- Splice-point tracker: each parameter reference in the body records the local FROM-scope schema at its position.
- Intersection: field-by-field with type compatibility.

**Example fixtures.** Add `functions/pred_demo.sql` exercising multi-splice intersection. Add `broken/models/fn_cte_cycle.sql`.

**Review checklist.** Two-pass analyser is pure. Cycle detection terminates in bounded time. Field intersection symmetric.

**Commit.** `db: CTE schema extraction and context inference from splice points (Phase 20)`

### Phase 21 — Column-name validation + annotation reconciliation

**Goal.** When a caller-provided fragment is bound to a context-annotated parameter, its column references validate against the context's schema. Explicit annotations are verified against the inferred / intersected context.

**Pre-conditions.** Phase 20.

**TDD tests.**
1. `fragment_argument_column_valid_in_context`.
2. `fragment_argument_column_missing_from_context_errors` — error at the column reference inside the caller-supplied fragment.
3. `explicit_annotation_wider_than_inference_errors`.
4. `explicit_annotation_narrower_than_inference_allowed` — narrowing is fine (documentation + tighter access control).
5. `agg_kind_context_binding_checks` — `SelectItems<Agg, sessionized>` caller-side fragment must be agg-kind over `sessionized`.

**Implementation.**
- Argument-fragment checker runs the argument's body under the context's schema as FROM scope; errors carry frame stack.
- Annotation vs inference: `inferred.is_subset_of(declared)` → ok; else error.

**Example fixtures.** Positive: `functions/session_rollup_fragment.sql`. Broken: `fn_fragment_col_missing.sql`, `fn_annotation_too_wide.sql`.

**Review checklist.** Annotation narrowing is documented as an intended feature in review notes.

**Commit.** `db: context-binding column validation and annotation reconciliation (Phase 21)`

### Phase 22 — `session_rollup` end-to-end (Step 4 complete)

**Goal.** Full `session_rollup` (research §6): `source: TableExpr`, `metrics: SelectItems<Agg, sessionized> = ()`, `filters: Expr<Boolean> = TRUE`, CTE-derived context. `docs/ROADMAP.md` marks Step 4 complete.

**Pre-conditions.** Phases 20, 21.

**TDD tests.**
1. `session_rollup_body_types_clean`.
2. `caller_metrics_fragment_sees_session_id` — `sessionized` CTE context exposes `session_id` to the caller.
3. `caller_metrics_fragment_rejects_non_sessionized_column`.
4. `empty_default_metrics_splice_comma_elision` — §16 #20 list-splice rule holds in generated SQL.

**Implementation.** Mostly integration — no new mechanism beyond Phases 19–21.

**Example fixtures.** `functions/session_rollup.sql` (full §6 example) + `models/rollup_dashboard.sql` exercising it with a non-empty parenthesised `metrics` argument (parenthesised form — `PASSING` lands in Step 6).

**Review checklist.** Step 4 closure notes in `docs/ROADMAP.md` + "Deferred during implementation" (§21 CTE item closed).

**Commit.** `db+examples: session_rollup end-to-end with CTE-derived context (Phase 22, Step 4 complete)`

---

## Step 5 — Phases 23 to 27

### Phase 23 — Tier 2 body check in isolation

**Goal.** Introduce `CheckMode { Tier1Expansion(arg_types), Tier2Isolated, Tier2CallSite(expected_ret) }`. Tier 2 functions (parameters annotated) type-check their bodies against declared parameter types, independent of any call site.

**Pre-conditions.** Phase 17 (schema inference mature).

**TDD tests.**
1. `tier2_body_checks_against_declared_params`.
2. `tier2_body_error_at_definition_time` — broken body surfaces even if never called.
3. `tier2_signature_survives_broken_body` — signature remains resolvable; callers still type-check against it (LSP-stability contract, §8).
4. `tier1_body_still_requires_call_site` — unchanged semantics for unannotated functions.

**Implementation.**
- `check_function_body` (Phase 5) takes a `CheckMode`. `Tier2Isolated` feeds declared types as the context seed.
- Signature-vs-body Salsa split (Phase 3) already keeps signature stable when body breaks.

**Example fixtures.** `functions/margin_tier2.sql` — annotated version of `add_margin`; `broken/models/fn_tier2_body_broken.sql`.

**Review checklist.** Signature stability proven via Salsa event counters.

**Commit.** `db: Tier 2 body check in isolation (Phase 23, Step 5 opens)`

### Phase 24 — Tier 3 return-type verification + LSP hover

**Goal.** A fully-annotated function has its body's synthesised return type checked against the declaration. LSP hover shows the declared return type without expanding.

**Pre-conditions.** Phase 23.

**TDD tests.**
1. `tier3_body_return_matches_annotation`.
2. `tier3_body_return_mismatch_errors_at_return_expression`.
3. `lsp_hover_tier3_shows_declared_return` — no expansion in hover output.
4. `tier3_row_variable_in_return_abstract_checked` — §8 "body must produce at least the declared fields."

**Implementation.**
- After body synthesis in `Tier2Isolated` mode, compare with declaration; produce diagnostic if different.
- Row-variable handling in return: structural subsumption check against body's shape (full row-var expansion lands in Phase 37).

**Example fixtures.** `functions/margin_tier3.sql` (all three annotations). `broken/models/fn_tier3_return_mismatch.sql`.

**Review checklist.** LSP hover test uses the existing e2e infrastructure.

**Commit.** `db+lsp: Tier 3 return verification and hover (Phase 24)`

### Phase 25 — Call-site bidirectional checking (pre-expansion)

**Goal.** At a Tier 2/3 call site, push declared parameter types into argument checking (checking mode). No expansion. Errors at the argument position, not inside the body.

**Pre-conditions.** Phase 24.

**TDD tests.**
1. `tier2_call_arg_checked_against_declared_param` — error message format per §8 Tier 2 example.
2. `tier3_call_arg_checked_same_as_tier2`.
3. `tier1_call_still_uses_expansion`.
4. `checking_mode_no_expansion_performed` — Salsa event counter confirms no `function_body` access.

**Implementation.** `CheckMode::Tier2CallSite(expected)` routes the argument through checking-mode inference. Expansion skipped.

**Example fixtures.** Upgrade `models/margin_report.sql` to call the Tier 3 version; add `broken/models/fn_tier2_call_arg_wrong.sql`.

**Review checklist.** Error message format exactly matches §8 spec ("expected X, got Y" + "parameter declared as …").

**Commit.** `db: Tier 2/3 call-site bidirectional checking (Phase 25)`

### Phase 26 — Tier 2 calling Tier 1: inline expansion at the Tier 2 body check (§16 #17)

**Goal.** When a Tier 2 body reaches a Tier 1 call, expand the Tier 1 body using the Tier 2 context's parameter types as concrete argument types. Errors surface against the Tier 2 body with the frame stack rooted at the Tier 2 call site.

**Pre-conditions.** Phase 25, Phase 6 (frame stack).

**TDD tests.**
1. `tier2_calling_tier1_expands_inline`.
2. `tier1_error_surfaces_against_tier2_body` — frame stack shows Tier 2 body → Tier 1 callee.
3. `transitive_tier1_chain_expands` — Tier 2 → Tier 1 → Tier 1 all expand.
4. `tier2_signature_stable_when_tier1_callee_breaks`.
5. `expansion_caches_per_arg_type_hash` — second expansion with identical arg types is a Salsa hit.

**Implementation.**
- `check_function_body(_, Tier2Isolated)` at a Tier 1 call recursively calls itself in `Tier1Expansion` with arg types pulled from the surrounding Tier 2 type context.
- `DataTypeHash` defined for Salsa caching.

**Example fixtures.** `functions/mixed_tier.sql` — Tier 2 helper calling a Tier 1 helper. `broken/models/fn_tier2_calls_broken_tier1.sql`.

**Review checklist.** Signature stability unit-tested. Salsa cache key correctness tested.

**Commit.** `db: Tier 2 → Tier 1 inline expansion with frame-stack propagation (Phase 26)`

### Phase 27 — Generics ↔ checking-mode interaction (Step 5 complete)

**Goal.** Built-in generics participate in bidirectional inference: expected return type at the call site contributes a position for any `T` appearing in the return (§16 #14). `unify_call(arg_types, expected_return: Option<DataType>)`. Ships `docs/smelt-functions-upgrade-story.md` to close §21's "Tier 1 → Tier 2 upgrade path" item.

**Pre-conditions.** Phase 26, Phase 8 (inert hook).

**TDD tests.**
1. `coalesce_expected_double_literals_widen` — context expects `Double`, call `COALESCE(1, 2)` → both literals widen to `Double`.
2. `no_expected_return_positions_unchanged` — synthesis mode unchanged from Phase 8.
3. `expected_return_conflict_local_error` — expected `Integer`, arg types force `BigInt` → error at call site showing both sides.
4. `generics_within_tier2_body` — Tier 2 body calling `MIN(revenue)` where `revenue: Expr<Decimal>` returns `Expr<Decimal>` under Tier 2's type context.

**Implementation.**
- `unify_call` extended as above. Phase 8's inert hook becomes active.
- Step 5 closure: add `docs/smelt-functions-upgrade-story.md` with Tier 1 → Tier 2 migration guidance (callers whose concrete arg types do not satisfy a newly-declared parameter type get a clear diagnostic with suggested annotation-widening or explicit CAST).

**Example fixtures.** Extend `functions_demo/models/uses_generics.sql` with a context-expected widening case.

**Review checklist.** All property tests pass. Upgrade doc reviewed. `docs/ROADMAP.md` marks Step 5 complete.

**Commit.** `db+docs: bidirectional generics and Tier 1→Tier 2 migration story (Phase 27, Step 5 complete)`

---

## Step 6 — Phases 28 to 29

### Phase 28 — Parser: context-sensitive `PASSING` clauses

**Goal.** Parse `PASSING name AS (...)` clauses trailing a `smelt.fn.*` or user-defined function call. `PASSING` stays a regular identifier everywhere else (§16 #18).

**Pre-conditions.** Phase 2 (call syntax).

**TDD tests.**
1. `parses_single_passing_clause`.
2. `parses_multiple_passing_clauses`.
3. `passing_not_reserved_elsewhere` — `SELECT passing FROM t` parses as a column ref; `CREATE TABLE t (passing INTEGER)` clean.
4. `passing_not_attached_to_plain_sql_call` — `SELECT UPPER(x) PASSING y AS (...)` errors at the unexpected `PASSING` identifier after `UPPER(x)`.
5. `passing_after_smelt_extern_call_rejected` — §16 #18 defers externs with fragment parameters.
6. `error_recovery_malformed_passing_body`.

**Implementation.**
- Parser peeks one token after the closing `)` of a recognised smelt call.
- New CST kinds `PASSING_CLAUSE`, `PASSING_NAME`, `PASSING_BODY`.

**Example fixtures.** Positive: `examples/functions_demo/models/rollup_with_passing.sql`. Broken: `broken/models/fn_passing_on_plain_call.sql`.

**Review checklist.** No collision with CTE `WITH` syntax (not relevant — `PASSING` not `WITH`). Parser-level only; no type-checker interaction.

**Commit.** `parser: context-sensitive PASSING clauses (Phase 28, Step 6 opens)`

### Phase 29 — PASSING binding + LSP completion (Step 6 complete)

**Goal.** Bind `PASSING name AS (body)` clauses to the callee's fragment-sort parameters by name. Type-check identically to inline arguments. Basic LSP completion inside the clause body.

**Pre-conditions.** Phase 28, Phase 21 (context validation).

**TDD tests.**
1. `passing_clause_binds_to_named_parameter`.
2. `passing_clause_name_mismatch_errors`.
3. `passing_clause_type_checked_same_as_inline`.
4. `default_fills_omitted_passing`.
5. `lsp_completion_in_passing_body_shows_context_columns` — inside `PASSING metrics AS (|)`, completion lists the `sessionized` context's columns.

**Implementation.**
- Argument binding: inline args first, then PASSING clauses merge by name, then defaults fill the rest.
- LSP completion provider keys off the clause's enclosing parameter's context.
- Decision 18's context-sensitive trigger already honoured by Phase 28's parser.

**Example fixtures.** Upgrade `rollup_with_passing.sql` to the exact §10 block-syntax example.

**Review checklist.** Completion test runs against LSP e2e harness. Fragment-sort parameters without a context still accept `PASSING` (unconstrained scope).

**Commit.** `db+lsp: PASSING clause binding and contextual completion (Phase 29, Step 6 complete)`

---

## Step 7 — Phases 30 to 34

### Phase 30 — Logical plan data model: functions as first-class nodes

**Goal.** Introduce `smelt-planner::logical::{Plan, LogicalNode}` with a `FunctionCall { fn_id, args, transparent, provenance, properties }` variant. Plan is built from Phase 1–22 CST/analysis outputs; expansion deferred to Level 2 (Phase 32+).

**Pre-conditions.** Phases 1–22.

**TDD tests** (new crate `smelt-planner`):
1. `plan_builds_function_call_node`.
2. `transparent_flag_matches_function_transparency` — `smelt.define` → transparent=true; `smelt.extern` → transparent=false.
3. `properties_propagate_from_frontmatter` — `deterministic: true` → `node.properties.deterministic == true`.
4. `plan_is_salsa_query` — `logical_plan(model_path)` cached; unrelated file edits don't invalidate.

**Implementation.**
- `crates/smelt-planner/` new crate (see `docs/planner_rule_api_design.md`).
- `Plan` is an `Arc<LogicalNode>` tree; no rules run yet.
- Salsa query `logical_plan(model_path)` in `smelt-db` builds the plan.

**Example fixtures.** No new SQL fixtures; verified through unit tests.

**Review checklist.** New crate has no cyclic dependency with `smelt-db`. Plan build is pure.

**Commit.** `planner: logical plan data model with function-call nodes (Phase 30, Step 7 opens)`

### Phase 31 — Column provenance + declared-property propagation

**Goal.** Function-call nodes carry column provenance (which output column comes from which input) and declared properties. v1 accepts explicitly declared provenance via frontmatter `provenance:` under an unstable-schema flag; automatic derivation deferred.

**Pre-conditions.** Phase 30.

**TDD tests.**
1. `provenance_parsed_from_frontmatter` — `provenance: { margin: [source.revenue, source.cost] }` attached to node.
2. `undeclared_provenance_is_opaque` — node marked `provenance: Unknown`.
3. `deterministic_idempotent_append_only_propagate`.
4. `provenance_schema_frozen_under_unstable_flag` — diagnostic if `provenance:` used without the unstable-schema flag set in `smelt.yml`.

**Implementation.**
- Extend the Phase 11 frontmatter parser with `provenance:` / `joins:` keys (YAML maps).
- Validate keys against schema version; require an opt-in flag in `smelt.yml`.

**Example fixtures.** `functions/add_margin_with_provenance.sql`.

**Review checklist.** Unstable-schema flag documented in `docs/ROADMAP.md`.

**Commit.** `planner: column provenance and declared properties on function nodes (Phase 31)`

### Phase 32 — Planner rule API + Level 2 expansion of function calls

**Goal.** `PlannerRule` trait with `apply(&mut Plan, &Context) -> RuleResult`. Fixed-point loop. First rule: `ExpandTransparentFunctionCalls` — expands every transparent `FunctionCall` node into its body subtree, with provenance preserved. Also wires CAST *emission* for canonical returns recorded in Phase 12.

**Pre-conditions.** Phase 31.

**TDD tests.**
1. `planner_rule_trait_shape`.
2. `expansion_rule_replaces_transparent_calls`.
3. `black_box_calls_left_intact`.
4. `fixed_point_terminates` — expansion runs once per transparent node; marker prevents re-expansion.
5. `expansion_preserves_provenance`.
6. `cast_emitted_for_needs_cast_returns` — `SUM(Integer)` on DuckDB emits `CAST(... AS BIGINT)`.

**Implementation.**
- `PlannerRule` trait + fixed-point loop.
- `ExpandTransparentFunctionCalls` built from Phase 12's expansion machinery.
- CAST emission driven by the `needs_cast` flag recorded in Phase 12.
- Termination marker on expanded nodes.

**Example fixtures.** Unit tests only.

**Review checklist.** Rule API is ergonomically close to `docs/planner_rule_api_design.md`. Fixed-point loop bounded.

**Commit.** `planner: rule API with transparent-function expansion and CAST emission (Phase 32)`

### Phase 33 — Filter pushdown across transparent-function boundaries

**Goal.** Second planner rule: push `WHERE` predicates below a transparent `FunctionCall` into the callee's body. Uses column provenance to decide which predicates are safe.

**Pre-conditions.** Phase 32.

**TDD tests.**
1. `pushdown_into_transparent_function_body`.
2. `pushdown_blocked_by_opaque_provenance`.
3. `pushdown_blocked_at_black_box`.
4. `combined_rule_set_reaches_fixed_point`.

**Implementation.**
- `PushFilterIntoTransparentFunction` rule.
- Provenance-based safety check.
- Termination marker ensures no re-push.

**Example fixtures.** Unit tests. One model fixture asserts pushed-through SQL via the CLI compile path.

**Review checklist.** Rule skips pushdown when the filter uses a non-deterministic expression.

**Commit.** `planner: filter pushdown across transparent functions (Phase 33)`

### Phase 34 — Join elimination example (Step 7 complete)

**Goal.** Demonstrate the §12 join-elimination example end-to-end: a function that left-joins a dimension table, and a downstream consumer that uses no column from the dimension — planner elides the join.

**Pre-conditions.** Phase 33.

**TDD tests.**
1. `join_elimination_fires_when_no_downstream_consumer`.
2. `join_elimination_blocked_when_column_used`.
3. `join_elimination_requires_declared_cardinality` — 1:1 ⇒ safe; 1:N ⇒ blocked.

**Implementation.**
- `EliminateUnusedLeftJoin` rule scans the full DAG for column consumption.
- Requires declared `joins: { dim: { type: LEFT, cardinality: 1:1 } }` under the unstable flag.
- Unsoundness in the declaration is the author's problem — rule does not verify cardinality against data.

**Example fixtures.** `functions/enriched_order.sql` + `models/order_totals.sql` (consumes no `dim_customer` column). Manual smoke: `smelt compile --show-plan` and diff SQL.

**Review checklist.** Step 7 closure note in `docs/ROADMAP.md`. Soundness caveat documented (§20E).

**Commit.** `planner: join elimination rule with declared cardinality (Phase 34, Step 7 complete)`

---

## Step 8 — Phases 35 to 38

### Phase 35 — Parser + types: row variables on `Struct<…>` and value-level spread

**Goal.** Parse `Expr<Struct<{ts: Timestamp, ..r}>>` and value-level `..event` spread in struct-literal positions. Types represent row variables at the signature level.

**Pre-conditions.** Phase 27 (bidirectional machinery).

**TDD tests.**
1. `parses_struct_with_named_row_var`.
2. `parses_struct_with_anonymous_row_tail`.
3. `parses_struct_literal_spread_in_body`.
4. `two_named_row_vars_in_one_signature_errors` — v1 constraint.
5. `anonymous_tail_unreferenced_in_body_ok`.

**Implementation.**
- Extend struct type parser to recognise row-tail tokens.
- Extend expression parser to recognise `..name` in struct literals.
- `SmeltType::Struct { fields, tail }` where `tail = None | Anon | Named(String)`.

**Example fixtures.** `functions/event_hour.sql` (the §11 example — declaration only; body check lands Phase 36).

**Review checklist.** Parser stays independent of the type checker.

**Commit.** `parser+types: struct row variables and value-level spread (Phase 35, Step 8 opens)`

### Phase 36 — Row unification at call sites with value-level erasure

**Goal.** At a call with a row-polymorphic struct parameter, the checker unifies the declared field shape against the concrete argument and binds the row variable to the remainder. Value-level spread `..event` in the body erases to explicit field references at expansion.

**Pre-conditions.** Phase 35.

**TDD tests.**
1. `event_hour_types_clean`.
2. `struct_missing_declared_field_errors`.
3. `struct_extra_fields_bind_named_row_var`.
4. `spread_in_body_expands_to_explicit_fields` — verify generated SQL.
5. `row_var_unification_is_local` — no HM-style global solving; unit-test via a second call with a different remainder.

**Implementation.**
- Row unification in `unify_call`: declared fields matched first, remainder bound to row var.
- Expansion replaces `..event` with explicit field accesses in declaration order.

**Example fixtures.** Update `functions/event_hour.sql` with a body + a usage model.

**Review checklist.** Errors never mention `..r` in user-facing text (§9 guarantee).

**Commit.** `db: row unification and struct-literal erasure at expansion (Phase 36)`

### Phase 37 — Row variable in return position: pass-through fields

**Goal.** `with_hour(event: Expr<Struct<{ts, ..r}>>) -> Expr<Struct<{hour, ..r}>>` — the return type's `..r` is the same variable bound at the parameter. Expansion produces explicit field accesses for every field carried by `r`.

**Pre-conditions.** Phase 36.

**TDD tests.**
1. `with_hour_types_clean`.
2. `return_row_var_binds_to_param_row_var`.
3. `caller_sees_fully_resolved_return_type` — LSP hover shows concrete fields.
4. `expansion_emits_explicit_field_references`.

**Implementation.**
- Row-variable substitution in the return type using the parameter-position binding.
- Expansion uses the concrete schema to emit backend-appropriate struct literals (DuckDB first; Spark / Postgres via backend printer).

**Example fixtures.** `functions/with_hour.sql` + a usage model.

**Review checklist.** LSP hover returns concrete fields in declaration-first order.

**Commit.** `db+lsp: row variables in return types with concrete erasure (Phase 37)`

### Phase 38 — `smelt.as_struct()` revisit (Step 8 complete)

**Goal.** Deliver `smelt.as_struct(<alias> [EXCEPT <cols>])` per §16 #19, now backed by Phase 35's row-var machinery. Closes Step 8.

**Pre-conditions.** Phase 37.

**TDD tests.**
1. `as_struct_basic_emits_struct_literal`.
2. `as_struct_except_filters_columns`.
3. `as_struct_backend_printer_emits_duckdb_spark_postgres` — three-way backend-printer test.
4. `as_struct_in_multi_join_context_resolves_without_collision` — §6 Strategy 3 use case.
5. `backend_without_struct_literal_errors` — backend-capability error.

**Implementation.**
- New parser form `smelt.as_struct(ALIAS [EXCEPT COL_LIST])`.
- Backend printer dispatches on a `struct-literal` capability flag.
- `as_struct` compiles to a struct literal before planner rules see it.

**Example fixtures.** `functions/enrich_order_with_as_struct.sql` (the §6 example). Broken: `fn_as_struct_no_backend_literal.sql` against a mocked backend without struct-literal support.

**Review checklist.** `docs/ROADMAP.md` marks Step 8 complete and notes the full smelt-functions experimentation roadmap as delivered in v1. Update `docs/TODO.md` with follow-ups.

**Commit.** `parser+planner+docs: smelt.as_struct and Step 8 closure (Phase 38, Step 8 complete)`

---

## Critical files

**Parser.**
- `crates/smelt-parser/src/syntax_kind.rs` — new kinds.
- `crates/smelt-parser/src/parser.rs` — `parse_file` refactor, `parse_smelt_define`, `parse_smelt_fn_call`, `parse_smelt_extern`.
- `crates/smelt-parser/src/ast.rs` — wrappers `SmeltDefine`, `SmeltExtern`, `SmeltFnCall`, `Param`, `TypeRef`, `DefineBody`, `File::defines()`/`externs()`.
- `crates/smelt-parser/src/lib.rs` — `find_frontmatter_blocks` (Phase 11).

**Types.**
- `crates/smelt-types/src/signatures.rs` **(new)** — `SmeltType`, `TypeConstraint { Numeric, Ordered, Any }`, `Signature`, `BuiltinRegistry`, `FunctionSig`, `parse_smelt_type`, `unify_call`, `promote_types` helper.
- `crates/smelt-types/src/functions.rs` — re-export / thin shim for legacy callers while Phase 9 migrates.

**Database.**
- `crates/smelt-db/src/type_inference.rs` — extend `TypeContext` with `function_params`; route `infer_function_type` through the registry (Phase 9); frame-stack threading.
- `crates/smelt-db/src/lib.rs` — new queries (`functions_in_file`, `function_signature`, `function_body`, `resolve_function`, `check_function_body`, `function_backends`); extend `DiagnosticData` (`ExpansionFrames`); new `DiagnosticCode` values.

**LSP.**
- `crates/smelt-lsp/src/lib.rs::to_lsp_diagnostic` — frame rendering (Phase 6 single-level, Phase 12 full).

**Examples.**
- `examples/functions_demo/` **(new, Phase 1)** — green end-to-end workspace; extended each phase with fixtures listed under that phase's "Example fixtures" bullet. Registered with `cargo test -p smelt-cli --test example_diagnostics`.
- `examples/broken/models/fn_*.sql` — negative fixtures added per phase.

**Tests.**
- `crates/smelt-db/tests/function_registry.rs` (Phase 3, new).
- `crates/smelt-db/tests/function_body_check.rs` (Phase 5, new).
- `crates/smelt-db/tests/smelt_fn_call_check.rs` (Phase 6, new).
- `crates/smelt-cli/tests/broken_function_diagnostics.rs` **(new, Phase 6)** — asserts `DiagnosticCode` + message substring for every `examples/broken/models/fn_*.sql` fixture. Phases 7–38 append rows here rather than creating new test files.
- `crates/smelt-db/tests/tableexpr_body_check.rs` **(new, Phase 15)** — TableExpr parameter body-checking with row polymorphism and shadow warnings.
- `crates/smelt-db/tests/context_binding_check.rs` **(new, Phase 21)** — context-binding column validation and annotation reconciliation.
- `crates/smelt-db/tests/tier2_body_check.rs` **(new, Phase 23)** — Tier 2 body check in isolation.
- `crates/smelt-db/tests/tier2_tier1_inline.rs` **(new, Phase 26)** — Tier 2 → Tier 1 inline expansion.
- `crates/smelt-parser/src/parser.rs` unit tests (Phases 1, 2, 13, 19, 28, 35).

**Planner (new crate, Phase 30 onwards).**
- `crates/smelt-planner/` **(new, Phase 30)** — logical plan data model + rule API.
- `crates/smelt-planner/src/logical.rs` **(new, Phase 30)** — `Plan`, `LogicalNode::FunctionCall { fn_id, args, transparent, properties }`.
- `crates/smelt-planner/src/rules.rs` **(new, Phase 32)** — `PlannerRule` trait + fixed-point loop + `ExpandTransparentFunctionCalls`.
- `crates/smelt-planner/src/rules/pushdown.rs` **(new, Phase 33)** — `PushFilterIntoTransparentFunction`.
- `crates/smelt-planner/src/rules/join_elimination.rs` **(new, Phase 34)** — `EliminateUnusedLeftJoin`.
- `crates/smelt-planner/tests/` **(new)** — unit tests for each rule.

**Docs.**
- `docs/smelt-functions-upgrade-story.md` **(new, Phase 27)** — Tier 1 → Tier 2 migration guidance.
- `docs/ROADMAP.md` — closure notes at the end of Steps 3 (Phase 18), 4 (Phase 22), 5 (Phase 27), 6 (Phase 29), 7 (Phase 34), 8 (Phase 38).

**Example fixtures (grow per phase).**
- `examples/functions_demo/functions/add_margin.sql` (Phase 13, tightened Phase 16).
- `examples/functions_demo/functions/sessionize.sql` (Phase 17).
- `examples/functions_demo/functions/pred_demo.sql` (Phase 20).
- `examples/functions_demo/functions/session_rollup.sql` (Phase 22, block-syntax upgrade in Phase 29).
- `examples/functions_demo/functions/margin_tier2.sql` (Phase 23).
- `examples/functions_demo/functions/margin_tier3.sql` (Phase 24).
- `examples/functions_demo/functions/mixed_tier.sql` (Phase 26).
- `examples/functions_demo/functions/event_hour.sql` (Phase 35, body in Phase 36).
- `examples/functions_demo/functions/with_hour.sql` (Phase 37).
- `examples/functions_demo/functions/enrich_order_with_as_struct.sql` (Phase 38).
- `examples/functions_demo/functions/enriched_order.sql` + `models/order_totals.sql` (Phase 34, join-elimination demo).
- `examples/functions_demo/models/margin_report.sql` (Phase 15; upgraded Phase 25), `models/sessions_report.sql` (Phase 17), `models/margin_by_session.sql` (Phase 18), `models/rollup_dashboard.sql` (Phase 22), `models/rollup_with_passing.sql` (Phase 28/29).
- `examples/broken/models/fn_window_in_where.sql` (Phase 14), `fn_tableexpr_missing_col.sql` + `fn_tableexpr_shadow_warning.sql` (Phase 15), `fn_row_requirement_missing.sql` (Phase 16), `fn_cte_cycle.sql` (Phase 20), `fn_fragment_col_missing.sql` + `fn_annotation_too_wide.sql` (Phase 21), `fn_tier2_body_broken.sql` (Phase 23), `fn_tier3_return_mismatch.sql` (Phase 24), `fn_tier2_call_arg_wrong.sql` (Phase 25), `fn_tier2_calls_broken_tier1.sql` (Phase 26), `fn_passing_on_plain_call.sql` (Phase 28), `fn_as_struct_no_backend_literal.sql` (Phase 38).

## Verification

Each phase must, before handoff to its reviewer:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets` — zero warnings.
- `cargo test` — all passing.
- `cargo test -p smelt-cli --test example_diagnostics` — zero diagnostics in non-broken examples.

After Phase 12:
- Manual smoke: author a `safe_divide.sql` in `examples/`, call it from a model, verify LSP shows clean diagnostics and that a deliberate type error surfaces with a multi-frame trace.
- `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` — deeper oracle check before declaring Step 2 done.
- Update `docs/ROADMAP.md` marking Steps 1 & 2 complete with the date.

After Phase 18 (Step 3 complete):
- Manual smoke: open `examples/functions_demo/models/margin_by_session.sql` in VS Code with the LSP running; hover over the `source` parameter of `add_margin` and verify the tooltip shows the inferred `TableExpr<{revenue, cost, ..}>` schema with resolved column types. Introduce a bare-column typo in the body and confirm the error points at the typo, not the call site.

After Phase 22 (Step 4 complete):
- Manual smoke: author a model that calls `session_rollup` with a non-empty `metrics` fragment referencing `session_id` from the `sessionized` CTE context. Confirm the LSP rejects a reference to a column outside `sessionized` with a context-scoped `UnknownIdentifier`.

After Phase 27 (Step 5 complete):
- Manual smoke: upgrade a Tier 1 function to Tier 2 by adding parameter annotations; verify the migration guidance in `docs/smelt-functions-upgrade-story.md` matches the diagnostic the LSP shows for callers whose concrete arg types no longer satisfy the declared parameter types.

After Phase 29 (Step 6 complete):
- Manual smoke: `PASSING metrics AS (|)` — confirm LSP completion inside the block body lists the columns of the clause's inferred context. Confirm `PASSING` remains usable as a plain identifier outside call trailers.

After Phase 34 (Step 7 complete):
- Manual smoke: `smelt compile models/order_totals.sql --show-plan` (or equivalent CLI dump) and verify the printed logical plan shows the `LEFT JOIN dim_customer` eliminated when no downstream column references the dimension.

After Phase 38 (Step 8 complete):
- Manual smoke: author `functions/session_rollup.sql` and call it with a `PASSING metrics AS (...)` clause from a model; verify LSP completion works inside the clause and diagnostics are frame-stack-aware.
- `smelt compile models/order_totals.sql --show-plan` should demonstrate join elimination on the example from Phase 34. _(Phase 53 audit: this smoke step is now executable post-Phase 39, which wired the logical-plan rule pipeline into `smelt build --show-plan`; see Phase 39 note in cross-phase findings.)_
- `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` — final oracle check.
- Update `docs/ROADMAP.md` marking Steps 3–8 complete. Update the experimentation-roadmap section of the research doc to mark §19 Steps 3–8 complete with dates.

## Progress tracking

Updated as phases complete. Format: `Phase N — <title> — <status> (<commit sha>, <date>)`. New deferrals appended under "Deferred during implementation".

| Phase | Title | Status | Commit | Date |
|---|---|---|---|---|
| 1 | Parser: `smelt.define` top-level grammar | done | 996c27d | 2026-04-22 |
| 2 | Parser: `smelt.fn.*` call syntax | done | e3db6fb | 2026-04-22 |
| 3 | Salsa function signature index | done | 936233d | 2026-04-22 |
| 4 | `Expr<T>` type-reference resolution | done | 0bd42b7 | 2026-04-22 |
| 5 | Tier 1 body check with parameter binding | done | 05a96f4 | 2026-04-22 |
| 6 | Call-site expansion + single-level frame trace (Step 1 complete) | done | a25f44f | 2026-04-22 |
| 7 | `Ordered` constraint + canonical registry skeleton | done | 2cc7fa9 | 2026-04-23 |
| 8 | Generics + variadics | done | deb2403 | 2026-04-23 |
| 9 | Rewire built-in inference through the registry | done | 1567e51 | 2026-04-23 |
| 10 | `smelt.extern` declarations | done | f8641ef | 2026-04-23 |
| 11 | Per-declaration frontmatter + `backends:` + backend namespace | done | 3bedc86 | 2026-04-23 |
| 12 | Multi-level frame rendering + CAST-enforcement flag (Step 2 complete) | done | 2d2a1a8 | 2026-04-23 |
| 13 | Parser: TableExpr / WindowExpr / SelectItems<K, ctx> in type refs (Step 3 opens) | done | 3e69c98 | 2026-04-23 |
| 14 | Types: WindowExpr sort and SelectItems<K> kind ceiling | done | 80553d1 | 2026-04-23 |
| 15 | TableExpr parameters: bare-column row polymorphism + shadow warnings | done | 85a9441 | 2026-04-24 |
| 16 | Row-requirement annotations: TableExpr<{…}> pre-expansion checking | done | 38609e5 | 2026-04-24 |
| 17 | `sessionize` end-to-end: TableExpr + WindowExpr in body | done | 9ccd605 | 2026-04-24 |
| 18 | LSP polish + examples (Step 3 complete) | done | 0c7abfd | 2026-04-24 |
| 19 | Parser + types: context-binding syntax (Step 4 opens) | done | 76cb0bd | 2026-04-24 |
| 20 | CTE schema extraction + splice-point context inference | done | a4d64c9 | 2026-04-24 |
| 21 | Column-name validation against contexts + annotation reconciliation | done | 40b00b9 | 2026-04-24 |
| 22 | `session_rollup` end-to-end (Step 4 complete) | done | f09eeb4 | 2026-04-24 |
| 23 | Tier 2 body check in isolation (Step 5 opens) | done | ee3fcaa | 2026-04-24 |
| 24 | Tier 3 return-type verification + LSP hover | done | ac49e57 | 2026-04-24 |
| 25 | Call-site bidirectional checking (pre-expansion) | done | 9928016 | 2026-04-24 |
| 26 | Tier 2 → Tier 1 inline expansion | done | 21fb270 | 2026-04-24 |
| 27 | Generics ↔ checking-mode interaction (Step 5 complete) | done | 8f61b94 | 2026-04-24 |
| 28 | Parser: context-sensitive `PASSING` clauses (Step 6 opens) | done | edb9c26 | 2026-04-24 |
| 29 | PASSING binding to fragment-sort params + LSP completion (Step 6 complete) | done | 4d4ce8a | 2026-04-24 |
| 30 | Logical plan data model: functions as first-class nodes (Step 7 opens) | done | a3a3150 | 2026-04-25 |
| 31 | Column provenance + declared-property propagation | done | 9cb6709 | 2026-04-25 |
| 32 | Planner rule API + Level 2 expansion of function calls | done | 3715013 | 2026-04-25 |
| 33 | Filter pushdown across transparent-function boundaries | done | 677f2e3 | 2026-04-25 |
| 34 | Join elimination example (Step 7 complete) | done | 35b125f | 2026-04-25 |
| 35 | Parser + types: row variables on `Struct<…>` and value-level spread (Step 8 opens) | done | 17b6c8f | 2026-04-25 |
| 36 | Row unification at call sites with value-level erasure | done | 836925e | 2026-04-25 |
| 37 | Row variable in return position: pass-through fields | done | 4a5d97b | 2026-04-25 |
| 38 | `smelt.as_struct()` revisit (Step 8 complete) | done | 143f3fe | 2026-04-25 |

### Deferred during implementation

- **Phase 17 — test 4 reframed to use `source.ts` body reference instead of `smelt.ref('events').event_time` arg** (2026-04-24). The plan's literal phrasing required member-access on a `smelt.ref()` call which isn't currently parseable, and bare-identifier args to a FROM-position `smelt.fn.*` call have no scope until Phase 19's context binding. The reframed test still asserts the Phase 17 contract — missing column on `source` surfaces as `UnknownIdentifier` with `ExpansionFrames` rooted at the callee. Proper arg-position column resolution follows in Phase 19+.
- **Phase 17 — default-value provenance tagged via `(default)` suffix on frame binding type string** (2026-04-24). The plan suggested a `Synthesized` marker; extending `FrameInfo` would ripple through the LSP renderer's Phase 12 surface. Minimal-surface-change approach: append `(default)` to the bound-type string in the frame, so the LSP trailer shows "in expansion of `sessionize`, `gap` was bound to Interval (default)". Upgrade to a structured flag if downstream phases need to discriminate programmatically.
- **Phase 17 — bare-identifier args resolving against TableExpr source schema rolled back** (2026-04-24). Over-eagerly flagged identifiers that the outer SELECT's other FROM entries would supply. Proper argument-scope resolution requires Phase 19's context binding.
- **Phase 16 — `RowTail::None` accepts extras (open-record semantics)** (2026-04-24). The Phase 16 plan's TDD test 1 (`row_requirement_satisfied_by_superset_schema`) asserts zero diagnostics when a caller supplies extra columns to a requirement with no tail written — `TableExpr<{revenue: Numeric, cost: Numeric}>`. Resolved toward research §8 "open-record" semantics: extras are always accepted; only `MissingColumn` and `TypeMismatch` are structural failures. `RowTail::Named` still captures extras into `row_var_env` (the observable difference). No `UnexpectedColumns` diagnostic exists in Phase 16; a future strict-schema switch can add it.
- **Phase 16 — `SchemaRequirement` built from CST, not re-parsed from string** (2026-04-24). Phase 13 already emits `ROW_REQUIREMENT` CST nodes; a CST-aware helper `tableexpr_type_from_cst` overrides `parse_smelt_type`'s string-path `UnsupportedSort` for annotated `TableExpr<{…}>`. Keeps the string-level parser unchanged as a best-effort fallback while the structured form is always accurate via CST.
- **Phase 16 — `row_var_env` binding recorded but opaque to user code** (2026-04-24). Named row tails (`..r`) write into the per-call `row_var_env` at the call site. `r` is not yet referenceable from body expressions or return types — that is Phase 17 (return-schema inference) and Phase 37 (row variables in return types). Unit tests exercise the binding via `#[doc(hidden)] pub` accessors.
- **Phase 15 — `add_margin.sql` temporarily downgraded to bare `TableExpr`** (2026-04-24). Phase 13's CST-only fixture used `TableExpr<{revenue: Numeric, cost: Numeric}>`, but Phase 15 scope is bare `TableExpr` (row requirements are Phase 16). The fixture was simplified to bare `TableExpr` so it type-checks end-to-end under Phase 15's semantics; Phase 16 re-tightens per its own "Example fixtures" instructions.
- **Phase 15 — SELECT-shaped function bodies** (2026-04-24). Phase 5's `walk_body` only walks `Expression` bodies; `TableExpr`-returning functions parse as `DEFINE_BODY → SELECT_STMT`. Added `BodyShape { Expression, Select }` discriminator plus `check_function_select_body` reusing `walk_select_columns_with_visitor`. No new abstractions — existing machinery is now branched on body shape.
- **Phase 15 — `parse_table_ref` now recognises `smelt.fn.<path>(...)` in FROM position** (2026-04-24). Required so Phase 15's end-to-end fixture can call `TableExpr`-returning functions. Parser-only change mirroring the existing `smelt.ref(...)` / `smelt.source(...)` triggers in FROM. No type-checker behaviour change.
- **Phase 15 — `tableexpr_schema_lookup` closure resolves only `smelt.ref('X')` / `smelt.source('a.b')` arguments** (2026-04-24). Other call-site argument shapes (inline subqueries, etc.) leave the TableExpr parameter's FROM scope empty, which surfaces `UnknownIdentifier` at the bare column usage with the call site as the top frame. Broader arg-shape support (CTEs, derived tables as TableExpr arguments) deferred to later phases as their fixtures surface.
- **Phase 15 — `margin_report.sql` uses `SELECT *`** (2026-04-24). `TableExpr` return-schema inference is Phase 17; until then a caller that projects explicit columns from a `TableExpr` result has no typed schema to resolve them against. `SELECT *` sidesteps this cleanly and the fixture will tighten in Phase 17.
- **Phase 14 — §21 "keep or collapse `AggExpr<T>`" closed (keep)** (2026-04-23, resolved). The linear subkind chain `Scalar < Agg < Window` (realised in `smelt-types::signatures::subkind_of`) gives `AggExpr<T>` its runtime witness as `ExprKind::Agg`. Keeping `AggExpr<T>` as a distinct sort in type-reference position (Phase 13) plus the runtime `ExprKind` tag is consistent, so no collapse. §21 "keep or collapse AggExpr<T>" is now closed.
- **Phase 14 — `infer_expression_kind` parallel-walker gap**. The kind walker falls through to `Expr::cast(child)` iteration for unhandled shapes (array literal, struct literal, `ROW(...)`, `IN`, `EXISTS`). These node kinds are not themselves `Expr`-kinded in the CST, so sub-expression kinds nested inside — e.g. `ARRAY[SUM(x) OVER (...)]` — silently dropped to `Scalar`. No Phase 14 test exercises this. Revisit when kind synthesis lands for those shapes in a later phase.
- **Phase 25 — `ArgTypeMismatch` message format diverges from §8 spec** (2026-04-24). Phase 6 established the current format `"Argument … has type …, which does not satisfy parameter …"`. The §8 spec example shows `"expected X, got Y" / "= note: parameter declared as …"`. The information content is equivalent but the phrasing differs. Changing the format would touch every caller assertion; deferred to a dedicated diagnostic-polish pass (post-Step 5).
- **Phase 26 — `walk_body_with_ctx` replaces `check_function_body_with_expansion` in `check_smelt_fn_call`** (2026-04-24). The pre-existing call-site expansion path called `check_function_body_with_expansion(&sig, body, ...)` which internally re-seeded the TypeContext from the signature — overwriting the call-site-bound types with `Unknown` for Tier 1 unannotated params and suppressing type errors. Phase 26 adds `walk_body_with_ctx(body, &body_ctx, ...)` which uses the already-built call-site context directly. This is a correctness fix for all Tier 1 expansion, not just the Tier 2→Tier 1 path.
- **Phase 25 — `has_schema_param` exception for Tier 2/3 expansion** (2026-04-24). Functions with `TableExpr` or `SelectItems` parameters bypass the Phase 25 early-return even when Tier 2/3 — their bodies still need call-site expansion to seed caller-supplied column schemas. This is correct behaviour but means some Tier 2/3 functions still expand at call sites. A future phase could eliminate redundant body re-walks by checking whether the body's `UnknownIdentifier`/`FunctionBodyTypeMismatch` errors depend on runtime schema, but that is out of Phase 25 scope.
- **Phase 24 — LSP hover wiring deferred** (2026-04-24). ~~Resolved in Phase 48 (2026-04-26): `find_smelt_fn_call_at_cursor`, `passing_body_completion_columns`, and multi-level `render_expansion_frames` landed in `crates/smelt-lsp/src/lib.rs` commit 221d7d8; all 5 Phase 48 TDD tests pass.~~
- **Phase 29 — LSP cursor-in-PASSING-body column completion deferred** (2026-04-24). ~~Resolved in Phase 48 (2026-04-26): `passing_body_completion_columns` and `passing_body_aggregate_labels` helpers landed; `lsp_completion_in_passing_body_lists_context_columns` and `lsp_completion_in_passing_body_filters_by_kind` pass.~~
- **Phase 22 — `rollup_dashboard.sql` uses defaults for `metrics`/`filters`** (2026-04-24). The plan specifies "non-empty parenthesised `metrics` argument" but `PASSING` syntax (the inline block form for fragment parameters) is Step 6. The fixture uses the positional form with defaults omitted, exercising the empty-default path. Non-default fragment passing via `PASSING` lands in Phase 28–29.
- **Phase 22 — opaque-CTE suppression for `smelt.fn.*` wildcard bodies** (2026-04-24). When a CTE body is `SELECT * FROM smelt.fn.<path>(...)`, the CTE schema cannot be inferred without resolving the callee's return schema (a future mechanism). `TypeContext` gains `mark_cte_opaque()` so the type-checker returns `Unknown` for any column access against such a CTE, suppressing false-positive `UnknownIdentifier` errors. The full smelt-fn-return-schema inference in CTE bodies is deferred to a later phase as it requires cross-function schema propagation.
- **Phase 22 — `empty_default_metrics_splice_comma_elision` scoped to type-checker level** (2026-04-24). The plan test name implies SQL comma-elision; §16 #20 explicitly places that rule at Level 2 materialisation (Phase 32+). The test validates that the type-checker handles `SelectItems<Agg, ctx> = ()` without errors and that calling without `metrics` doesn't surface a diagnostic. SQL generation deferred to Phase 32.
- **Phase 39 — `--show-plan` hosted on `smelt build`, no separate `compile` subcommand** (2026-04-25). The CLI has no `compile` subcommand. The plan permits "or equivalent build/run command"; chose `smelt build` because it already takes a project root and supports a target, and the show-plan path early-returns before seeding/running so it never touches the database.
- **Phase 39 — rule order is `[PushFilter, ExpandTransparent, EliminateUnused]`, not the plan's literal `[ExpandTransparent, PushFilter, EliminateUnused]`** (2026-04-25). With the plan's order, expansion replaces `FunctionCall` with `ExpandedCall` first, leaving `PushFilterIntoTransparentFunction` (which only matches `FunctionCall`) with no remaining matches. Reorder is a correctness fix, not just perf; `combined_rule_set_reaches_fixed_point` in `crates/smelt-planner/tests/pushdown_tests.rs` already used this order. `show_plan_rules()`'s doc-comment captures the constraint.
- **Phase 39 — `LogicalNode::ExpandedCall` gained a `pushed_filter: Option<Plan>` field** (2026-04-25). Without preserving the filter through expansion, the only evidence that pushdown ran is erased and Phase 39 test #3 (`pushed_filter: Some(_)` on the call node) cannot pass. Field is load-bearing for the contract; not the body splice that Phase 41 lands.
- **Phase 39 — tests #3 and #4 use hand-built plans rather than driving SQL through the CLI** (2026-04-25). `build_logical_plan_pure` (`crates/smelt-db/src/lib.rs:3847`) walks `smelt.fn.*` calls only — `Select.filter` and `LeftJoin` nodes aren't yet constructed from raw SQL. End-to-end SQL → plan coverage waits on Phase 41–42's logical-plan-builder enrichment; modifying `smelt-db` is out of Phase 39's scope.
- **Phase 39 — `default_compile_unchanged` is a contract test, not a byte-for-byte snapshot** (2026-04-25). Snapshotting an entire `smelt build` invocation is brittle (timestamps, log config, DuckDB schema serialisation). The test pins the contract that matters: no plan output appears without the flag, and the existing `smelt: built N model(s)` summary still fires.
- **Phase 40 — `RuleContext` is no longer a unit struct; existing tests migrated to `RuleContext::default()`** (2026-04-25). Threading the `CanonicalReturnLookup` registry into rules required gaining a field. All Phase 32–34 tests in `crates/smelt-planner/tests/` were mechanically updated from `let ctx = RuleContext;` and `&RuleContext` to `RuleContext::default()` and `&RuleContext::default()`. The rule API stayed source-compatible at the `PlannerRule::apply` trait surface.
- **Phase 40 — `cast_emitted_for_needs_cast_returns` (Phase 32 test) updated to use `RuleContext::with_builtins()` and a registered fn name** (2026-04-25). Under Phase 40 semantics, Cast emission requires both `needs_cast: true` AND a registered canonical_return. The Phase 32 test used a synthetic `cast_fn` that isn't registered; switched to `SUM` which has `canonical_return: BigInt`. Same structural assertion (Cast wraps ExpandedCall), now exercising the actual registry-driven path rather than the removed BigInt placeholder.
- **Phase 40 — `sum_decimal_cast_target_is_decimal` accepts BigInt OR Decimal(38, 0)** (2026-04-25). Per §16 #9, Decimal precision/scale tracking is deferred for v1, and the registry encodes `canonical_return` once per signature (not per-arg-type). SUM's canonical_return is BigInt today; Decimal(38,0) is encoded as the duckdb `engine_native`. Test asserts the cast target matches either v1-acceptable widening; tightening to per-arg-type canonical resolution is a Phase 50 (registry expansion) concern.
- **Phase 41 — body subtree represented as `LogicalNode::Raw { sql_text }`** (2026-04-25). Phase 41 needs a `body: Plan` to splice, but smelt-db cannot yet lower an arbitrary SQL expression body to a structured `Select`/`Cast`/etc. plan. `LogicalNode::Raw { sql_text }` was added as a verbatim-text placeholder so the splice mechanics, provenance tagging, and cycle handling can be exercised end-to-end. Replacing `Raw` with structured plan nodes is a follow-up for the body lowering work (likely alongside Phase 46's TableExpr argument shapes or a dedicated phase).
- **Phase 41 — `ProvenanceTag::Caller` defined but unproduced** (2026-04-25). The plan's checklist item #6 says "every spliced node carries a `Caller` / `Callee(fn_id, ...)` tag (decision 12 model)". Phase 41's `splice_body` clones the callee body verbatim — there is no argument-substitution step, so no caller-side subtree to tag. The `Caller` enum variant exists in `ProvenanceTag` for forward compatibility; a future phase introducing real argument substitution will produce it. Test `provenance_preserved_through_splice` therefore only verifies the `Callee` half of the tag invariant.
- **Phase 41 — synthesised-cycle visited-set added in planner rule** (2026-04-25). Plan's "Cross-Step risks" guidance ("recursive expansion of nested transparent calls must stop at the cycle-detection boundary ... Use a per-pass visited-set keyed on FnId") is now honoured by `expand_recursive`/`build_expanded_call`'s threaded `HashSet<FnId>`. smelt-db's pre-pass remains the primary cycle defence (sets `body: None`); the planner-rule visited-set defends against synthesised plans / future direct callers that bypass smelt-db. Regression test `synthesised_self_referential_body_terminates` exercises this path.
- **Phase 41 — workspace cycle helpers became `#[salsa::tracked]`** (2026-04-25). `workspace_function_bodies`, `workspace_function_call_graph`, and `function_call_cycle_fn_ids` were promoted from plain orchestrators to tracked queries. Returns wrapped in `Arc<...>` per Salsa's interning requirement (matching `all_models`'s precedent in the same file). Closes the reviewer's perf concern that workspace-wide cycle/body computation was running uncached up to 4× per file.
- **Phase 41 — recursive DFS in `find_function_call_cycles` uses unbounded stack** (2026-04-25). For workspaces with very long deterministic call chains (`A → B → … → Z` with depth ≫ Rust's default ~8MB stack), the recursive DFS could overflow. Hardening to an iterative DFS deferred to a later phase if the huge-workspace stress test surfaces the limit.
- **Phase 42 — `as_struct` lowering relocated but not yet wired into SQL emission** (2026-04-25). Phase 42's "Goal" originally read "invoked during physical-plan emission for every `SMELT_AS_STRUCT_CALL` node"; that wiring requires structured-body lowering (replacing the Phase 41 `LogicalNode::Raw` placeholder with `Select`/`Cast`/etc.) and a SQL printer that walks expanded plans. Phase 42 instead delivers the *relocation* (`smelt-planner/src/lowering/as_struct.rs` is now canonical, smelt-db retains a `pub use` shim) plus the broadened capability gate against `project_active_backends`. End-to-end emission lands alongside Phase 46's TableExpr argument shapes or a dedicated body-lowering phase. Phase 42's Goal text was tightened in the plan and `docs/TODO.md` was split into `[x] relocate / [ ] wire` to reflect what shipped.
- **Phase 42 — TDD tests 1+2 verify the lowering helper directly, not through `smelt build`** (2026-04-25). Per the SQL-emission deferral above, `as_struct_lowering_emits_duckdb_struct_literal` and `as_struct_lowering_emits_spark_struct_constructor` exercise `smelt_planner::lowering::as_struct::as_struct_to_sql` directly. The plan's literal "compile a model … resulting SQL contains …" framing is satisfied at the unit level; full compile-path coverage waits on the body-lowering wiring.
- **Phase 42 — broken fixture for test 4 lives inline in the cli test rather than in `examples/broken/models/`** (2026-04-25). `crates/smelt-cli/tests/broken_function_diagnostics.rs::no_orphan_fn_fixtures` enforces every `examples/broken/models/fn_*.sql` fixture must produce a diagnostic under that test's harness, which uses `set_project_input(root, "")` (no `smelt.yml`). A default-backends fixture there could not exercise the active-backend gate (the gate fires only when `smelt.yml` is parseable). The fixture content lives inside `as_struct_capability_tests.rs` to preserve the orphan invariant while still hitting the Phase 42 path end-to-end.
- **Phase 42 — `parse_active_backends` extracted to `smelt-core::config`** (2026-04-25). Reviewer flagged that the initial implementation inlined YAML parsing inside the `project_active_backends` Salsa query; the canonical pattern in `crates/smelt-db/src/lib.rs` (e.g. `project_unstable_schema` → `smelt_core::parse_unstable_schema_flag`) is a thin tracked wrapper around a pure helper. Refactor extracts `pub fn parse_active_backends(text: &str) -> Option<Vec<String>>` to `smelt-core/src/config.rs` and reduces the Salsa query to a single-line wrapper.
- **Phase 47 — `mark_cte_opaque` retained as fallback rather than removed** (2026-04-26). The plan suggested deleting or `#[deprecated]`-marking the API once cross-function CTE inference lands. The implementation kept it: when `smelt_fn_schema_lookup` returns `None` (e.g. the inner `smelt.fn.X(...)` call's name doesn't resolve, the callee returns a non-TableExpr type, the call participates in a function-call cycle, or `infer_tableexpr_return_schema` fails for any other reason), `extract_function_body_cte_schemas` falls back to `ctx.mark_cte_opaque(cte_name)`. This keeps outer-SELECT column references quiet on a single error rather than cascading. Phase 22's deferral entry "opaque-CTE suppression for `smelt.fn.*` wildcard bodies" is now resolved as the dominant path; the marker remains as a graceful-degradation escape hatch.
- **Phase 47 — caching deferred** (2026-04-26). Plan called out reusing Phase 26's `DataTypeHash` for per-(callee fn_id, arg types) Salsa caching of return-schema inference. The shipped code uses `SalsaRefSchemaProvider::resolve_smelt_fn_call_schema` directly, which is uncached. No measurable property-test slowdown observed. Caching is a follow-up if the schema-inference path becomes a hot loop in larger workspaces.
- **Phase 47 — recursive resolution guarded by `function_call_cycle_fn_ids`** (2026-04-26). The new closure inside `SalsaRefSchemaProvider::resolve_smelt_fn_call_schema` recurses on chained calls (`SELECT * FROM smelt.fn.f(smelt.fn.g(...))`). To prevent infinite recursion on cyclic call graphs, the closure consults the workspace-level `function_call_cycle_fn_ids` Salsa query once and short-circuits with `None` when the inner call's tail-segment name is in the cycle set. The pre-existing recursion at `lib.rs:3100` (TableExpr-arg-as-smelt.fn.*) remains unguarded — flagged for a future hardening pass if synthesised cycles slip past the smelt-db pre-pass.
- **Phase 46 — test 2 reframed: SUBQUERY parser does not accept `AS alias` in expression position** (2026-04-26). The plan's literal test 2 (`tableexpr_arg_from_derived_table`) used `(SELECT … FROM y) AS d` as the function argument. The smelt parser only accepts `AS alias` after a parenthesised SELECT in *FROM-clause* position; in *expression* position (which is where a function argument lives) the trailing `AS d` parses as an unexpected keyword. Reframed to drop the trailing alias: the test still asserts that a derived-table subquery shape resolves correctly — the difference between tests 2 and 3 is now whether the inner SELECT has its own FROM clause or only literal projections. Both share the same `SUBQUERY` lookup path. If the parser is later widened to accept aliases on subqueries in expression position, the test name remains accurate.
- **Phase 46 — `is_clearly_non_table` scoped to bare literals only** (2026-04-26). Plan permitted broadening to BinaryExpr / UnaryExpr / CaseExpr at the implementer's discretion; the conservative landing detects only single-content-token expressions whose token is `NUMBER`, `STRING`, or `NULL_KW`. Booleans tokenize as `IDENT` (no `TRUE_KW`/`FALSE_KW` in the lexer today) and therefore fall through to the existing empty-schema path — acceptable since `smelt.fn.f(true)` would surface `UnknownIdentifier` inside the body, not a misleadingly-clean compile. Widening the detector is a follow-up if real fixtures hit confusing cases.
- **Phase 46 — Phase 15 `tableexpr_schema_lookup` deferral resolved (CTEs + derived tables + subqueries)** (2026-04-26). The Phase 15 deferral note "`tableexpr_schema_lookup` closure resolves only `smelt.ref('X')` / `smelt.source('a.b')` arguments" is now resolved for CTE references, derived tables, and inline subqueries. Phase 47 widens further (CTE-body return-schema inference for `WITH x AS (SELECT * FROM smelt.fn.*(…))`) which is the inverse direction.
- **Phase 45 — JOIN-alias seeding lifted ahead of `compute_shadow_warnings`** (2026-04-26). The shadow check at `function_body_check.rs:1148` runs before the body re-walk, but Phase 45's test 2 (`joined_alias_shadow_warning`) requires that an `Expr<T>` parameter colliding with a *joined* column also surface `ParameterShadowsColumn`. The implementer therefore calls `body_lookup` once to "preview" the body shape, runs `register_join_alias_schemas` against it, and only then runs the shadow check. The body re-walk later still resolves the body shape and reuses the already-seeded `body_ctx`. No double-seeding: `register_join_alias_schemas` is idempotent (`add_tableexpr_param` overwrites under the same key) and the body re-walk does not call it a second time. Worth noting because future phases that touch the call-site flow must preserve this preview-then-shadow ordering.
- **Phase 45 — `table_ref_schema_lookup` source-resolution duplicated across two closures** (2026-04-26). The free closure in `smelt_fn_call_diagnostics_for_file` (`crates/smelt-db/src/lib.rs:1825`) and `SalsaRefSchemaProvider::resolve_table_ref_schema` (`crates/smelt-db/src/lib.rs:2901`) share ~50 lines of `RefCall` / `SourceCall` / `sources_config` resolution. Both have to exist because the body-checker path takes a `&dyn Fn` closure (no Salsa imports allowed) while the return-schema-inference path lives inside `SalsaRefSchemaProvider` (which already owns the db handle). A future cleanup phase could consolidate by giving the closure a `SalsaRefSchemaProvider` instead of capturing `db`/`workspace` directly. Not blocking; flagged for the next pass that touches the body-checker plumbing.
- **Phase 44 — `monitored_session_rollup` carved into Phase 44b** (2026-04-26). The plan framed Phase 44 as fixture-only but research §10 needs two compiler capabilities the codebase does not yet have: (a) the parser rejects a bare `smelt.fn.X(...) PASSING ...` as a CTE body (it expects `SELECT`/`WITH`/`VALUES`), and (b) the type checker treats a bare reference to a fragment-typed parameter (`SelectItems<…>`) as a `Scalar`-kind column reference, so the inner kind check fails (`Argument for metrics ... must be Agg-kind or higher`) and `check_fragment_context_bindings` then complains that `metrics` is missing from the splice context. Both gaps are real type-system / parser work, not fixture polish. The `safe_divide` half (finding #9) lands under Phase 44; finding #8 moves to a new Phase 44b that adds the parser support + fragment-param kind inheritance + splice-column exemption, then ships the `monitored_session_rollup.sql` and `monitored_dashboard.sql` fixtures.
- **Phase 43 — frontmatter parse diagnostics decoupled from `unstable_schema` gate** (2026-04-26). The first implementer pass routed `FrontmatterParseError` emission through `provenance_unstable_diagnostics_for_file`, which has an early-return on `unstable_schema: true`. That meant any workspace opting into the unstable feature (e.g. `examples/functions_demo`) silently lost all malformed-YAML and unknown-key diagnostics. Reviewer flagged the gap. Fix: extract a new `frontmatter_parse_diagnostics_for_file(db, file) -> Vec<Diagnostic>` (no flag parameter), wire it into `check_file_diagnostics` parallel to the existing provenance helper, and add a `crates/smelt-db/tests/frontmatter_parse_diagnostics.rs` regression test under `unstable_schema: true` for both severities. The two surfaces stay separate: the unstable-schema helper continues to police the `provenance:` feature gate; the new helper polices syntactic parseability.
- **Phase 43 — `KNOWN_KEYS` allowlist for cross-pass frontmatter keys** (2026-04-26). The new serde_yaml-backed parser now warns on unknown top-level keys, but several existing fixture frontmatter blocks legitimately carry keys consumed by *other* passes (`backends:` for the active-backend gate, `incremental:` / `materialization:` / `tags:` for model materialization). To prevent green-workspace fixtures from regressing, `parse_function_properties` keeps an in-crate `KNOWN_KEYS` allowlist of these cross-pass keys; encountering them is silently skipped rather than warned. The list lives next to the unknown-key check in `crates/smelt-planner/src/logical.rs`. Shrink as those passes either move under this parser or get removed.
- **Phase 43 — `JoinSpec.cardinality` is a raw `String`, not the existing `Cardinality` enum** (2026-04-26). Phase 43 is parser-only — nothing consumes `joins:` yet. Mapping the raw string (`"1:1"`, `"1:N"`, …) to `smelt_planner::logical::Cardinality` is deferred to whatever phase first reads `FunctionProperties::joins` (Phase 51's provenance/joins validator is the natural home). Keeping the parser decoupled from the enum lets the v1 tolerant-skip policy stay in the parser without leaking validator decisions upstream.
- **Phase 8 broken fixtures → Phase 10** (2026-04-23, resolved). `examples/broken/models/fn_coalesce_text_int.sql` and `fn_greatest_no_args.sql` were originally slated for Phase 9. During Phase 9 implementation, reading `function_body_check::check_smelt_fn_call` (the only path that emits `ArgTypeMismatch` / `MissingArgument` for a `smelt.fn.*` call today) confirmed it only resolves user-declared functions via `ctx.lookup_function_signature`. Phase 9's `try_registry_inference` hook is a pure inference path and doesn't emit diagnostics — the rewire preserves `Unknown`-returning behaviour for coverage gaps, it doesn't spawn new error codes. The fix landed in Phase 10 via Option B: a new `builtin_lookup` closure on `check_smelt_fn_call` that dispatches built-ins through `unify_call` when the user-declared signature index misses, translating `UnificationError::{ConstraintViolation, MissingArgs, InconsistentBinding, EmptyVariadicTypeVar}` into `ArgTypeMismatch` / `MissingArgument` diagnostics. Both fixtures and their `broken_function_diagnostics.rs` CASES rows now live under Phase 10's coverage — `smelt.fn.COALESCE('x', 1)` surfaces `ArgTypeMismatch` (via `InconsistentBinding`) and `smelt.fn.GREATEST()` surfaces `MissingArgument` (via `EmptyVariadicTypeVar`, mapped to "variadic requires at least one argument").

---

## Review: implementation vs. research / plan (2026-04-25)

This review compares the shipped Phase 1–38 implementation against the research doc (`docs/research/20260413-smelt-functions.md`) and the plan above. All 38 phases are marked `done`. The review focuses on what is actually wired end-to-end, what is plumbed but inert, and what the research promised that never materialised. Findings are bucketed by severity, then by area.

### Headline assessment

The type-system half of the design landed cleanly: fragment sorts (`Expr<T>`, `AggExpr<T>`, `WindowExpr<T>`, `TableExpr<{…}>`, `SelectItems<K, ctx>`, `Struct<{…, ..r}>`), three-tier checking, frame-stack diagnostics, generics, variadics, bidirectional inference with expected-return, row variables (struct + table), context bindings, PASSING binding, and the canonical signature registry skeleton are all real and tested. The pure-function rule held — `type_inference.rs` has no Salsa imports, and the new `function_body_check.rs`, `signatures.rs`, and `smelt-planner` crates obey the same discipline.

The planner half is the weak link. Phases 30–34 produced a logical-plan data model, three rules, and Salsa wiring (`logical_plan(workspace, file)` at `crates/smelt-db/src/lib.rs:3742`), but **none of the rules are reachable from any user-facing CLI surface or codegen path**. Several Phase 32–38 deliverables are present as code but inert: CAST emission uses a placeholder type, `smelt.as_struct` has a backend printer that nothing calls, list-splice comma elision is not implemented anywhere, and the `--show-plan` smoke test referenced in this plan's Verification section does not exist as a CLI flag. The "Steps 1–5 are real, Steps 7–8 are scaffolding" framing is accurate; the plan's `done` markers somewhat overstate Step 7–8 completeness.

### Severity 1 — shipped-but-inert (planner side)

These items are marked `done` in the progress table but the production code path doesn't exercise them. They will silently regress without anyone noticing, because the only thing exercising them is unit tests inside `smelt-planner`.

1. **CAST emission target type is hardcoded `BigInt`** (Phase 32). `crates/smelt-planner/src/logical_plan_rules.rs:191` wraps an `ExpandedCall` with `LogicalNode::Cast { target_type: DataType::BigInt }` whenever `properties.needs_cast` is true, with the comment "Phase 32: use BigInt as a placeholder target type. Phase 33+ will resolve the actual return type from the function registry." Phase 33 added pushdown and never came back to this; Phase 34–38 also did not. The `Signature` struct already carries `canonical_return: DataType` plus `engine_native: HashMap<BackendId, DataType>` (`crates/smelt-types/src/signatures.rs` ~line 1518), so the data is there — the rule just needs to read it.

2. **The new logical-plan rule pipeline is not wired into compile**. `apply_rules_to_fixed_point` (`crates/smelt-planner/src/logical_plan_rules.rs:62`) and the `logical_plan` Salsa query exist, but `crates/smelt-cli/src/commands/` has no `--show-plan` flag and no call site grepping for `apply_rules_to_fixed_point` outside tests. The legacy `Planner` struct in `crates/smelt-planner/src/rules/mod.rs` (with `cube_split` / `incremental` / `python` rules over a `ModelGraph`) is the one the CLI runs. The new pipeline runs only inside `crates/smelt-planner/tests/*.rs`. The Phase 34 verification step "`smelt compile models/order_totals.sql --show-plan`" is therefore not executable as written.

3. **`ExpandedCall` is a marker, not an expansion**. `LogicalNode::ExpandedCall { fn_id, provenance, properties }` records that expansion *should* happen, but the rule does not splice in the callee's body subtree. The "Phase 32 ships expansion of function calls into their body subtree, with provenance preserved" goal is partially met — provenance is preserved, but the body subtree is not. Codegen emission of expanded SQL (Phase 32 review checklist line: "Rule API is ergonomically close to `docs/planner_rule_api_design.md`") cannot run because there is no body subtree to print.

4. **`smelt.as_struct` backend printer is unreachable** (Phase 38). `as_struct_to_sql` lives at `crates/smelt-db/src/function_body_check.rs:2511` as a pure utility, and `as_struct_backend_diagnostics_for_file` (`crates/smelt-db/src/lib.rs:1257`) does the capability check. But no caller invokes `as_struct_to_sql` during physical-plan emission — the function is dead code outside its own unit tests. `docs/TODO.md` already records this as "Planner-time as_struct expansion".

5. **`smelt.as_struct` capability gate misses functions with default backends**. `as_struct_backend_diagnostics_for_file` (`crates/smelt-db/src/lib.rs:1280`) only walks function bodies whose `BackendSet` is `Only(names)`. Functions with no `backends:` frontmatter resolve to `BackendSet::All` (`crates/smelt-db/src/backends.rs:39, :95, :142, :144`) and are skipped, so a function using `smelt.as_struct` without an explicit backends declaration silently passes regardless of the deployment target's struct-literal capability. Phase 38 TDD test `backend_without_struct_literal_errors` is therefore tighter than the runtime check.

6. **List-splice comma elision is unimplemented anywhere**. Decision 20 places this rule at Level 2 materialisation (Phase 32+); the Phase 22 deferral note ("`empty_default_metrics_splice_comma_elision` scoped to type-checker level") promised SQL emission would land in Phase 32. It did not. Grep for `comma_elide`, `comma_elision`, `splice` in `smelt-planner` shows no matches. `metrics = ()` callers compile only because `ExpandedCall` is a marker and never lowers to SQL through this path.

7. **Per-function structured frontmatter (`provenance`, `joins`) parser is line-based, not YAML**. `parse_function_properties` (`crates/smelt-planner/src/logical.rs:212`) walks lines using `strip_prefix("provenance:")` etc. — fine for `deterministic: true`, fragile for the structured `provenance: { margin: [source.revenue, source.cost] }` and the multi-line `joins:` block in `enriched_order.sql`. Multi-line YAML maps and any indentation other than the one on the test fixtures will silently miss. This is the most likely cross-phase regression vector if anyone touches the example fixtures.

### Severity 2 — research surface that never materialised

Things the research called out as canonical end-to-end demonstrations and the plan did not redirect, but are missing from `examples/functions_demo/`.

8. **`monitored_session_rollup` (research §10) was never created**. This is the only example in the research that exercises *block-syntax composition* — a function declared with fragment-sort parameters (`metrics: SelectItems<Agg>`, `alerts: SelectItems<Agg, base>`) that internally calls another function and forwards a PASSING fragment (`PASSING metrics AS (metrics)`). Without it, "Blocks compose" is asserted but not exercised end-to-end. Phase 29's `rollup_with_passing.sql` covers a single PASSING use, not the compositional case. Recommend: add `examples/functions_demo/functions/monitored_session_rollup.sql` as a regression fixture for Step 6 closure.

9. **`safe_divide` example fixture diverges from research §3**. `examples/functions_demo/functions/safe_divide.sql` is the canonical Step 1 deliverable; the body in the fixture is `CASE WHEN denominator = 0 THEN NULL ELSE CAST(numerator AS DOUBLE) / denominator END`. The research-spec body is `CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE) END`. Two divergences: (a) missing `OR denominator IS NULL` guard (a real correctness gap on inputs where DuckDB returns NULL), (b) only one CAST instead of two (an integer-division precision footgun on engines that don't auto-widen). The fixture is the headline example readers will copy; recommend tightening it back to the spec.

10. **`enriched_order` (Phase 34) is a stub for join elimination**. `examples/functions_demo/functions/enriched_order.sql` declares `provenance: { customer_name: [dim_customer.customer_name], customer_tier: [dim_customer.customer_tier] }` and a `joins:` block, but the body emits `CAST(NULL AS VARCHAR) AS customer_name, CAST(NULL AS VARCHAR) AS customer_tier` because (per the inline comment) "JOIN aliases are not yet tracked in the function body check scope". The function declares it consumes from `dim_customer` but its body cannot reference the join. This is a soundness hole if a planner rule ever trusts the declared provenance — the rule would push filters that mention columns the body never reads. The Phase 34 join-elimination unit tests work because they synthesise `LeftJoin` plan nodes directly, bypassing the smelt-define body. The end-to-end `examples/functions_demo/models/order_totals.sql` model that Phase 34's verification calls out cannot demonstrate join elimination on this fixture; it can only assert that the rule fires on a hand-built plan.

11. **`smelt.extern` with fragment-sort parameters is not exercised**. Decision 18 explicitly defers PASSING-after-extern but the broader question — can an extern declare a `SelectItems` parameter at all? — has no fixture either way. `examples/functions_demo/functions/externs.sql` only declares `Expr<T>`/`TableExpr` externs. If the answer is "v1 externs are scalar-only" that should be encoded as a parser-level reject with a `broken/models/fn_extern_fragment_param.sql` fixture; today it is silently accepted at parse time and behaviour at the call site is untested.

12. **Parameterized models (research §4 "Parameterized Models") are entirely deferred**. The research positions `smelt.ref('orders', default => smelt.source('us_orders'))` as one of the unified-model payoffs. There is no fixture, no parser path for the `default =>` named-arg on `smelt.ref`, and no plan phase for it. This is fine — §18 lists it under "Open Questions / Unified Model" — but the plan's Scope section does not name it as deferred, so it is invisible to a reader who only reads the plan.

13. **`smelt.metric()` was correctly held out of scope** (decision 6) and is not addressed; this is consistent with the research and plan, just noting for completeness that no regression has crept in here.

### Severity 3 — LSP completion / hover deferrals

Already documented under "Deferred during implementation" but worth surfacing because they affect the human review experience of every prior phase.

14. **Phase 24 LSP hover wiring deferred**. Pure helper `declared_return_hover_text(sig)` is implemented and unit-tested; the LSP `hover()` handler does not call it on `smelt.fn.*` call sites. Net effect: Tier 3 functions advertise return types that hover never displays.

15. **Phase 29 PASSING-body completion deferred**. Same shape: type-checking inside PASSING bodies is correct (tests 1–4 pass), but cursor-in-PASSING-body column completion (test 5) is not wired. The research §10 user value proposition ("LSP completion inside the block body lists the columns of the clause's inferred context") is therefore untested in the LSP e2e harness.

16. **Phase 12 multi-level renderer is single-level in disguise**. `render_expansion_frames` (`crates/smelt-lsp/src/lib.rs:819`) walks the full frame vector and emits `relatedInformation`, but the diagnostic *message* itself is the innermost frame — the outer frames live in `relatedInformation` only. Editors that don't surface `relatedInformation` (some terminals, simple LSP clients) see a single-level trace. This is an editor-coverage regression, not a code regression — the data is there. Worth flagging because §16 #16 Step 2 promised "every frame contributes a 'in expansion of `fn`, parameter `p` was bound to ...' line, call-site first" in the rendered error, not in side-channel data.

### Severity 3 — registry / type-system thinness

17. **Built-in registry seeds ~30 entries; production SQL needs many more**. `crates/smelt-types/src/signatures.rs:2102` populates aggregates (SUM, AVG, MIN, MAX, COUNT), windows (ROW_NUMBER, RANK, DENSE_RANK, LAG, LEAD), null/coalesce (COALESCE, GREATEST, LEAST, NULLIF, IFNULL), arithmetic (ABS, POWER, SQRT, LOG, LN, ROUND, CEIL, FLOOR), text (LOWER, UPPER, LENGTH, SUBSTRING, TRIM, CONCAT), and date/time (DATE_TRUNC, EXTRACT, DATE, NOW, CURRENT_DATE, CURRENT_TIMESTAMP). Notably absent: `LIKE`, `ILIKE`, `IS NULL`, `BETWEEN`, `IN`, `EXISTS` (operators — handled by parser primitives, but still need typed signatures), `STRING_AGG`/`LISTAGG`, `ARRAY_AGG`, `MEDIAN`, `STDDEV`, `VARIANCE`, `DATE_ADD`/`DATE_SUB`, `JSON_*` family, `NTILE`, `FIRST_VALUE`, `LAST_VALUE`, `CAST` (operator). The legacy `crates/smelt-types/src/functions.rs` (773 lines, ~150 functions across categories) was left in place rather than migrated — the registry is the *new* canonical surface. Plan note: Phase 9 was supposed to "Replace the hand-written match at `type_inference.rs:541` with a registry lookup" while preserving behaviour; the actual implementation kept the legacy match as a fallback, so a built-in absent from the registry still types via the legacy path. This is fine for correctness but means the registry isn't actually canonical — Phase 9's headline goal is partially achieved.

18. **`Decimal` precision/scale tracking is deferred (decision 9) but the divergence with DuckDB is now observable**. `docs/TODO.md` line "ABS(Decimal) returns Double — pre-existing divergence; add to `divergences.rs` registry or fix type inference" — this is the first user-visible consequence of the v1 deferral. Either fix or document in `docs/research/20260413-smelt-functions.md` §16 #9 deferred items.

19. **No `Predicate` sort and no `WindowInScalarContext` analogue for `WHERE EXISTS (... ORDER BY rownum())`**. `WindowInScalarContext` (Phase 14) catches `WHERE ROW_NUMBER() OVER (...) > 1` directly. It does not catch a window function buried inside a scalar subquery in WHERE. This is a real SQL footgun; the research lists it as a top-3 SQL error class. Worth a follow-up phase if the kind discipline is meant to live up to the research.

### Severity 3 — context / scoping edge cases

20. **TableExpr argument resolution covers only `smelt.ref()` / `smelt.source()`**. `crates/smelt-db/src/function_body_check.rs` resolves a TableExpr argument's schema by pattern-matching `smelt.ref('X')` / `smelt.source('a.b')`. Other shapes — inline subqueries, CTEs, derived tables — leave the FROM scope empty and produce false-positive `UnknownIdentifier` at body bare-column references. This is recorded under "Phase 15 — `tableexpr_schema_lookup` closure" but is the most common cause a function caller will hit when a composition gets non-trivial. The end-to-end `add_margin → sessionize` pipeline in `models/margin_by_session.sql` works because each link uses `smelt.ref()`; any synthesised intermediate table will fail.

21. **Bare-column resolution from JOIN aliases inside a function body is unsupported**. The `enriched_order.sql` workaround (CAST(NULL ...) for join-supplied columns) is the visible symptom. The TableExpr body checker only knows the top-level FROM target's schema; aliased JOIN sources are not threaded into the body's lookup. Plan does not flag this and no broken fixture exists for it. Recommend a follow-up phase or at minimum a TODO entry.

22. **Cross-function CTE schema inference deferred**. Phase 22's "opaque-CTE suppression for `smelt.fn.*` wildcard bodies" silently weakens checking inside any function body that wraps a `smelt.fn.*` call in a CTE — which is exactly the `session_rollup` body shape. The function ships green, but a typo in a column reference inside the WHERE/GROUP BY of the outer SELECT is not caught. The deferral is documented; users will hit it the moment they write a CTE-heavy function.

### Severity 3 — soundness / consistency holes

23. **Provenance / joins frontmatter is unverified against the body** (research §20E "Property correctness is unverified"). `add_margin_with_provenance.sql` declares `provenance: { margin: [source.revenue, source.cost] }`; `enriched_order.sql` declares both `provenance` and `joins`. Nothing checks the declaration against the body's projection list or join graph. The unstable-schema flag is the only safety mechanism. This matches §20E's expectation but the plan does not flag it as a known soundness hole; future planner rules using these properties will inherit the trust.

24. **`provenance: Unknown` propagation — no diagnostic when a transparent function lacks provenance and is downstream of a pushdown candidate**. `PushFilterIntoTransparentFunction` correctly refuses to push when provenance is `Unknown`, but the user gets no signal that they've left optimization on the table. A `lint`-level diagnostic ("function `X` is transparent but has no declared provenance — filter pushdown will be skipped") would help adopters reason about why optimizer behaviour changes when annotations move.

25. **`smelt.extern` collisions are checked against built-ins; cross-file extern collisions are checked; built-in shadowing the same name in another file isn't tested**. The decision-21 deferred item "Cross-file name collision rules for externs declared in multiple files with the same name" is noted as resolved by Phase 10, but the symmetric case ("two files each declare `smelt.extern foo` — both at the same level") needs a fixture. Phase 10 has `fn_extern_duplicate.sql` plus `_other.sql` — good. There's no negative fixture for "two files each declare a different `smelt.extern foo` aimed at different backends" — the canonical multi-backend extern split.

### Severity 3 — plan-document drift

26. **Verification section references CLI flags that don't exist**. `--show-plan` (Phase 34, Phase 38), and the Phase 38 manual smoke "`smelt compile models/order_totals.sql --show-plan` should demonstrate join elimination" — the flag is unimplemented. Either (a) implement the flag as a closing task, or (b) delete the smoke step and replace with a unit-test reference inside `crates/smelt-planner/tests/join_elimination_tests.rs`.

27. **Progress table has two rows with empty commit SHA** despite status `done`: Phase 13 (2026-04-23) and Phase 34 / Phase 37 (2026-04-25). The plan's own execution prompt mandates filling in the SHA at phase commit; the gap suggests these commits were squashed or that the plan-record commit dropped before the implementation commit. Recommend filling these in for traceability — `git log --oneline | grep -i "Phase 13\|Phase 34\|Phase 37"` should pin them.

28. **Plan's "Cross-phase risks" section flagged risks that materialised silently**. "Planner-rule fixed point on transparent functions (Step 7). Phase 33's first rewrite pushes filters across `LogicalNode::FunctionCall { transparent: true, .. }`. If the rule doesn't terminate ... Termination is guaranteed by an "already-pushed" marker in `Context`, tested explicitly." The implementation uses `pushed_filter.is_some()` on the node itself (`logical_plan_rules.rs:268`) — equivalent in effect, but the `RuleContext` type is a unit struct with no fields (not the "Context" the plan promised). Termination is correct; the design comment is stale.

### Recommendations (priority order)

If this review prompts a follow-up phase, the most-leverage items are:

R1. **Wire the new logical-plan rule pipeline into `smelt compile`** with `--show-plan` (Severity 1 #2). Without this, every Phase 30–34 deliverable is exercised only by tests in one crate. The most mechanical single change with the largest visibility win — and it unblocks the Phase 34 / 38 verification smoke that the plan promised but cannot run today.

R2. **Resolve the CAST emission target type from `Signature::canonical_return`** (Severity 1 #1). One file, one rule; the data is already on the registry entry.

R3. **Add `monitored_session_rollup.sql` and tighten `safe_divide.sql`** (Severity 2 #8, #9). Cheap, restores fidelity to the research's headline examples, and catches regressions in PASSING composition that nothing else exercises.

R4. **Either implement `smelt.as_struct` lowering or downgrade Phase 38 to "type-checked, lowering deferred"** (Severity 1 #4, #5). The current state — parsed, type-checked, capability-gated for one slice of cases, never emitted — is the worst-of-three-worlds. `docs/TODO.md` already records the follow-up; consider promoting it from TODO to a Phase 38 review-fix commit.

R5. **Add a `lint`-level diagnostic when a transparent function lacks declared provenance and a pushdown candidate sits above it** (Severity 3 #24). Closes the discoverability gap on the §20E soundness story.

R6. **Replace the line-based YAML walker in `parse_function_properties` with the existing serde_yaml stack** (Severity 1 #7). The `provenance:` and `joins:` blocks in fixtures are already structured YAML; one mis-indentation away from a silent miss.

R7. **Track the deferred LSP hover (#14) and PASSING completion (#15) under a single follow-up phase** rather than as scattered "Deferred during implementation" bullets. Both share the same cursor-in-CST infrastructure; landing them together is cheaper than landing them separately.

R8. **Resolve the empty-SHA progress rows** (Severity 3 #27) so the plan can be replayed against `git log` for audit.

### What this review does NOT change

- The §16 24 decisions are not re-opened. Every decision is honoured by the implementation as far as the code shows.
- The pure-function rule held across all 38 phases. `type_inference.rs`, `function_body_check.rs`, `signatures.rs`, and `smelt-planner/logical*.rs` are Salsa-free; Salsa queries wrap them as designed.
- The fragment-sort thesis (research §2) is validated by the type-checker test surface — `WindowInScalarContext`, `ParameterShadowsColumn`, kind-ceiling checks, row-variable unification all fire as the research predicted.
- The three-tier gradual typing model works end-to-end — Tier 2 isolation, Tier 3 hover (pure form), Tier 2→Tier 1 inline expansion — modulo the LSP hover wiring deferral noted above.

The headline finding is narrow: **Steps 1–6 are real, Step 7 ships a rule API but no compile integration, and Step 8 ships parser+typer for `as_struct` but no SQL emission**. Closing R1, R2, R4 brings the planner half up to the type-system half.

---

## Steps 9–13 — Review remediation (Phases 39–53)

These phases close every Severity 1, 2, and 3 finding from the review section above. They are organised so each Step delivers one cohesive class of fix and ships green: each phase ends with `cargo fmt --all -- --check`, `cargo clippy --all-targets` (zero warnings), `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` clean.

The execution prompt at the top of this file applies unchanged: each phase is dispatched to a fresh implementer subagent with a self-contained brief built from that phase's section, then to a fresh reviewer subagent with the diff and the phase's review checklist. Iterate until clean, then record + commit + push to the tracking branch.

### Cross-Step design choices

| Decision | Choice | Rationale |
|---|---|---|
| New rule API integration | The new `apply_rules_to_fixed_point` (`crates/smelt-planner/src/logical_plan_rules.rs`) is the canonical compile-time rule pipeline. The legacy `Planner` struct (`crates/smelt-planner/src/rules/mod.rs`) stays as the runtime DAG orchestrator. Phase 39 introduces a new compile-side entry point rather than merging the two. | Merging would re-open every Phase 30–34 design choice; legacy rules operate on `ModelGraph`, new rules operate on `Plan`. Two pipelines, one shared crate. |
| Frontmatter YAML parser | Replace `parse_function_properties` line-walker with `serde_yaml`. The crate is already a dependency in `smelt-cli` (used for `smelt.yml`); add it to `smelt-planner`. | Structured properties (`provenance:`, `joins:`) are real YAML maps; line-walker silently misses any deviation from the test-fixture indentation. |
| JOIN-alias scoping in TableExpr bodies | Extend the body-checker's FROM-scope layer to record per-alias schemas, not just the top-level TableExpr's schema. Aliases come from `JOIN smelt.ref('X') AS y` and `JOIN source AS s` patterns. | The `enriched_order` workaround (`CAST(NULL AS VARCHAR)`) is the visible symptom; properly threading aliases removes the workaround and unblocks Phase 34's example. |
| TableExpr argument-shape resolution | Phase 46 widens the call-site closure to inline-subqueries and CTE references; full Tier 2 schema-propagation is still deferred. | Covers ~95% of real call sites; Tier 2 propagation is its own phase elsewhere. |
| Built-in registry expansion strategy | Phase 50 ships the operators (`LIKE`, `IN`, `BETWEEN`, `IS NULL`), the missing aggregates (`STRING_AGG`, `ARRAY_AGG`, `MEDIAN`, `STDDEV`/`VARIANCE`), and the missing window functions (`NTILE`, `FIRST_VALUE`, `LAST_VALUE`). JSON family is its own follow-up because of the `Json` type-tracking question. | Mirrors `docs/TODO.md`'s coverage map; matches what real production SQL needs first. |
| Multi-level frame rendering | Phase 49 prepends every frame's "in expansion of …" trailer into the diagnostic *message* (outer-most first). Existing `relatedInformation` payload remains a parallel surface for editors that consume it. | §16 #16 Step 2 promised in-message rendering; today it's only in side-channel data. |

### Cross-Step risks

- **Phase 39 wiring fan-out.** Hooking `apply_rules_to_fixed_point` into the compile path will surface every dormant correctness issue in Phases 30–34's data model. Expect at least one round of reviewer findings that pull in a Phase 30–34 callee fix; treat those as Phase 39 review-fixes, not new phases.
- **Phase 41 body-splice termination.** When `ExpandedCall` learns to splice the callee body, recursive expansion of nested transparent calls must stop at the cycle-detection boundary (§3 no-recursion guarantee, but Salsa cycles in the planner are a separate risk). Use a per-pass visited-set keyed on `FnId`.
- **Phase 50 registry coverage breaks property tests.** Adding signatures will route more SQL through the registry path (Phase 9's hybrid). Run `PROPTEST_CASES=1000` after this phase before declaring it done.
- **Phase 47 dropping opaque-CTE suppression.** When cross-function CTE inference lands, the existing `mark_cte_opaque` shortcut becomes dead code — but every existing test that depends on the silent `Unknown` for `SELECT * FROM smelt.fn.*` CTE bodies will start emitting diagnostics. Audit `session_rollup` and its callers explicitly.

---

## Step 9 — Planner integration & frontmatter hardening (Phases 39 to 43)

Closes Severity 1 findings #1–#7 from the review. By the end of Step 9, `smelt compile --show-plan` runs the new logical-plan rule pipeline end-to-end, CAST emission resolves to canonical-return types, transparent calls expand into body subtrees, list-splice comma elision works at lowering, `smelt.as_struct` emits backend SQL with a complete capability gate, and structured frontmatter (`provenance`, `joins`) parses through `serde_yaml`.

### Phase 39 — Wire logical-plan rules into `smelt compile`; add `--show-plan`

**Goal.** Add a `--show-plan` flag to `smelt compile` (or equivalent build/run command). When set, run `apply_rules_to_fixed_point` over the new logical plan from `smelt-db::logical_plan(workspace, file)` and print the result. Default behaviour without the flag is unchanged. Closes review findings #2 and #26.

**Pre-conditions.** Phase 30 (logical plan), Phase 32 (rule API).

**TDD tests.**
1. `cli_show_plan_prints_logical_plan` — run `smelt compile examples/functions_demo/models/uses_safe_divide.sql --show-plan` and assert the printed output contains a `FunctionCall` node for `safe_divide`.
2. `cli_show_plan_runs_expand_rule` — same, but assert the output contains an `ExpandedCall` node, demonstrating Phase 32's expansion fired.
3. `cli_show_plan_runs_pushdown_when_eligible` — model with `WHERE` over a transparent + deterministic call shows `pushed_filter: Some(_)` on the call node.
4. `cli_show_plan_eliminates_unused_join` — `models/order_totals.sql` shows the `LeftJoin` elided (Phase 34's E2E claim).
5. `default_compile_unchanged` — without `--show-plan`, generated SQL byte-for-byte equal to pre-Phase-39 output.

**Implementation.**
- New flag in `crates/smelt-cli/src/commands/build.rs` (or wherever `compile` lives) wired to a function in `crates/smelt-cli/src/lib.rs`.
- Build the rule list: `vec![Box::new(ExpandTransparentFunctionCalls), Box::new(PushFilterIntoTransparentFunction), Box::new(EliminateUnusedLeftJoin)]`. Run `apply_rules_to_fixed_point`, format the result with a debug printer (new `pub fn format_plan(&Plan) -> String` in `smelt-planner`).
- Emit the printed plan to stdout when `--show-plan` is set; gate by a new `--show-plan` clap field on the relevant subcommand.

**Example fixtures.** Reuses existing `models/uses_safe_divide.sql`, `models/order_totals.sql`. No new fixtures.

**Review checklist.** Default build path unchanged. Plan printer is deterministic across runs. The flag only affects compile/build subcommand, not run/test. The legacy `Planner` (model-graph-based) is not touched. `format_plan` lives in `smelt-planner`, not `smelt-cli` (keeps printer pure and reusable).

**Commit.** `cli+planner: wire logical-plan rules into compile with --show-plan (Phase 39, Step 9 opens)`

### Phase 40 — CAST emission resolves target type from `Signature::canonical_return`

**Goal.** Replace the hardcoded `target_type: DataType::BigInt` placeholder in `ExpandTransparentFunctionCalls::build_expanded_call` (`crates/smelt-planner/src/logical_plan_rules.rs:191`) with a lookup of the callee's `Signature::canonical_return`. Closes review finding #1.

**Pre-conditions.** Phase 39.

**TDD tests.**
1. `sum_integer_cast_target_is_bigint` — `SUM(Integer)` on DuckDB emits `Cast { target_type: BigInt }`.
2. `sum_decimal_cast_target_is_decimal` — registry's canonical for SUM(Decimal) (resolve per §16 #9 deferred — for v1, accept `Decimal { precision: 38, scale: 0 }`).
3. `non_cast_function_emits_no_cast_node` — `LOWER(Text)` has `needs_cast: false` and emits no `Cast` wrapper.
4. `unknown_signature_falls_back_to_no_cast` — extern with no `canonical_return` → no Cast node, no panic.

**Implementation.**
- Thread `&BuiltinRegistry` (or `Workspace + Salsa`) into the rule's `RuleContext`. Currently `RuleContext` is a unit struct (`logical_plan_rules.rs`); add a `registry: Arc<BuiltinRegistry>` field.
- `build_expanded_call` reads `registry.resolve(fn_id).and_then(|sig| sig.canonical_return.clone())`. If `Some(dt)`, wrap in `Cast { target_type: dt }`. If `None`, no Cast.
- Update the stale comment "Phase 32: use BigInt as a placeholder target type. Phase 33+ will resolve …" — mark resolved.

**Example fixtures.** No new fixtures; existing planner unit tests assert the new behaviour.

**Review checklist.** `RuleContext` change is backward-compatible at the trait level (downstream rules ignore the new field). `needs_cast: false` path unchanged. Stale design comment in `logical_plan_rules.rs:191` removed. Cross-phase risk in this plan's risks section ("CAST emission ... Phase 33+") marked resolved.

**Commit.** `planner: resolve CAST emission target type from canonical_return (Phase 40)`

### Phase 41 — `ExpandedCall` body splice + list-splice comma elision

**Goal.** `ExpandTransparentFunctionCalls` no longer produces a marker; it splices the callee's body subtree (cloned with provenance tags per §16 #12) into the call site. List-valued fragment-sort splices (`metrics: SelectItems<…> = ()` or omitted) elide adjacent commas at lowering (§16 #20). Closes review findings #3 and #6.

**Pre-conditions.** Phase 40.

**TDD tests.**
1. `expanded_call_contains_body_subtree` — assert the `ExpandedCall` node's child is the callee's parsed body, not just an `fn_id` marker.
2. `nested_transparent_calls_expand_recursively` — A calls B calls C; final plan has C's body inlined under A.
3. `cycle_detection_terminates_expansion` — synthesised cycle (call graph has a back-edge) produces a `FunctionCallCycle` diagnostic and stops expanding.
4. `empty_selectitems_default_elides_comma` — `metrics = ()` in `session_rollup` body's `SELECT user_col, ..., metrics FROM sessionized` lowers to `SELECT user_col, ... FROM sessionized` (comma after the last non-empty item is removed).
5. `non_empty_selectitems_keeps_commas` — `metrics = (COUNT(*) AS n)` lowers with the trailing column included.
6. `provenance_preserved_through_splice` — every spliced node carries a `Caller` / `Callee(fn_id, ...)` tag (decision 12 model).

**Implementation.**
- Extend `LogicalNode::ExpandedCall` to optionally carry a `body: Plan` field, or replace `ExpandedCall` with a richer spliced subtree. Decision: add `body: Plan` to keep diff small.
- Implement `splice_body(callee_body: Plan, args: &Args, provenance_tag: ProvenanceTag) -> Plan`. Substitute parameter placeholders with argument subtrees. Tag every cloned node.
- Comma elision: a new rule `ElideEmptySelectItemsSplices` runs after `ExpandTransparentFunctionCalls`. Walks `Select::projections` and removes `SelectItems` placeholder positions whose splice resolved to `()`.
- Cycle detection: pre-pass over the call graph in `smelt-db::logical_plan` (Salsa-cached). Cycle → `DiagnosticCode::FunctionCallCycle` + abort splice for that fn_id.

**Example fixtures.** No new SQL fixtures; existing `session_rollup` workflow exercises both empty-default and non-empty `metrics`. New broken fixture `examples/broken/models/fn_call_cycle_a.sql` + `_b.sql` for the cycle test.

**Review checklist.** Splice termination guaranteed by the cycle pre-pass. Provenance tags survive multi-level splicing. Comma elision rule is idempotent (running twice = running once).

**Commit.** `planner: expand transparent function bodies with splice + comma elision (Phase 41)`

### Phase 42 — `smelt.as_struct` lowering wired in + capability gate broadened

**Goal.** Relocate `as_struct_to_sql` (`crates/smelt-db/src/function_body_check.rs:2511`) to `smelt-planner/src/lowering/as_struct.rs` so it becomes the canonical lowering surface (smelt-db retains a `pub use` shim for backward compat). Broaden the backend-capability gate (`as_struct_backend_diagnostics_for_file`) to fire for functions with `BackendSet::All` against the workspace's *active* backends (not just `BackendSet::Only(names)`). Closes review finding #5 in full and review finding #4 partially; the end-to-end "invoked during physical-plan emission" half of finding #4 lands when body lowering replaces `LogicalNode::Raw` placeholders with structured `Select`/`Cast`/etc. nodes (alongside Phase 46's TableExpr argument shapes or a dedicated body-lowering phase).

**Pre-conditions.** Phase 41.

**TDD tests.**
1. `as_struct_lowering_emits_duckdb_struct_literal` — compile a model that calls a function using `smelt.as_struct(o EXCEPT customer_id)`; resulting SQL contains `{'order_id': o.order_id, 'total': o.total}` (DuckDB form).
2. `as_struct_lowering_emits_spark_struct_constructor` — same call, Spark backend, contains `struct(o.order_id AS order_id, o.total AS total)`.
3. `as_struct_unsupported_backend_errors` — backend without struct-literal capability + `smelt.as_struct` → `BackendCapabilityViolation` diagnostic.
4. `as_struct_default_backends_capability_check_fires` — function declared with no `backends:` frontmatter (`BackendSet::All`) using `smelt.as_struct` against a workspace whose active-backend set lacks struct-literal capability → diagnostic emitted at the function declaration.
5. `as_struct_with_explicit_backends_only_unchanged` — Phase 38's existing behaviour unchanged.

**Implementation.**
- Move `as_struct_to_sql` from `crates/smelt-db/src/function_body_check.rs` to `crates/smelt-planner/src/lowering/as_struct.rs`. Pure helper; takes a `SmeltAsStructCall` AST + concrete schema + backend.
- Wire into the physical-plan printer (the same Step 7 emission path that handles `LogicalNode::ExpandedCall`).
- Extend `as_struct_backend_diagnostics_for_file` to consult the workspace's active-backend set when the function's `BackendSet == All`. The active-backend set lives in `smelt.yml`; thread via Salsa.

**Example fixtures.** Existing `examples/functions_demo/functions/enrich_order_with_as_struct.sql` becomes a real lowering test (not just type-check). New `examples/broken/models/fn_as_struct_default_backends.sql` for the capability-check test.

**Review checklist.** `as_struct_to_sql` no longer lives under `function_body_check.rs` (it's a lowering concern, not a body-check concern). The capability check honours `smelt.yml`'s active backends. Existing Phase 38 tests still pass unchanged.

**Commit.** `planner+db: lower smelt.as_struct and broaden capability gate (Phase 42)`

### Phase 43 — Frontmatter YAML parser via `serde_yaml`

**Goal.** Replace `parse_function_properties` in `crates/smelt-planner/src/logical.rs:212` with a `serde_yaml`-backed parser. Structured `provenance:` and `joins:` properties parse correctly across indentation styles and multi-line maps. Closes review finding #7.

**Pre-conditions.** Phase 42.

**TDD tests.**
1. `parses_simple_boolean_properties` — `deterministic: true`, `idempotent: true`, etc. unchanged.
2. `parses_inline_provenance_map` — `provenance: { margin: [source.revenue, source.cost] }` parses to a structured map.
3. `parses_multi_line_provenance_map` — same content on multiple lines, varying indentation.
4. `parses_joins_block_with_nested_map` — the `enriched_order.sql` `joins:` shape (a list of `{ table, on, cardinality }` maps).
5. `malformed_yaml_emits_diagnostic_not_panic` — bad YAML inside frontmatter → `DiagnosticCode::FrontmatterParseError` at the frontmatter span.
6. `unknown_keys_warned_not_errored` — forward-compatible: `unknown_property: foo` produces a `Severity::Warning` not a hard error.

**Implementation.**
- Add `serde_yaml` to `smelt-planner/Cargo.toml`.
- Define a `#[derive(Deserialize)]` `RawFunctionProperties` struct mirroring the documented frontmatter schema. Convert to `FunctionProperties` after parsing.
- New `DiagnosticCode::FrontmatterParseError` (severity Error) and an `unknown_key` warning (Severity::Warning).
- Delete the line-walker. Existing call sites switch to the new parser transparently.

**Example fixtures.** Tighten `examples/functions_demo/functions/add_margin_with_provenance.sql` and `enriched_order.sql` — the existing fixtures already use structured YAML and become regression tests automatically. Broken: `examples/broken/models/fn_frontmatter_malformed.sql`, `fn_frontmatter_unknown_key.sql`.

**Review checklist.** No silent parse failures. Existing Phase 11 fixtures (`safe_divide.sql`, `multi_decl.sql`) still parse cleanly. Indentation tolerance verified.

**Commit.** `planner: parse function frontmatter via serde_yaml (Phase 43, Step 9 complete)`

---

## Step 10 — Canonical fixture fidelity & body-scope (Phases 44 to 47)

Closes review findings #8–#10 (research-fidelity examples) and #20–#22 (TableExpr body-scope edges + cross-function CTE inference). By the end of Step 10, every research-doc example up through §11 ships green in `examples/functions_demo/`, JOIN aliases are visible inside `TableExpr`-returning function bodies, and CTE bodies that wrap `smelt.fn.*` calls have inferred return schemas.

### Phase 44 — Canonical fixture restoration: `monitored_session_rollup` + `safe_divide` tighten

**Goal.** Add the missing research §10 example `monitored_session_rollup` (the only fixture exercising block-syntax composition — a function declared with fragment-sort parameters that internally calls another function and forwards a `PASSING` fragment). Tighten `safe_divide.sql` to match research §3 exactly (re-introduce `OR denominator IS NULL` guard and the second `CAST(denominator AS DOUBLE)`). Closes review findings #8 and #9.

**Pre-conditions.** Phase 29 (PASSING binding).

**TDD tests.**
1. `monitored_session_rollup_typechecks_clean` — workspace fixture types clean under `cargo test -p smelt-cli --test example_diagnostics`.
2. `monitored_session_rollup_passing_forward_typechecks` — a model that calls `monitored_session_rollup` with `PASSING metrics AS (…)` correctly forwards through to the inner `session_rollup` PASSING binding.
3. `safe_divide_handles_null_denominator` — call `safe_divide(x, NULL)` in a model; expansion produces the `OR denominator IS NULL` guard (verified in `--show-plan` output).
4. `safe_divide_double_cast_preserved` — generated SQL contains both `CAST(numerator AS DOUBLE)` and `CAST(denominator AS DOUBLE)`.

**Implementation.**
- New `examples/functions_demo/functions/monitored_session_rollup.sql` containing the verbatim research §10 example.
- New `examples/functions_demo/models/monitored_dashboard.sql` exercising the function with a `PASSING metrics AS (…)` clause that forwards.
- Edit `examples/functions_demo/functions/safe_divide.sql` to restore the dropped `OR denominator IS NULL` guard and the second CAST.

**Review checklist.** No type-system change required — these are fixture-only edits. Phase 29's PASSING-forward path is exercised end-to-end for the first time. `safe_divide` matches research §3 verbatim.

**Commit.** `examples: tighten safe_divide to research §3 spec (Phase 44, Step 10 opens)`

**Status note (2026-04-26):** Phase 44 shipped as a *partial close* — finding #9 (`safe_divide` tighten) lands; finding #8 (`monitored_session_rollup` fixture) carved into a new **Phase 44b** below. Tests 3 + 4 (the `safe_divide` ones) pass; tests 1 + 2 belong to 44b.

### Phase 44b — Fragment-forward parser + type-system support (closes finding #8)

**Goal.** Land the parser and type-system primitives that the research §10 `monitored_session_rollup` example needs, then ship the fixtures from Phase 44's deferred half. This is *not* a fixture-only phase — it adds compiler capability.

**Pre-conditions.** Phase 29 (PASSING binding), Phase 41 (body splice), Phase 44 (safe_divide tighten).

**TDD tests.**
1. **Parser** `cte_body_accepts_bare_smelt_fn_call` — a `WITH name AS (smelt.fn.<path>(...) PASSING <name> AS (...))` body parses without "Expected SELECT, WITH, or VALUES in CTE." The parser treats a bare smelt.fn-call (optionally followed by trailing PASSING clauses) as a CTE body equivalent to `SELECT * FROM <call>`.
2. **Type system** `fragment_param_reference_in_passing_body_inherits_kind` — a `PASSING items AS (outer_metrics)` body where `outer_metrics: SelectItems<Agg>` is the enclosing function's parameter binds the inner parameter without a `FragmentKindMismatch` (the inner parameter expects `Agg`-or-higher; the outer fragment-typed parameter satisfies that).
3. **Type system** `fragment_param_reference_exempt_from_splice_column_validation` — the same body does not surface a `FragmentColumnMissing` for the parameter name (`outer_metrics`); fragment-typed parameter references skip the splice-context column-validation walk.
4. **Fixture** `monitored_session_rollup_typechecks_clean` — adding `examples/functions_demo/functions/monitored_session_rollup.sql` (research §10 verbatim) leaves `cargo test -p smelt-cli --test example_diagnostics` green.
5. **Fixture** `monitored_session_rollup_passing_forward_typechecks` — adding `examples/functions_demo/models/monitored_dashboard.sql` calling `monitored_session_rollup` with `PASSING metrics AS (…)` types clean; the two-level forward expansion path is exercised.
6. **Negative** `non_fragment_param_reference_still_kind_checked` — referencing an `Expr<Integer>` parameter inside a `SelectItems<Agg, ctx>` PASSING body still surfaces a kind error (regression guard against over-broad exemption).
7. **Negative** `non_param_column_in_fragment_body_still_validated` — a column reference inside a fragment-param body that is *not* the fragment-typed parameter's name still hits the existing `FragmentColumnMissing` path against the splice context.

**Implementation.**
- **Parser** (`crates/smelt-parser/src/parser.rs`): when parsing a CTE body, accept `smelt.fn.<path>(...) [PASSING ... ]*` as a valid alternative to a `SELECT`/`WITH`/`VALUES` start. The CST shape can desugar to a wrapping `SELECT * FROM <call>` synthesized node, or the body can carry a new `CTE_FN_CALL` variant — choose whichever keeps the rest of the type-checker reuse cheapest.
- **Type system** (`crates/smelt-db/src/function_body_check.rs`): teach the kind-inference and splice-column-validation walkers that a bare reference to a fragment-typed parameter (`SelectItems<…>`, future fragment sorts) inherits the parameter's declared kind and is *not* a column reference for the purpose of `check_fragment_context_bindings`. Implement as a small `lookup_fragment_param_kind` helper checked before the existing column-reference path.
- **Fixtures**: land the `monitored_session_rollup.sql` and `monitored_dashboard.sql` files Phase 44 deferred.

**Example fixtures.** As above. No new broken fixtures unless tests 6 / 7 surface new ones — the regression guards may live as unit tests instead.

**Review checklist.** Parser change additive (existing CTE-body shapes still parse). Kind-inheritance and splice-exemption rules don't regress any Phase 21 / Phase 29 fixtures. The two new fixtures stay clean under `example_diagnostics`. The `--show-plan` output for `models/monitored_dashboard.sql` shows two-level expansion (outer `monitored_session_rollup` body splice + inner `session_rollup` body splice).

**Commit.** `parser+db: fragment-forward through PASSING; ship monitored_session_rollup (Phase 44b, finding #8)`

### Phase 45 — JOIN aliases visible in `TableExpr`-returning function bodies

**Goal.** A function body that joins a `TableExpr` parameter to an external table (`JOIN smelt.ref('Y') AS y`) sees `y`'s columns through the body's bare-column resolver. Closes review findings #10 and #21.

**Pre-conditions.** Phase 15 (TableExpr row polymorphism), Phase 17 (return-schema inference).

**TDD tests.**
1. `joined_alias_columns_visible_in_body` — `enriched_order` body uses `dim_customer.customer_name` directly; resolves correctly.
2. `joined_alias_shadow_warning` — alias name collides with a parameter → Phase 15's shadow warning fires consistently.
3. `joined_alias_missing_column_errors` — `JOIN smelt.ref('Y') AS y` referencing `y.does_not_exist` → `UnknownIdentifier` rooted at the body span.
4. `joined_alias_in_select_star_expansion` — `SELECT y.*` inside the body expands the alias's schema into the function's return schema.
5. `enriched_order_no_longer_uses_cast_null_workaround` — fixture body actually reads `dim_customer.customer_name` (delete the `CAST(NULL AS VARCHAR)` workaround).

**Implementation.**
- Extend `function_body_check.rs::tableexpr_schema_lookup` (currently scoped to `smelt.ref()` / `smelt.source()` args) with a JOIN-alias visitor. Walk the body's `FROM` and `JOIN` clauses; for each `JOIN <smelt.ref|source|sub-call> AS alias`, register the alias→schema in the body's FROM scope.
- Aliased TableExpr parameters (`FROM source AS s`) work the same way — `s` becomes a synonym.
- Extend Phase 17's return-schema inference so `SELECT y.*` from a joined alias contributes to the inferred return schema.

**Example fixtures.** Edit `examples/functions_demo/functions/enriched_order.sql` to drop the `CAST(NULL AS VARCHAR)` workaround and read real `dim_customer.*` columns. New broken fixtures `examples/broken/models/fn_join_alias_missing_col.sql` and `fn_join_alias_shadow.sql`.

**Review checklist.** `enriched_order.sql` no longer has the workaround comment. Phase 34's join-elimination demo (`models/order_totals.sql`) actually elides a real join now. Existing Phase 15–17 tests still pass.

**Commit.** `db: JOIN aliases in TableExpr body scope (Phase 45)`

### Phase 46 — `TableExpr` argument shapes: CTEs, derived tables, subqueries

**Goal.** Calls of a `TableExpr`-parameterised function whose argument is a CTE reference, a derived table (`FROM (SELECT …) AS x`), or an inline subquery resolve the schema correctly. Closes review finding #20.

**Pre-conditions.** Phase 45.

**TDD tests.**
1. `tableexpr_arg_from_cte` — `WITH x AS (SELECT …) SELECT * FROM smelt.fn.add_margin(x)` resolves `x`'s columns inside the body.
2. `tableexpr_arg_from_derived_table` — `FROM smelt.fn.add_margin((SELECT … FROM y) AS d)` — derived-table arg resolves.
3. `tableexpr_arg_inline_subquery` — `smelt.fn.add_margin((SELECT * FROM y))` resolves.
4. `tableexpr_arg_unrecognised_shape_errors_clearly` — argument is a literal or a non-table expression → diagnostic at the arg span (not buried inside the body).

**Implementation.**
- Extend the `tableexpr_schema_lookup` closure to recognise CTE references (look up via `TypeContext::cte_columns`), derived tables (recurse into the inner SELECT for schema inference), and parenthesised subqueries (same as derived).
- Drop the "Phase 15 — `tableexpr_schema_lookup` closure resolves only `smelt.ref('X')` / `smelt.source('a.b')` arguments" deferral entry in this plan's "Deferred during implementation" section once the test passes.

**Example fixtures.** Add `examples/functions_demo/models/margin_via_cte.sql`. No broken fixtures (the current behaviour was a false-positive; this phase removes them).

**Review checklist.** Phase 22's `mark_cte_opaque` shortcut is no longer the dominant path — note the relationship with Phase 47.

**Commit.** `db: resolve TableExpr arguments from CTEs, derived tables, and subqueries (Phase 46)`

### Phase 47 — Cross-function CTE schema inference: drop opaque-CTE suppression

**Goal.** A CTE body of the shape `WITH x AS (SELECT * FROM smelt.fn.<…>(…)) …` infers `x`'s schema by resolving the callee's return schema. Drops the Phase 22 `mark_cte_opaque` workaround. Closes review finding #22.

**Pre-conditions.** Phase 46, Phase 17.

**TDD tests.**
1. `cte_schema_inferred_from_smelt_fn_call` — `WITH x AS (SELECT * FROM smelt.fn.sessionize(…)) SELECT user_col, session_id FROM x` types clean (today, the `SELECT user_col, …` line surfaces false-positive `UnknownIdentifier`).
2. `cte_schema_typo_inside_caller_caught` — same shape, but the outer SELECT references a column that does NOT exist in the inferred schema → `UnknownIdentifier` at the typo (today, suppressed by `mark_cte_opaque`).
3. `session_rollup_existing_tests_still_pass` — Phase 22's workflow regressions explicitly checked.
4. `cte_schema_inference_handles_chained_smelt_fn_calls` — `WITH x AS (SELECT * FROM smelt.fn.add_margin(smelt.fn.sessionize(…)))` resolves.

**Implementation.**
- Replace `mark_cte_opaque` shortcut in `function_body_check.rs` with a real return-schema lookup. Use Phase 17's `infer_tableexpr_return_schema(callee_sig, callee_body, call-site arg types)` machinery.
- The inferred CTE schema flows into `TypeContext::cte_columns` for downstream resolution.
- Salsa caching: schema inference is per-(callee fn_id, arg types) — reuses Phase 26's `DataTypeHash`.

**Example fixtures.** No new fixtures; tighten `models/rollup_dashboard.sql` to actually project columns from the `sessionized` CTE (today it's `SELECT *` to dodge the suppression). Add a broken fixture for the typo case.

**Review checklist.** Phase 22's "opaque-CTE suppression for `smelt.fn.*` wildcard bodies" deferral entry in this plan is marked resolved. No regression in existing Phase 20–22 tests. The `mark_cte_opaque` API is deleted (or marked `#[deprecated]` if downstream code outside the function-checker uses it).

**Commit.** `db: cross-function CTE schema inference, drop opaque-CTE suppression (Phase 47, Step 10 complete)`

---

## Step 11 — LSP polish (Phase 48)

Closes review findings #14, #15, #16. Single phase, three deliverables that share the same cursor-in-CST + signature-resolution infrastructure.

### Phase 48 — LSP hover wiring + PASSING completion + multi-level frame trace in message

**Goal.** Three deferred LSP polish items from Phase 24, Phase 29, and Phase 12, landed together because they share the same cursor traversal and signature lookup machinery.

**Pre-conditions.** Phase 24 (pure hover helper), Phase 29 (PASSING type-check), Phase 12 (frame data structure).

**TDD tests.**
1. `lsp_hover_on_smelt_fn_call_shows_declared_return` — hover on a `smelt.fn.*` call site whose callee has a Tier 3 declared return type → tooltip contains `format_smelt_type_hover` output.
2. `lsp_hover_on_passing_clause_param_shows_param_signature` — hover on `PASSING metrics AS (…)` highlights the `metrics` parameter declaration (clause name → param signature).
3. `lsp_completion_in_passing_body_lists_context_columns` — cursor inside `PASSING metrics AS (|)` for a `metrics: SelectItems<Agg, sessionized>` parameter returns `sessionized`'s columns as completions.
4. `lsp_completion_in_passing_body_filters_by_kind` — completion only suggests aggregate-kind expressions when the parameter's kind is `Agg`.
5. `multi_level_frame_trace_in_message_body` — diagnostic message text contains "in expansion of `outer`", "in expansion of `middle`", and "in expansion of `inner`" lines (outer-most first), not just in `relatedInformation`.

**Implementation.**
- New helper `find_smelt_fn_call_at_cursor(syntax, position) -> Option<SmeltFnCall>`. Used by both hover and completion.
- Hover handler: when the cursor is on a `smelt.fn.*` call name segment, resolve the signature via Salsa and format with `declared_return_hover_text(sig)`.
- Completion handler: when the cursor is inside a `PASSING_BODY` node, walk up to `SMELT_FN_CALL` + `PASSING_NAME`, resolve the parameter's context, and return the context's columns (filtered by parameter kind).
- Multi-level frame rendering: extend `render_expansion_frames` (`crates/smelt-lsp/src/lib.rs:819`) to prepend every frame's "in expansion of …" line into the message body, outer-most first. `relatedInformation` is unchanged (parallel surface).

**Example fixtures.** Manual smoke per the plan's existing Phase 18/22/27 patterns. New LSP e2e tests in the existing harness.

**Review checklist.** Phases 24/29 deferral notes in "Deferred during implementation" are marked resolved. Multi-level rendering shows in editors that don't surface `relatedInformation`. No regression in single-level rendering tests.

**Commit.** `lsp: hover, PASSING completion, and multi-level frame rendering (Phase 48, Step 11 complete)`

---

## Step 12 — Type-system depth (Phases 49 to 50)

Closes review findings #17 (registry coverage), #19 (kind discipline depth). Finding #18 (Decimal divergence) is a one-liner addressed inside Phase 50's tests.

### Phase 49 — `WindowInScalarContext` deep-walk: catch window functions buried in scalar subqueries

**Goal.** The Phase 14 `WindowInScalarContext` check fires not only at top-level expression positions but also when a window function appears inside a scalar subquery in `WHERE`, `GROUP BY`, or `HAVING`. Closes review finding #19.

**Pre-conditions.** Phase 14.

**TDD tests.**
1. `where_subquery_with_window_func_errors` — `WHERE col > (SELECT MAX(ROW_NUMBER() OVER (…)) FROM t)` → `WindowInScalarContext` diagnostic at the inner `ROW_NUMBER()`.
2. `having_subquery_with_window_func_errors` — same with HAVING.
3. `select_list_subquery_with_window_func_allowed` — top-level SELECT accepts window kind, so the check does not fire here (regression guard).
4. `from_clause_subquery_with_window_func_allowed` — derived tables don't trigger the scalar-context check.

**Implementation.**
- The current `infer_expression_kind` walker stops at sub-expression boundaries that aren't themselves `Expr`-kinded. Extend it to recurse into nested SELECT/subquery expressions, propagating the surrounding kind expectation.
- `Phase 14 — infer_expression_kind parallel-walker gap` deferral note becomes "partially resolved" (this phase does scalar subqueries; array literals / struct literals / `IN` / `EXISTS` remain — flag the residual).

**Example fixtures.** Add `examples/broken/models/fn_window_in_subquery_where.sql`, `fn_window_in_subquery_having.sql`. Append rows to `broken_function_diagnostics.rs`.

**Review checklist.** No false positives in `examples/timeseries/` and `examples/retail_analytics/`. Single-level Phase 14 tests still green.

**Commit.** `db: WindowInScalarContext deep-walk into scalar subqueries (Phase 49, Step 12 opens)`

### Phase 50 — Built-in registry expansion: operators + missing aggregates + missing window funcs

**Goal.** Seed the canonical registry with the operators and built-ins needed for production SQL coverage. Closes review findings #17 and #18.

**Pre-conditions.** Phase 9 (registry rewire).

**TDD tests** (new `crates/smelt-types/tests/registry_coverage.rs`):
1. **Operators** — `LIKE`, `ILIKE`, `IS NULL`, `IS NOT NULL`, `BETWEEN`, `IN`, `EXISTS`, `CAST` each have a registry entry. (`CAST` is special-cased in the parser but still gets a typed signature for hover / completion.)
2. **Aggregates** — `STRING_AGG`, `LISTAGG`, `ARRAY_AGG`, `MEDIAN`, `STDDEV`, `STDDEV_POP`, `STDDEV_SAMP`, `VARIANCE`, `VAR_POP`, `VAR_SAMP`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR`, `BIT_XOR`, `ANY_VALUE`, `APPROX_COUNT_DISTINCT`.
3. **Window functions** — `NTILE`, `FIRST_VALUE`, `LAST_VALUE`, `NTH_VALUE`, `CUME_DIST`, `PERCENT_RANK`.
4. **String** — `LTRIM`, `RTRIM`, `CHAR_LENGTH`, `CHARACTER_LENGTH`, `REPLACE`, `LPAD`, `RPAD`, `REPEAT`, `SUBSTR`, `SPLIT_PART`, `STRPOS`, `LEFT`, `RIGHT`.
5. **Math** — `EXP`, `LOG10`, `LOG2`, `MOD`, `SIGN`, `SIN`, `COS`, `TAN`, `ATAN`, `ATAN2`, `SINH`, `COSH`, `TANH`, `PI`.
6. **Temporal** — `DATE_PART`, `DATE_ADD`, `DATE_SUB`, `MAKE_DATE`, `MAKE_TIMESTAMP`, `AGE`.
7. **Decimal divergence registered** — `ABS<T: Numeric>(T) -> T` produces a `Decimal{p,s}` result for `ABS(Decimal{p,s})` (matching DuckDB), or the divergence is added to `divergences.rs` with explicit comment. Closes review finding #18.
8. **Property test** — `prop_registry_signatures_consistent_with_duckdb` runs every new signature through the DuckDB oracle.

**Implementation.**
- Append to the `REGISTRY` LazyLock initialiser in `crates/smelt-types/src/signatures.rs`. Group by family with section comments matching the existing layout.
- Operators (`LIKE`, `IN`, `BETWEEN`) get signatures even though parser handles them as primitive grammar — used for hover/completion only.
- `CAST` signature is `CAST<T>(Any, Type) -> Expr<T>` — `Type` is a placeholder enum tag (a future signature-language extension per §13 Category 3); for v1, accept it as `Any` to avoid blocking.
- Decimal: either thread Decimal precision through `ABS` (return `T` with the same precision) or document the divergence in `crates/smelt-db/tests/prop_helpers/divergences.rs`. Decision: divergence registry — full Decimal-precision arithmetic is out of scope for v1 per §16 #9.

**Example fixtures.** Extend `examples/functions_demo/models/uses_generics.sql` with calls to a sampling of new signatures. No broken fixtures unless the property tests surface unexpected behaviour.

**Review checklist.** `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` still green. No regressions in `examples/timeseries/` or `examples/retail_analytics/`. Plan note about "Phase 9 hand-written match fallback" is removed — the registry is now genuinely canonical.

**Commit.** `types: expand canonical registry with operators, aggregates, and window funcs (Phase 50, Step 12 complete)`

---

## Step 13 — Soundness, lint, and cleanup (Phases 51 to 53)

Closes review findings #11, #23–#25, #27, #28. Three short phases.

### Phase 51 — `provenance` / `joins` validator

**Goal.** When a function declares `provenance:` or `joins:` in frontmatter, the compiler verifies the declaration against the body. Mismatches emit diagnostics. Closes review finding #23.

**Pre-conditions.** Phase 31, Phase 43, Phase 45.

**TDD tests.**
1. `provenance_matches_body_projection` — declared `provenance: { margin: [source.revenue, source.cost] }` and body `SELECT revenue - cost AS margin FROM source` types clean.
2. `provenance_extra_column_errors` — declared `provenance: { margin: [source.revenue, source.cost, dim.x] }` but body never reads `dim.x` → `ProvenanceMismatch` diagnostic.
3. `provenance_missing_column_errors` — body reads `source.revenue` for an output column but provenance omits it → diagnostic.
4. `joins_declared_but_body_has_different_join_set` — `joins: [{ table: dim_a, … }]` but body joins `dim_b` → diagnostic.
5. `joins_cardinality_unverifiable_warning` — declared cardinality (`1:1`, `1:N`) cannot be verified statically; explicit `Severity::Warning` documenting the §20E soundness caveat.

**Implementation.**
- Pure validator: takes the parsed body's projection list / join graph + the declared provenance/joins YAML map, returns a `Vec<Diagnostic>`.
- New `DiagnosticCode::ProvenanceMismatch`, `DiagnosticCode::JoinsMismatch`, `DiagnosticCode::DeclaredCardinalityUnverifiable` (warning).
- The validator runs only when the workspace has the `unstable_schema` flag set — same gate as Phase 31's provenance parsing.

**Example fixtures.** Tighten `examples/functions_demo/functions/enriched_order.sql` so its declared `provenance` and `joins` actually match the body (now possible after Phase 45). Broken: `examples/broken/models/fn_provenance_extra_col.sql`, `fn_joins_mismatch.sql`.

**Review checklist.** Validator is pure, runs in `smelt-db`. The §20E soundness caveat is now actively flagged at compile time (warning, not error, per §20E "the rule does not verify cardinality against data").

**Commit.** `db: provenance and joins frontmatter validator (Phase 51, Step 13 opens)`

### Phase 52 — Discoverability lint: missing-provenance pushdown advisory + extern fragment-param reject

**Goal.** Two unrelated small lints shipped together: (a) when a transparent function lacks declared provenance and a pushdown candidate sits above it, emit an info-level diagnostic explaining the lost optimisation; (b) reject `smelt.extern` declarations that use fragment-sort parameters (`SelectItems`, `OrderSpec`) at parse time per §16 #18 deferral. Closes review findings #11 and #24.

**Pre-conditions.** Phase 51, Phase 33 (pushdown).

**TDD tests.**
1. `missing_provenance_pushdown_advisory` — model with `WHERE` over a transparent call whose function lacks `provenance:` → `Severity::Hint` diagnostic at the model's WHERE clause referencing the function declaration.
2. `provenance_present_no_advisory` — same model but the function has `provenance: …` → no diagnostic.
3. `extern_with_selectitems_param_rejected` — `smelt.extern foo(items: SelectItems<Agg>) -> TableExpr` → parse-time `DiagnosticCode::ExternFragmentParamUnsupported`.
4. `extern_with_orderspec_param_rejected` — same for `OrderSpec`.
5. `extern_with_expr_or_tableexpr_param_unchanged` — Phase 10's existing externs still parse clean.

**Implementation.**
- For (a): a planner-side or LSP-side post-pass that walks the logical plan looking for `Select { filter: Some, from: FunctionCall { transparent: true, provenance: Unknown, … } }` and emits a `Hint`-severity diagnostic.
- For (b): `parse_smelt_extern` validates that no parameter type is a fragment sort. New `DiagnosticCode::ExternFragmentParamUnsupported`.

**Example fixtures.** Extend `examples/functions_demo/models/uses_safe_divide.sql` with a `WHERE` clause exercising the advisory (or document why it doesn't fire). Broken: `examples/broken/models/fn_extern_with_selectitems.sql`.

**Review checklist.** `Hint` severity surfaces in LSP as a code-action opportunity, not a hard error. Phase 10's existing externs don't regress.

**Commit.** `db+parser: missing-provenance lint and extern fragment-param reject (Phase 52)`

### Phase 53 — Plan audit: empty SHAs, stale comments, cross-file extern collision fixture

**Goal.** Final cleanup pass. Fill the empty commit-SHA cells in the progress table (review #27). Fix the stale "`Context`" type comment in cross-phase risks (review #28). Add the missing cross-file extern same-name multi-backend negative fixture (review #25). Closes the remaining minor findings.

**Pre-conditions.** None (cleanup-only, but should run after all other Phase 39–52 work to catch any new gaps).

**TDD tests.**
1. `cross_file_extern_same_name_different_backends_rejected` — two files each declaring `smelt.extern foo` with different `backends:` sets → `DiagnosticCode::ExternDuplicateDeclaration` at the second declaration.

**Implementation.**
- Audit: `git log --oneline --grep="Phase 13"`, `--grep="Phase 34"`, `--grep="Phase 37"`, fill the empty cells in the progress-tracking table.
- Edit cross-phase-risks paragraph for Phase 33 to remove the stale "marker in `Context`" reference (the actual implementation uses `pushed_filter.is_some()` on the node).
- Confirm Phase 38 ("Step 8 closure") notes `--show-plan` smoke step now executable post-Phase 39.
- Add `examples/broken/models/fn_extern_collide_cross_file_a.sql` + `_b.sql` for the cross-file collision test.

**Example fixtures.** As above.

**Review checklist.** No code drift between this phase and Phases 39–52. Progress table is fully audited. `docs/ROADMAP.md` updated to mark "Smelt Functions — Steps 9–13" complete with date.

**Commit.** `plan+examples: progress-table audit and cross-file extern collision fixture (Phase 53, Step 13 complete)`

---

## Verification — Steps 9–13 closure

After Phase 53:
- `cargo fmt --all -- --check` ✅ (2026-04-27)
- `cargo clippy --all-targets` — zero warnings ✅ (2026-04-27)
- `cargo test` — all green ✅ (2026-04-27)
- `cargo test -p smelt-cli --test example_diagnostics` — zero diagnostics ✅ (2026-04-27)
- `PROPTEST_CASES=200 cargo test -p smelt-db --test type_property_tests prop_type_inference` — oracle passes ✅ (2026-04-27)
- `smelt compile examples/functions_demo/models/order_totals.sql --show-plan` — requires running `smelt compile`; the rule pipeline fires in unit tests; CLI integration is the Phase 39 deliverable; verified via `crates/smelt-cli/tests/show_plan.rs` test suite.
- Update `docs/ROADMAP.md` marking "Smelt Functions — Steps 6–13" complete ✅ (Phase 54, 2026-04-27)
- `docs/research/20260413-smelt-functions.md` §16 Decision 19 update deferred: `smelt.as_struct` SQL emission is not yet end-to-end wired through `smelt build` (body lowering via `LogicalNode::Raw` placeholders is the remaining step). Research doc retains "Step 8 revisit" framing until SQL emission lands. See `docs/TODO.md` open item.

## Phase 54 — End-user documentation for smelt functions

**Goal.** Add user-facing documentation so developers can discover and use the functions feature without reading the research paper or plan. Closes the documentation gap identified in the post-Phase-53 review. Also fixes stale/duplicate ROADMAP.md entries and checks off completed TODO.md items.

**Deliverables.**
1. `docs-site/docs/guide/functions.md` — complete guide covering: `smelt.define` / `smelt.fn.*` syntax, three-tier annotation model, type constraints, fragment sorts (`TableExpr`, `SelectItems`, `AggExpr`, `WindowExpr`), PASSING clauses, `smelt.extern`, `smelt.as_struct`, frontmatter (`backends:`, `deterministic:`, `provenance:`, `joins:`), and a diagnostic reference table.
2. `docs-site/docs/reference/language.md` — extend the smelt extensions section with `smelt.define`, `smelt.fn.*`, `smelt.extern`, and `smelt.as_struct` entries.
3. `docs-site/mkdocs.yml` — add "Functions" entry in Guide nav.
4. `docs/ROADMAP.md` — add Steps 6–13 "Recently Completed" entry; remove duplicate "What's Next" items; remove stale "Smelt Functions — Steps 6–8" from "Future / Exploration".
5. `docs/TODO.md` — check off ABS(Decimal) divergence (registered in Phase 53).

**Commit.** `docs: functions guide, language-ref extensions, ROADMAP cleanup (Phase 54)`

## Progress tracking — Phases 39 to 53

Updated as phases complete. Same format as the earlier progress-tracking table.

| Phase | Title | Status | Commit | Date |
|---|---|---|---|---|
| 39 | Wire logical-plan rules into `smelt compile`; add `--show-plan` (Step 9 opens) | done | 029d7fa | 2026-04-25 |
| 40 | CAST emission resolves target type from `Signature::canonical_return` | done | 1b1d613 | 2026-04-25 |
| 41 | `ExpandedCall` body splice + list-splice comma elision | done | 7dc0ea9 | 2026-04-25 |
| 42 | `smelt.as_struct` lowering wired in + capability gate broadened | done | 3bac61d | 2026-04-25 |
| 43 | Frontmatter YAML parser via `serde_yaml` (Step 9 complete) | done | 9594d15 | 2026-04-26 |
| 44 | Canonical fixture restoration: `safe_divide` tighten (Step 10 opens) — partial close, #8 carved to 44b | done | 3856027 | 2026-04-26 |
| 44b | Fragment-forward parser + type-system support; ship `monitored_session_rollup` (closes finding #8) | done | 221d7d8 | 2026-04-26 |
| 45 | JOIN aliases visible in `TableExpr`-returning function bodies | done | 8e5fe6a | 2026-04-26 |
| 46 | `TableExpr` argument shapes: CTEs, derived tables, subqueries | done | 1c39757 | 2026-04-26 |
| 47 | Cross-function CTE schema inference: drop opaque-CTE suppression (Step 10 complete) | done | c89d2b7 | 2026-04-26 |
| 48 | LSP hover wiring + PASSING completion + multi-level frame trace (Step 11 complete) | done | 221d7d8 | 2026-04-26 |
| 49 | `WindowInScalarContext` deep-walk into scalar subqueries (Step 12 opens) | done | 179de87 | 2026-04-26 |
| 50 | Built-in registry expansion: operators + missing aggregates + window funcs (Step 12 complete) | done | 20c5eb0 | 2026-04-26 |
| 51 | `provenance` / `joins` validator (Step 13 opens) | done | 75dd429 | 2026-04-26 |
| 52 | Missing-provenance pushdown advisory + extern fragment-param reject | done | 15e4ada | 2026-04-26 |
| 53 | Plan audit: empty SHAs, stale comments, cross-file extern fixture (Step 13 complete) | done | 3a800d5 | 2026-04-26 |
| 54 | End-user documentation: functions guide + language-ref + ROADMAP cleanup | done | ae8e58f | 2026-04-27 |
| 55 | `smelt.as_struct()` + `smelt.fn.*` SQL emission wired into `smelt build` | done | 6d6e5b1 | 2026-04-27 |
