# Plan: medium-loop follow-up findings

**Date**: 2026-05-03
**Tracking branch**: worktree-example-usage
**Spec**: `docs/specs/functions.md` (update — return type propagation semantics; path-prefix enforcement scope)

**Spec diff**: Phase 2 adds a §"Semantics" rule (§16) for declared return type propagation. Phase 1 tightens the existing §"Function call syntax" note ("path-prefix enforcement is normative") by applying it to the build/run path, not only the LSP path.

**Source**: 5-iteration medium-tier smelt-loop run (2026-05-03). All 5 runs passed 14/14 eval checks. Findings are therefore about user experience and latent correctness gaps, not outright build failures.

## Context

The `20260502-smelt-loop-findings.md` plan closed TB-1 through TB-5 and all DG items from the small-tier loop. Its Phase 2 (TB-5: path-prefix enforcement) wired the check into the LSP diagnostic path (`smelt_fn_call_diagnostics_for_file`) but not into the build/execution path. All five medium-tier agents used the stem-included call form (`smelt.functions.revenue.safe_revenue(...)`) — the form the medium fixture spec explicitly instructs — which the build path silently accepts even though the spec and the user docs both say the stem is not part of the call path.

The compounding effect: the medium fixture spec and the skill both teach the wrong call form, so agents never discover the discrepancy. The fix requires extending enforcement to the build path (TB-A), updating the fixture to the canonical form (FIX-A), and updating the skill; then the discrepancy collapses.

TB-B and TB-C (function return type not flowing into `smelt table`) are independent and affect schema introspection and downstream type inference. They are cosmetically harmless in practice (DuckDB physical types are correct) but undermine the skill's core advice to "validate schema, not just rows."

## Phase order

```
1. TB-A + FIX-A   path-prefix on build path + fixture + skill clarification (must land together)
2. TB-B + TB-C    function return type flows into model schema + SUM inference
3. Docs + harness  smelt table reference, functions guide tips, validate.py fix, skill cleanup
```

## Progress tracking

| Phase | Topic | Status | Date | Commit |
|-------|-------|--------|------|--------|
| 1 | TB-A: build-path path-prefix enforcement + fixture | done | 2026-05-03 | b207d41 |
| 2 | TB-B/C: function return type propagation into schema | done | 2026-05-03 | b31a0ff |
| 3 | Docs, harness, skill cleanup | pending | | |

---

## Phase 1 — TB-A: Path-prefix enforcement on the build/run path + FIX-A

**Spec:** `docs/specs/functions.md` §"Function call syntax":

> File-location → call-path mapping (path-prefix enforcement is normative; a wrong-prefix call emits `UnknownSmeltFn`):
>
> | Filesystem location | Declared name | Call path |
> |---|---|---|
> | `functions/status.sql` | `is_shipped` | `smelt.functions.is_shipped(...)` |

The spec says "enforcement is normative." After Phase 2 of `20260502-smelt-loop-findings.md` the LSP path enforces this. The build/execution path does not. The medium fixture spec and the skill both teach the stem-included form (`smelt.functions.revenue.safe_revenue`); the fixture must be updated atomically with the enforcement fix.

### Reproduction

```bash
# Project from any medium-loop run, e.g.:
cd ~/.smelt-test-runs/loop-1-20260503-051314/project

# The staging model calls smelt.functions.revenue.safe_revenue(...)
# (function declared in functions/revenue.sql — stem "revenue" in path is wrong per spec)
.venv/bin/smelt build           # succeeds — should fail
.venv/bin/smelt build --show-plan models/stg_orders.sql   # also succeeds — should fail

# The LSP path correctly rejects the stem-included form:
# (running smelt lsp diagnostics against the same file would show UnknownSmeltFn)
```

### Tests (write first — red-green)

In `crates/smelt-cli/tests/path_prefix_build.rs` (new file):

1. `test_build_rejects_stem_in_call_path` — project with `functions/status.sql` declaring `is_shipped`; model calls `smelt.functions.status.is_shipped(s)` (stem included) → `smelt build` must exit non-zero, stderr must mention `UnknownSmeltFn` or the call path.
2. `test_build_accepts_canonical_no_stem_call_path` — same project; model calls `smelt.functions.is_shipped(s)` (no stem) → `smelt build` must exit 0.
3. `test_show_plan_rejects_stem_in_call_path` — `smelt build --show-plan models/stg.sql` with stem-included call → must exit non-zero.

After implementation, confirm examples still pass:
```bash
cargo test -p smelt-cli --test example_diagnostics
```

### Implementation notes

