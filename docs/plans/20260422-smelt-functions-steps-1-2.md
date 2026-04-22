# Smelt Functions — Steps 1 & 2 Implementation Plan

**Date:** 2026-04-22
**Research:** [`docs/research/20260413-smelt-functions.md`](../research/20260413-smelt-functions.md) (§2, §3, §8, §13, §16, §19, §21)
**Tracking PR:** #108 (branch `worktree-review`)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion autonomously.

**Before touching any code:**

1. Read this entire plan file. Then read §2, §3, §8, §13, §16 (all 24 decisions), §19, and §21 of `docs/research/20260413-smelt-functions.md`. The plan assumes those decisions are settled — do not re-open them.
2. Confirm you are on branch `worktree-review` (PR #108): `git rev-parse --abbrev-ref HEAD`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the "Progress tracking" table below. That is your starting point. If every phase is `done`, run the post-Phase-12 verification under "Verification" and stop.

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
- Don't widen scope: TableExpr, PASSING, Tier 2/3 checking, planner visibility, etc. are all out of scope. If you think a phase "needs" one of those to pass its tests, re-read the phase — the tests are scoped deliberately.
- Commit messages are the phase's `Commit.` line verbatim; include the standard Claude Code co-author trailer.

You may now begin. Start by reading the files listed above, then proceed with the first `pending` phase.

---

## Context

The smelt-functions research has converged — §16 records 24 settled decisions and §21's pre-implementation checklist is complete. Step 1 (`smelt.define` for `Expr<T>` functions with Tier 1 checking, `safe_divide` end-to-end) and Step 2 (canonical built-in signature registry, generics, variadics, `smelt.extern`, `backends:` frontmatter) are the first two rungs of the experimentation roadmap. Together they establish the fragment-sort expansion model, the canonical signature vocabulary, and the Tier 1 error-tracing infrastructure every later step reuses.

This plan decomposes Steps 1 & 2 into twelve phases executed via red-green TDD. Each phase ships independently, commits atomically, and is reviewed by a subagent before the next begins.

## Scope

### In scope

- **Step 1:** `smelt.define` top-level declaration. `Expr<T>` sort only. Tier 1 checking (bind params→arg types in `TypeContext`, re-check body). `smelt.fn.*` call surface. Single-level frame-stack rendering (innermost frame + outermost call site). `safe_divide` end-to-end.
- **Step 2:** Canonical signature registry for ~80% of SQL built-ins (SUM, COUNT, MIN/MAX, COALESCE, CONCAT, LOWER/UPPER, ABS, POWER, etc.). `Ordered` constraint (§16 #13). Generics (§16 #14) and variadics (§16 #15). `smelt.extern` declarations (§16 #21). Per-declaration frontmatter (§16 #22). `backends:` inference and narrow-only declaration (§16 #23). Backend namespace sugar (`duckdb.*`). Multi-level frame-stack renderer. CAST-enforcement flag on canonical returns.

### Explicitly deferred (out of scope)

From §16 and §21:
- TableExpr sort, row polymorphism, structural column resolution → Step 3.
- AggExpr / WindowExpr / SelectItems / OrderSpec sorts → Step 3+ (§16 #8, #24).
- Context bindings (§6) and CTE forward-reference handling → Step 4.
- Tier 2 / Tier 3 annotation enforcement and bidirectional checking → Step 5 (§16 #17 *decided*; implementation is Step 5).
- `PASSING` block syntax → Step 6 (§16 #18).
- Planner visibility / provenance-driven optimisation → Step 7.
- `smelt.as_struct()` → Step 8 (§16 #19).
- Decimal precision/scale, nullability tracking, Text collation (§16 #9/#10/#13).
- Generics and variadics in `smelt.define` (§16 #14/#15 — monomorphic in v1).
- Expansion caching, span-based error deduplication (§16 #12).
- CAST *emission* in generated SQL (Step 2 only records the flag; codegen integrates with planner lowering in Step 7).

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

## Cross-phase risks

- **`smelt.define` vs. identifier ambiguity.** `smelt` is not a reserved word. `smelt.define` is only special at top-level statement position (§16 #11). Phase 1 must encode this trigger; regressions would break models that happen to reference a column called `define`.
- **Frontmatter model change (§16 #22).** The current `strip_frontmatter` assumes a single file-level block. Phase 11 moves to per-declaration. All prior phases must use the legacy single-block rule unchanged so fixtures don't break mid-plan.
- **`UnrecognizedFunction` collision.** Phases 6 and 9 both touch function lookup. New `DiagnosticCode::UnknownSmeltFn` keeps `smelt.fn.*` misses distinct from plain SQL function misses.
- **Registry coverage gap (Phase 9).** Rewiring `infer_function_type` through the registry must preserve current property-test behaviour. Spike first: confirm every `SqlFunction` variant removed from the legacy match has a registry entry.

## §21 status after Steps 1 & 2

Unblocked or settled by this work: `smelt.define` grammar, expansion mechanics, Tier 1 error tracing MVP + full rendering, `Ordered` constraint, generics syntax/inference, variadics, `smelt.extern` full syntax, unified frontmatter, engine-agnostic bodies.

Still blocked (for Step 3+): CTE forward reference, function file discovery policy, `AggExpr` keep-or-collapse, Tier 1→Tier 2 upgrade migration story.

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
- `crates/smelt-cli/tests/broken_function_diagnostics.rs` **(new, Phase 6)** — asserts `DiagnosticCode` + message substring for every `examples/broken/models/fn_*.sql` fixture. Phases 7–12 append rows here rather than creating new test files.
- `crates/smelt-parser/src/parser.rs` unit tests (Phases 1, 2).

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

## Progress tracking

Updated as phases complete. Format: `Phase N — <title> — <status> (<commit sha>, <date>)`. New deferrals appended under "Deferred during implementation".

| Phase | Title | Status | Commit | Date |
|---|---|---|---|---|
| 1 | Parser: `smelt.define` top-level grammar | done | 996c27d | 2026-04-22 |
| 2 | Parser: `smelt.fn.*` call syntax | done | e3db6fb | 2026-04-22 |
| 3 | Salsa function signature index | done | 936233d | 2026-04-22 |
| 4 | `Expr<T>` type-reference resolution | done | 0bd42b7 | 2026-04-22 |
| 5 | Tier 1 body check with parameter binding | done | 05a96f4 | 2026-04-22 |
| 6 | Call-site expansion + single-level frame trace (Step 1 complete) | pending | — | — |
| 7 | `Ordered` constraint + canonical registry skeleton | pending | — | — |
| 8 | Generics + variadics | pending | — | — |
| 9 | Rewire built-in inference through the registry | pending | — | — |
| 10 | `smelt.extern` declarations | pending | — | — |
| 11 | Per-declaration frontmatter + `backends:` + backend namespace | pending | — | — |
| 12 | Multi-level frame rendering + CAST-enforcement flag (Step 2 complete) | pending | — | — |

### Deferred during implementation

_None yet._
