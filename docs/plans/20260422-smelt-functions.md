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
- **Planner-rule fixed point on transparent functions (Step 7).** Phase 33's first rewrite pushes filters across `LogicalNode::FunctionCall { transparent: true, .. }`. If the rule doesn't terminate (pushing the same filter repeatedly), the planner loop loops. Termination is guaranteed by an "already-pushed" marker in `Context`, tested explicitly.
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
- `smelt compile models/order_totals.sql --show-plan` should demonstrate join elimination on the example from Phase 34.
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
| 13 | Parser: TableExpr / WindowExpr / SelectItems<K, ctx> in type refs (Step 3 opens) | done | | 2026-04-23 |
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
| 24 | Tier 3 return-type verification + LSP hover | pending | | |
| 25 | Call-site bidirectional checking (pre-expansion) | pending | | |
| 26 | Tier 2 → Tier 1 inline expansion | pending | | |
| 27 | Generics ↔ checking-mode interaction (Step 5 complete) | pending | | |
| 28 | Parser: context-sensitive `PASSING` clauses (Step 6 opens) | pending | | |
| 29 | PASSING binding to fragment-sort params + LSP completion (Step 6 complete) | pending | | |
| 30 | Logical plan data model: functions as first-class nodes (Step 7 opens) | pending | | |
| 31 | Column provenance + declared-property propagation | pending | | |
| 32 | Planner rule API + Level 2 expansion of function calls | pending | | |
| 33 | Filter pushdown across transparent-function boundaries | pending | | |
| 34 | Join elimination example (Step 7 complete) | pending | | |
| 35 | Parser + types: row variables on `Struct<…>` and value-level spread (Step 8 opens) | pending | | |
| 36 | Row unification at call sites with value-level erasure | pending | | |
| 37 | Row variable in return position: pass-through fields | pending | | |
| 38 | `smelt.as_struct()` revisit (Step 8 complete) | pending | | |

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
- **Phase 22 — `rollup_dashboard.sql` uses defaults for `metrics`/`filters`** (2026-04-24). The plan specifies "non-empty parenthesised `metrics` argument" but `PASSING` syntax (the inline block form for fragment parameters) is Step 6. The fixture uses the positional form with defaults omitted, exercising the empty-default path. Non-default fragment passing via `PASSING` lands in Phase 28–29.
- **Phase 22 — opaque-CTE suppression for `smelt.fn.*` wildcard bodies** (2026-04-24). When a CTE body is `SELECT * FROM smelt.fn.<path>(...)`, the CTE schema cannot be inferred without resolving the callee's return schema (a future mechanism). `TypeContext` gains `mark_cte_opaque()` so the type-checker returns `Unknown` for any column access against such a CTE, suppressing false-positive `UnknownIdentifier` errors. The full smelt-fn-return-schema inference in CTE bodies is deferred to a later phase as it requires cross-function schema propagation.
- **Phase 22 — `empty_default_metrics_splice_comma_elision` scoped to type-checker level** (2026-04-24). The plan test name implies SQL comma-elision; §16 #20 explicitly places that rule at Level 2 materialisation (Phase 32+). The test validates that the type-checker handles `SelectItems<Agg, ctx> = ()` without errors and that calling without `metrics` doesn't surface a diagnostic. SQL generation deferred to Phase 32.
- **Phase 8 broken fixtures → Phase 10** (2026-04-23, resolved). `examples/broken/models/fn_coalesce_text_int.sql` and `fn_greatest_no_args.sql` were originally slated for Phase 9. During Phase 9 implementation, reading `function_body_check::check_smelt_fn_call` (the only path that emits `ArgTypeMismatch` / `MissingArgument` for a `smelt.fn.*` call today) confirmed it only resolves user-declared functions via `ctx.lookup_function_signature`. Phase 9's `try_registry_inference` hook is a pure inference path and doesn't emit diagnostics — the rewire preserves `Unknown`-returning behaviour for coverage gaps, it doesn't spawn new error codes. The fix landed in Phase 10 via Option B: a new `builtin_lookup` closure on `check_smelt_fn_call` that dispatches built-ins through `unify_call` when the user-declared signature index misses, translating `UnificationError::{ConstraintViolation, MissingArgs, InconsistentBinding, EmptyVariadicTypeVar}` into `ArgTypeMismatch` / `MissingArgument` diagnostics. Both fixtures and their `broken_function_diagnostics.rs` CASES rows now live under Phase 10's coverage — `smelt.fn.COALESCE('x', 1)` surfaces `ArgTypeMismatch` (via `InconsistentBinding`) and `smelt.fn.GREATEST()` surfaces `MissingArgument` (via `EmptyVariadicTypeVar`, mapped to "variadic requires at least one argument").