- The LSP enforcement lives in `crates/smelt-db/src/function_body_check.rs::check_smelt_path_call` via the `path_prefix_validator` closure (added in the prior plan's Phase 2).
- The build/execution path runs function expansion in the planner step. Find where `smelt.<path>(...)` calls are resolved/expanded for execution (look for `ExpandedCall` in the planner crate or `smelt-db/src/`; `smelt build --show-plan` shows an `ExpandedCall` node in the plan output, making the expansion site visible). The path-prefix check must fire at or before that expansion point.
- The same `check_smelt_path_call` logic should be reusable — the implementer should call it (or the core path-prefix predicate) from the build path rather than re-implementing.
- After the tests pass, update these files atomically:
  - `tests/agent-loop/fixtures/medium/spec.md`: change `smelt.functions.revenue.safe_revenue(o.amount)` → `smelt.functions.safe_revenue(o.amount)` and `smelt.functions.<path>.<name>(...)` → `smelt.functions.<name>(...)` throughout.
  - `.claude/skills/smelt-app-builder/SKILL.md` line 159: change the ambiguous parenthetical `(does the filename stem appear?)` to a definitive statement: `The filename stem does **not** appear in the call path — \`functions/revenue.sql\` declaring \`safe_revenue\` is called as \`smelt.functions.safe_revenue(...)\`, not \`smelt.functions.revenue.safe_revenue(...)\`.`

### Critical files

- `crates/smelt-db/src/function_body_check.rs` — existing path-prefix validation logic
- `crates/smelt-db/src/lib.rs` — orchestrates both LSP and build paths; find the build-path entry point
- Planner crate or `smelt-db` function-expansion code (search for `ExpandedCall` or function resolution logic)
- `crates/smelt-cli/tests/path_prefix_build.rs` (new)
- `tests/agent-loop/fixtures/medium/spec.md` — fix call-path examples to canonical no-stem form
- `.claude/skills/smelt-app-builder/SKILL.md` — clarify stem does not appear

### Docs touched

None in this phase (Phase 3 handles user docs).

**Commit (Phase 1):** `fix(smelt-db/cli): enforce path-prefix validation on the build path; update medium fixture and skill to canonical no-stem call form`

---

## Phase 2 — TB-B + TB-C: Function return type propagation into model schema

**Spec:** `docs/specs/functions.md` §"Semantics" — add rule 16 (see below). The surface spec already says `-> <Type>` is the declared return type; the missing normative rule is that this type must flow into downstream schema inference.

### Reproduction

```bash
cd ~/.smelt-test-runs/loop-1-20260503-051314/project
.venv/bin/smelt build
.venv/bin/smelt table stg_orders
# amount       UNKNOWN     yes  ← expected: DOUBLE (function declares -> Expr<Double>)
.venv/bin/smelt table int_orders_by_day
# total_revenue  BIGINT  ...   ← expected: DOUBLE (SUM over DOUBLE should be DOUBLE)
duckdb orders.duckdb -c 'DESCRIBE stg_orders'
# amount       DOUBLE   YES    ← DuckDB physical type is correct; smelt inferred type is wrong
```

### Spec update (write before implementing)

Add to `docs/specs/functions.md` §"Semantics" after rule 15:

> 16. **Declared return type is authoritative for call-site typing.** When the type checker encounters a `smelt.<path>(...)` call, it looks up the function's declared return type. If the function carries a `-> <Type>` annotation (Tier 3), that type is the call expression's type — it takes precedence over inferred body type. If no return type is declared (Tier 1/2), the call expression's type is `Unknown`. The schema of any model that projects such a call reflects this rule: a column whose source expression is a `smelt.<path>(...)` call inherits the declared return type. Downstream aggregate functions (`SUM`, `AVG`, etc.) apply their standard return-type rules to the declared type, not to `Unknown`.

### Tests (write first — red-green)

In `crates/smelt-db/tests/function_return_type.rs` (new file):

1. `test_double_return_type_appears_in_model_schema` — workspace with `functions/revenue.sql` declaring `safe_revenue(a: Expr<Double>) -> Expr<Double>`; staging model selects `smelt.functions.safe_revenue(amount) AS amount`; call `model_schema()` on the staging model → column `amount` must have type `Double`, not `Unknown`.
2. `test_boolean_return_type_appears_in_model_schema` — `functions/status.sql` declaring `is_shipped(s: Expr<Text>) -> Expr<Boolean>`; model projects the call → column type must be `Boolean`.
3. `test_sum_over_function_double_infers_double` — aggregate model selecting `SUM(smelt.functions.safe_revenue(amount)) AS total_revenue`; type inference must return `Double`, not `BigInt`.
4. `test_untyped_function_produces_unknown` — `smelt.define f(x) AS (x + 1)` (no return type); model projects the call → column type is `Unknown` (no regression on the untyped path).

### Implementation notes

- Lives primarily in `crates/smelt-db/src/type_inference.rs` — the function that assigns a type to `smelt.<path>(...)` call expressions.
- Current behaviour: the call is typed as `Unknown` because the inferencer does not look up or use the declared `-> Expr<T>` signature.
- Fix: when the type inferencer encounters a `smelt.<path>(...)` call, resolve the function, retrieve its declared return type (if any), and return that type as the call expression's type. Use the same function-registry/Salsa lookup used by `check_smelt_path_call`.
- TB-C (`SUM(DOUBLE)` → `BIGINT`) is downstream of TB-B: once the call expression is typed as `Double`, `SUM(Double) → Double` applies correctly per existing rules in types.md §5.
- After tests pass, update `docs/specs/functions.md` §"Semantics" — add rule 16 as written above.

### Critical files

- `crates/smelt-db/src/type_inference.rs` — main fix location
- `crates/smelt-db/src/lib.rs` — Salsa query orchestration for type lookup
- `crates/smelt-db/tests/function_return_type.rs` (new)
- `docs/specs/functions.md` — add §Semantics rule 16

### Docs touched

- `docs/specs/functions.md` — §Semantics rule 16 (return type propagation)

**Commit (Phase 2):** `fix(smelt-db): propagate smelt.define declared return type into model schema; fix SUM(function-call) type inference`

---

## Phase 3 — User docs, harness fix, skill cleanup

**Goal.** Close four doc gaps (DOCS-B through DOCS-D, HARNESS-1) that don't require code changes, and clean up skill noise that Phase 2 makes obsolete.

### DOCS-B — `smelt table` missing from `reference/cli`

`smelt table <model>` is a real subcommand used by the skill and surfaced in multiple loop retros. It is not listed in `docs-site/docs/reference/cli.md`.

Add an entry under a "Schema inspection" or "Introspection" section:

> **`smelt table <model>`** — Print the inferred schema (column names and types) for a built model. The types shown are smelt's compile-time inferred types; use `duckdb <db> -c 'DESCRIBE <model>'` to see the physical DuckDB column types. After Phase 2 lands, the two views agree for models whose columns originate from typed `smelt.define` calls.

### DOCS-C — `guide/functions` missing `--show-plan` as function verification step

`docs-site/docs/guide/functions.md` explains call-path rules and boolean positioning but does not mention `smelt build --show-plan models/<m>.sql` as the recommended pre-build verification step for function calls. Add a short "Verifying function calls before a full build" subsection:

> Run `smelt build --show-plan models/<m>.sql` to confirm a call expands correctly without executing. The `ExpandedCall` node in the plan output shows the inlined body with argument substitution, making it easy to spot wrong-path calls or type mismatches before a full `smelt build`.

### DOCS-D — `guide/functions` should document that return types flow into schema

After Phase 2, `smelt table` correctly reflects `-> Expr<T>` return types. Add one sentence in the "Calling a function" section:

> For typed functions (those with a `-> ReturnType` annotation), smelt uses the declared return type as the column type in downstream models. `smelt table <model>` reflects this — a column fed by a `-> Expr<Double>` call will show as `DOUBLE`.

### HARNESS-1 — Medium fixture spec references `../validate.py` at wrong path

`tests/agent-loop/fixtures/medium/spec.md` instructs agents:

> Run `python ../validate.py` from inside your project directory.

The harness runs validation via `eval.sh`; `validate.py` is not placed at `../` relative to the project directory. Agents who follow the instruction get a `FileNotFoundError` (iteration 5 retro confirms this).

Remove the `python ../validate.py` instruction. Replace with:

> The harness runs `validate.py` automatically after you finish — you do not need to invoke it manually. To self-check, query the output tables directly:
> ```bash
> duckdb my-project.duckdb -c "SELECT COUNT(*) FROM stg_orders"
> duckdb my-project.duckdb -c "DESCRIBE int_orders_by_day"
> ```

### Skill cleanup

After Phase 2, remove stale noise from `.claude/skills/smelt-app-builder/SKILL.md`:

- **Remove line ~149** (the UNKNOWN caveat): `smelt table shows UNKNOWN for a column fed by a smelt.functions.* call → known cosmetic gap...` — Phase 2 fixes this; the caveat is no longer accurate.
- **Keep line ~160** (`Expr<Boolean>` composes inside `CASE WHEN` / `SUM(CASE WHEN ...)`) — this is still accurate and useful.
- **Keep line ~161** (function `-> Expr<Double>` forces the column to `DOUBLE`) — this is now true for a better reason (return type propagation per spec rule 16); the practical guidance is unchanged.

### Critical files

- `docs-site/docs/reference/cli.md` — add `smelt table` entry (DOCS-B)
- `docs-site/docs/guide/functions.md` — add `--show-plan` verification subsection (DOCS-C) and return-type propagation sentence (DOCS-D)
- `tests/agent-loop/fixtures/medium/spec.md` — fix `../validate.py` instruction (HARNESS-1)
- `.claude/skills/smelt-app-builder/SKILL.md` — remove UNKNOWN caveat (after Phase 2)

**Commit (Phase 3):** `docs: smelt table reference entry, functions guide --show-plan tip, harness validate.py fix, skill cleanup`

---

## Original findings (reference)

### TB-A: Build path accepts wrong-prefix function calls

Source: all 5 medium-tier iterations, confirmed by iteration 1 reviewer reproduction steps.

The LSP path (`smelt_fn_call_diagnostics_for_file`) correctly enforces the no-stem call-path rule after Phase 2 of `20260502-smelt-loop-findings.md`. The `smelt build` execution path (which the reviewer confirmed by modifying the agent's project in place) silently accepts both `smelt.functions.safe_revenue(...)` and `smelt.functions.revenue.safe_revenue(...)`. The fixture spec (line 57-59 of `medium/spec.md`) instructs agents to use the stem-included form; because the build succeeds with it, all 5 iterations pass eval using the technically-wrong call form.

### TB-B: Function `-> Expr<T>` return type not reflected in `smelt table`

Source: iterations 1, 3, 4 (suspected tool bug), iteration 2 (doc gap framing). All four independently observed:

```bash
smelt table stg_orders     # amount   UNKNOWN  (expected: DOUBLE)
smelt table int_orders_by_day  # total_revenue  BIGINT  (expected: DOUBLE)
duckdb orders.duckdb -c 'DESCRIBE stg_orders'  # amount  DOUBLE  (physical type correct)
```

### TB-C: `SUM(function-call)` infers as `BIGINT` instead of `DOUBLE`

Downstream of TB-B: when `safe_revenue(...)` is typed as `Unknown`, `SUM(Unknown)` falls back to `BIGINT`. Once TB-B is fixed, TB-C should resolve automatically.

### DOCS-B: `smelt table` undocumented in `reference/cli`

Iteration 1 reviewer flagged. The subcommand exists and is recommended by the skill; the reference page has no entry for it.

### DOCS-C: `guide/functions` missing `--show-plan` verification guidance

Iterations 2, 3 retros and iteration 3 reviewer flagged. The skill mentions `--show-plan`; the official docs do not cross-reference it in the context of function debugging.

### DOCS-D: Function return-type → `smelt table` behavior undocumented

Iteration 4 retro specifically noted the divergence would be useful to document regardless of whether it is a bug or a known gap. After Phase 2 fixes it, a positive documentation of the correct behavior (declared return type is the canonical column type) prevents future confusion.

### HARNESS-1: `../validate.py` not found at runtime

Iterations 3 and 5 retros flagged. The spec instructs agents to run `python ../validate.py`; the harness places the script at a different path and runs it itself. The mismatch causes a dead-end error that distracts agents.

### FIX-A: Medium fixture teaches wrong call-path form

`tests/agent-loop/fixtures/medium/spec.md` lines 56-59 instruct agents to call `smelt.functions.revenue.safe_revenue(o.amount)` (stem-included). This contradicts `docs-site/docs/guide/functions.md` (which the spec says to read) and the `docs/specs/functions.md` table. Fixed in Phase 1 atomically with the build-path enforcement.

## Deferred

- **TB-B/C cosmetic gap in LSP / `smelt type`** — the Salsa-based `model_schema()` query may have a separate code path from the build-path schema used by `smelt table`. If tests pass but `smelt type` still shows `UNKNOWN`, a follow-up note in the Phase 2 commit should flag it for a separate fix.
- **`guide/functions` full worked example** — iteration 5 retro noted the function guide never shows a complete worked example (define + call-in-model + `smelt build` output). Deferred; tracked separately as DG-E if prioritised.
- **`smelt table` type divergence note in `guide/seeds`** — iteration 3 reviewer suggested `guide/seeds` cross-link to `guide/functions` for the case where a function-typed column overrides a seed-derived type. Low priority; deferred.
