# Plan: Codegen Soundness — CTE collisions diagnosed, TableExpr `source.*` emits valid SQL

**Parent (master plan)**: `docs/plans/20260530-feature-sweep.md` — a **sub-plan** spawned from the feature sweep to remediate the "codegen soundness" cluster of its ledger findings: **BUG-007** (soundness — silent wrong data), **BUG-009** (schema/codegen disagreement), and the bundled doc fix **BUG-008**. The autonomy loop works this sub-plan phase by phase and rolls up to the master only when it is exhausted.

**Date**: 2026-06-04
**Spec**: `docs/specs/expansion.md` §"Hygiene v1" (CTE-collision diagnostic mandate, lines ~21, 89–90; the `<generator>` frame note ~142); `docs/specs/scoping.md` §"Diagnostic codes" (where `CteCycle` lives); `docs/specs/function_schema_inference.md` §Invariant #2 (schema/codegen agreement) + Semantics rule 3 (`source.*`).
**Spec diff**: (1) add the mandated CTE-collision code `CteShadowsCallerCte` to `scoping.md`'s diagnostic table and narrow the `expansion.md` Known Divergence that records it as not-yet-minted; (2) correct the stale `make_generator_frame` signature in `expansion.md` (3-arg → 2-arg). No new behavior is *invented* — both code fixes honor invariants the specs already state.
**Tracking branch**: `worktree-test_features`
**Docs**: code+docs.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file. Run the next `pending` phase in the Progress-tracking table (skip `done` and `blocked` rows) using the per-phase routine below (pre-flight → spec increment if listed → red-green `/smelt:implement` on the phase's tests, spec as oracle, implementer + reviewer → verification gates → update the table row → commit + push with the phase's commit message). Emit exactly one sentinel: `<<PHASE_COMPLETE>>` (phase done), `<<PHASE_BLOCKED>>` (decision/off-target-red recorded; see §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (sub-plan exhausted; see the loop's roll-up rule), or `<<ALL_DONE>>`. There is no hard-stop: a block is recorded and the loop continues to the next pending phase.

## Goal

Close the two **silent-until-`smelt run`** codegen defects in function expansion — the worst class in the ledger because `smelt type` reports clean while the build is wrong:

- **BUG-007 (soundness)** — a function-body CTE colliding with a **caller's** CTE name silently shadows and emits **wrong data**. The spec mandates a codegen-time collision *diagnostic* (no rename in v1); it does not exist. Add it.
- **BUG-009** — a `TableExpr` function with a `source.*` body, called with a `smelt.<path>` argument, substitutes the parameter for the qualified physical name and emits over-qualified `schema.table.*`, which DuckDB rejects (`syntax error at or near "*"`). The schema layer resolves correctly, so this is a pure codegen defect violating Invariant #2 (schema/codegen agreement). Emit valid SQL.
- **BUG-008 (bundled doc fix)** — `expansion.md` documents `make_generator_frame` with a stale 3-arg signature (`…, file_text`); the code is 2-arg.

## Design decisions (resolved — do not re-litigate; from spec + the layer analysis)

- **BUG-007 is diagnostic-only, `Error`, refuse — no hygiene/auto-rename** (spec-mandated). `expansion.md` §"Hygiene v1" rule 3: *"When a function body declares a CTE whose name would collide with a CTE in the caller's scope at codegen time, the compiler emits a collision diagnostic anchored at the body CTE's declaration. v1 makes no attempt to rename."* So we emit a diagnostic and refuse; auto-rename/alpha-hygiene stays a future (v2) item.
- **BUG-007 check runs at ANALYSIS time, not in the printer** (the load-bearing call). Both inputs are statically known during model analysis: the model's own top-level CTE names (the caller scope) and, for each transparent function the model calls, that function body's CTE names (already analysis-accessible — `CteCycle` is detected on those bodies today in `function_body_check.rs`). So a Salsa `file_diagnostics` check computes the overlap and emits `CteShadowsCallerCte` (Error), anchored at the body CTE declaration with a frame at the call site. *Rationale*: this flows through the **existing diag-parity P2 Error-gate** (parity-clean) and needs **no** new diagnostic-emission pathway in the planner/printer (which has none today). v1 scope is **direct** collisions (model CTEs ⊗ directly-called function body CTEs); deeper transitive-expansion collisions, if they surface, are recorded as a Known Divergence rather than blocking.
- **BUG-007 code name `CteShadowsCallerCte`**, owned by `scoping.md`'s diagnostic table (where `CteCycle` lives), following the `ParameterShadowsColumn` "name both parties" precedent. Reusing `CteCycle` is rejected — it is a different defect (within-body recursion vs cross-scope shadow).
- **BUG-009 fix = alias the argument at the splice, keep the body verbatim** (`FROM <arg-physical-name> AS <param>`), rather than text-stripping the wildcard qualifier. *Rationale*: aliasing keeps every body reference (`source.*`, `source.col`, `FROM source`) valid uniformly and is robust to all argument forms; qualifier-stripping (`main.base.*` → `base.*`) is fragile for non-star qualified refs and complex arguments. The TableExpr parameter is bound by aliasing the FROM target, so the parameter identifier is **not** blindly text-replaced into qualified positions. The CTE-argument case (arg already a named CTE) keeps working (aliasing a CTE is valid).
- **Gating relationship.** BUG-007's analysis diagnostic is caught by the P2 Error-gate automatically. BUG-009 produces **no** analysis diagnostic (the schema layer is correct) — it is a *codegen correctness* fix, so its regression guard is a build+execute fixture (the emitted SQL runs on DuckDB), not the diagnostic gate.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red, check **what** is red: if the failure is this phase's own acceptance target (the fixture/test this phase exists to make green), that is expected — **proceed**. If the red is unrelated breakage, treat it as a block (record + continue, per §"Block conditions"); do not build on a broken baseline.
2. **Spec increment** (only the phases that list one): edit the named spec section first; keep it timeless (no phase vocabulary in the spec body).
3. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, spec as oracle. Implementer pass, then reviewer pass (material findings only).
4. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; plus the phase's gates: `example_diagnostics`, `example_workspaces`, and the function codegen/e2e suites. For `example_builds`: run **scoped** — `SMELT_EXAMPLE_BUILDS_ONLY="<ws…>" cargo test -p smelt-cli --test example_builds`. The full sweep runs only in C4 / CI.
5. **Record + commit.** Update the status-table row to `done` + date; commit and push tests + impl + spec + table together. Emit `<<PHASE_COMPLETE>>` (or `<<ALL_DONE>>` on the last green phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue, no hard-stop)
When a phase hits a condition below, **do not halt**. Instead: (1) set the row to `blocked` with a one-line reason; (2) append a dated entry to §"Blocked phases" (phase id, reason/decision, candidate options); (3) restore the tree to a clean committed state; (4) commit + push; (5) emit `<<PHASE_BLOCKED>>`. The next iteration skips the blocked row.

Conditions:
- The phase needs a design decision **not** answered by this plan (the decisions above are settled) or the spec — e.g. if the analysis-time BUG-007 check proves infeasible for a needed call shape, or alias-injection regresses a function form in a way that needs a product call.
- Pre-flight is red on **unrelated** breakage (not this phase's own target).
- The tree can't be returned to green after the phase.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| C1 | BUG-008 doc fix: correct `make_generator_frame` signature in `expansion.md` (3-arg → 2-arg) | pending | BUG-008 | | |
| C2 | BUG-009: TableExpr `source.*` over a `smelt.<path>` arg emits valid SQL — alias the argument at the splice (`FROM <arg> AS <param>`), keep the body verbatim | pending | BUG-009 | | |
| C3 | BUG-007: analysis-time CTE-collision check — mint `CteShadowsCallerCte` (Error) for a model CTE colliding with a directly-called function body CTE; spec increment (scoping.md table + narrow expansion.md divergence) | pending | BUG-007 | | |
| C4 | Close-out: flip BUG-007/008/009 to `fixed` in the ledger with regression-test names; update master sub-plan table + ROADMAP; full `example_builds` + all gates green | pending | — | | |

**Status values**: `pending`, `done`, `blocked`. A phase is `done` only when its tests are red-green confirmed and all gates are green. A `blocked` phase has a dated §"Blocked phases" entry and returns to `pending` once a human resolves it.

## Blocked phases

Append-only log of phases the loop recorded as `blocked` and continued past. Each entry: date, phase id, reason/decision, candidate options. *(None yet.)*

## Phase detail

### C1 — BUG-008 doc fix (`make_generator_frame` signature)
- **Change**: in `docs/specs/expansion.md` (~line 142, the `<generator>` frame Known-Divergence note), correct `make_generator_frame(path, body_range, file_text)` → `make_generator_frame(path, body_range)` to match `crates/smelt-db/src/function_body_check.rs:357`. Pure retraction, no behavior change.
- **Tests**: none (doc-only); the existing 2-arg call sites are the implicit oracle. Verify the workspace still builds.
- **Commit**: `docs(expansion): correct make_generator_frame signature to 2-arg (closes BUG-008)`

### C2 — BUG-009: valid SQL for TableExpr `source.*` (alias the argument)
- **Change**: when a transparent `TableExpr` function call binds a `smelt.<path>` argument to a parameter used as a table (`FROM <param>` / `<param>.*` / `<param>.col`), bind it by **aliasing the argument's physical name to the parameter** (`FROM <arg-physical-name> AS <param>`) instead of text-replacing `<param>` with the qualified name. The body's `<param>.*` then stays a valid single-part wildcard. Lives where the splice/substitution happens (`crates/smelt-runtime/src/compile.rs` `substitute_params_with_named` and/or the planner's `ExpandTransparentFunctionCalls` in `crates/smelt-planner/src/logical_plan_rules.rs` — the implementer picks the cleanest seam, preferring a logical-plan alias annotation over a text rewrite). The CTE-argument form (arg already a named CTE) must keep working.
- **Tests (red-green)**: a new `examples/fn_tableexpr_star/` fixture — a `TableExpr` function with a `source.*` body called with a `smelt.<path>` model/source argument — **builds + executes** on DuckDB with the expected columns/values (was: `syntax error at or near "*"`). Regression: `examples/functions_demo` (the `add_margin` / `margin_via_cte` CTE-argument path) still builds + executes. Add the e2e assertion under `crates/smelt-cli/tests/`.
- **Commit**: `fix(runtime,planner): alias TableExpr arguments so source.* emits valid SQL (closes BUG-009)`

### C3 — BUG-007: analysis-time CTE-collision diagnostic
- **Spec increment (first)**: add `CteShadowsCallerCte` (Error) to `docs/specs/scoping.md`'s diagnostic-codes table with its message shape ("a CTE `{name}` in this model collides with a CTE declared in the body of called function `{fn}`; rename one — v1 does not auto-rename"); narrow the `expansion.md` Known Divergence (~line 138) that records the code as not-yet-minted.
- **Change**: mint `CteShadowsCallerCte` in `crates/smelt-db/src/diagnostics_types.rs` (alongside `CteCycle`). Add a `file_diagnostics` check: for each model, gather its top-level CTE names; for each transparent function it directly calls, gather that body's CTE names; on overlap emit `CteShadowsCallerCte` (Error), anchored at the body CTE declaration with a call-site frame. v1 = direct collisions only.
- **Tests (red-green)**: a `*_broken_*` fixture `examples/expansion_broken_cte_caller_collision/` — a function whose body declares CTE `helper`, called from a model that also declares CTE `helper` — is **build-refused** naming `CteShadowsCallerCte` (and the analyzer surfaces it, so `example_diagnostics` sees it). Positive/regression: a non-colliding body CTE builds + executes; `crates/smelt-db/tests/cte_splice.rs::cte_cycle_detected` (within-body cycle) still passes; `functions_demo` stays clean.
- **Commit**: `feat(db): diagnose function-body CTE collisions with caller CTEs at analysis time (closes BUG-007)`

### C4 — Close-out
- **Tests**: full `example_builds` (var unset) + full suite + all gates green.
- **Docs**: flip BUG-007/008/009 to `fixed` in `docs/bug-hunt/2026-05-30-findings.md` with their regression-test names; set the master plan's §"Spawned sub-plans" row to `done`; update `docs/ROADMAP.md`. Record any transitive-expansion CTE-collision gap (if found) as an explicit Known Divergence in `expansion.md`.
- **Commit**: `docs(codegen-soundness): close out — CTE collisions diagnosed, source.* valid, ledger + roadmap updated`

## Verification
- Every table row `done` (or `blocked` with a recorded entry).
- `cargo fmt --all -- --check`, `cargo clippy --all-targets` (no warnings), `cargo test`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-cli --test example_builds`, `cargo test -p smelt-lsp --test example_workspaces` all green.
- BUG-009: the new `fn_tableexpr_star` fixture executes on DuckDB (confirmed red — `syntax error at or near "*"` — on `git stash` of the fix).
- BUG-007: the broken-collision fixture is build-refused naming `CteShadowsCallerCte` (confirmed it builds with silent/wrong output on `git stash` of the fix), and `cte_cycle_detected` still passes.
