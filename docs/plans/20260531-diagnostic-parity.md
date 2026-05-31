# Plan: Diagnostic Parity (analysis ↔ build) + function/meta codegen correctness

**Date**: 2026-05-31
**Spec**: `docs/specs/architecture.md` §"Diagnostic parity rule (analysis ↔ build)" (+ §"Run pipeline parity rule (CLI ↔ UI)"); `docs/specs/meta_language.md` (Surface: build-path execution, added in Wave C phases)
**Spec diff**: added §"Diagnostic parity rule (analysis ↔ build)"; sharpened the pre-execution gate reference in §"Run pipeline parity rule"; Known Divergences entry recording the current drift this plan closes.
**Tracking branch**: `worktree-test_features`
**Docs**: code+docs (each Wave C phase lands the `meta_language.md` Surface increment for the construct it executes).

## Execution prompt (for a fresh session / autonomy iteration)

Read this file. Run the next `pending` phase in the Progress-tracking table using the per-phase routine below (pre-flight green → spec increment if listed → red-green `/smelt:implement` on the phase's tests, spec as oracle, implementer + reviewer → verification gates → update the table row → commit + push with the phase's commit message). Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<ALL_DONE>>`, or `<<PAUSE_FOR_HUMAN>>`.

## Goal

Make `smelt build`/`run` enforce exactly what the analysis layer (the LSP/`file_diagnostics`) sees, and make every analyzer-accepted construct compile to valid engine SQL. This closes the "LSP-clean but unbuildable" bug class surfaced by the feature sweep: BUG-011, 013, 015, 018, 019, 024, 032 (bounded) and BUG-006 (the meta-language evaluator, multi-phase).

Two guarantees, per the spec rule:
1. **Parity gate** — before any model is compiled/executed, the build runs `file_diagnostics` over the selected models + in-DAG deps and fails fast on any `Error`-severity diagnostic, through one shared gate for both the CLI and `execute_project`. The blocking set is exactly `severity == Error` (not a code allow-list).
2. **Codegen honors the analyzer** — constructs the analyzer accepts (nested `smelt.define`, block `PASSING` fragments, in-model meta) compile to valid SQL rather than being emitted verbatim/dropped.

## Non-goals (deferred, tracked separately)
- Full CLI→`execute_project` migration (BUG-001). This plan reaches parity via a **shared gate helper** both run paths call; the single-entry-point migration is a later effort. The `execute_parity` CI gate named in the spec remains future work beyond the shared gate.
- Backend coverage beyond DuckDB.
- A post-expansion `analyze_sql_string` diagnostic query (a backstop). Source-level gating + correct codegen cover this plan's scope; revisit only if a gap remains.
- Demoting/retuning individual diagnostic *codes* beyond what the gate requires (only bump Warning↔Error where a phase's red-green test proves the verdict is wrong).

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If already red, emit `<<PAUSE_FOR_HUMAN>>`.
2. **Spec increment** (only the phases that list one): edit the named spec section first; keep it timeless (no phase vocabulary in the spec body).
3. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, with the spec as oracle. Implementer pass, then reviewer pass (material findings only).
4. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; plus the phase's specific gates: `example_diagnostics`, `example_workspaces`, `smelt-runtime` parity tests. For `example_builds`: run it **scoped to only the workspace(s) this phase touches** — `SMELT_EXAMPLE_BUILDS_ONLY="<ws1>,<ws2>" cargo test -p smelt-cli --test example_builds` — because a clean-copy build + DuckDB execution of the whole example set is the gate's dominant cost. The **full** sweep (var unset) runs only in P8 / CI; do not run it unscoped in P2–P7.
5. **Record + commit.** Update the status-table row to `done` + date; commit and push tests + impl + spec + table together with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or `<<ALL_DONE>>` on the last green phase).

## Pause conditions (`<<PAUSE_FOR_HUMAN>>`)
- Pre-flight already red (no clean baseline).
- The tree can't be returned to green after the phase.
- The phase needs a design decision not answered by this plan or the spec (do not guess on architecture-invariant-touching choices; pause with a one-line reason).

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | `example_builds` harness (build+execute every example; discover & allow-list the currently-unbuildable, each with a logged reason) | done | — (structural) | | 2026-05-31 |
| P2 | Shared `Error`-severity diagnostic gate, wired into both the CLI run path and `execute_project` | pending | BUG-011, 015, 019, 024, 032 | | |
| P3 | BUG-013: expand nested `smelt.define` calls to a fixpoint | pending | BUG-013 | | |
| P4 | BUG-018: thread block `PASSING` fragment bindings through substitution | pending | BUG-018 | | |
| P5 | BUG-006a: in-model list spread executes at build (meta_lists) | pending | BUG-006 (lists) | | |
| P6 | BUG-006b: HOFs/pipe/lambda + ternary + `config.var` execute at build (meta_hofs, meta_polish) | pending | BUG-006 (hofs) | | |
| P7 | BUG-006c: reflection (`columns_of`, `models.with_tag`, wide reflection) + `config.loader` execute at build (meta_columns, meta_workspace, meta_config) | pending | BUG-006 (reflection/config) | | |
| P8 | Close-out: `example_builds` allow-list empty (only intentional `*_broken_*` remain); remove the resolved Known Divergence from architecture.md; update ROADMAP + bug ledger statuses | pending | — | | |

**Status values**: `pending` → `done`. A phase is `done` only when its tests are red-green confirmed and all gates (incl. `example_builds`) are green.

## Phase detail

### P1 — `example_builds` harness
- **Spec**: none (the spec's CI-gate sentence already names `example_builds`).
- **Tests (red-green)**: new `crates/smelt-cli/tests/example_builds.rs`. For every `examples/*` workspace, run `smelt build` (compile **and** execute on DuckDB). A workspace that builds clean must pass; a workspace on the `KNOWN_UNBUILDABLE` allow-list is skipped **with its failure reason logged** (no silent skip); a `*_broken_*` workspace asserts its expected `Error` diagnostic at the gate rather than executing. Land green: discover the currently-unbuildable set empirically (expect `functions_demo`, `meta_lists`, `meta_hofs`, `meta_columns`, `meta_config`, `meta_polish`, `meta_workspace`) and allow-list them with the observed error. Later phases remove entries.
- **Commit**: `test(diag-parity): add example_builds gate (build+execute every example) with logged known-unbuildable allow-list`

### P2 — Shared `Error`-severity diagnostic gate
- **Spec**: `architecture.md` §"Diagnostic parity rule" (already added) is the oracle.
- **Tests (red-green)**: pick `*_broken_*` fixtures whose error is **not** `UnknownSmeltFn` — at least one each for the codes behind BUG-019 (`CteCycle`), BUG-024 (`MalformedTimeseries`), BUG-011 (a cumulative classifier code), BUG-015 (a loader content-validation code), BUG-032 (a malformed per-entity source). Each test asserts `smelt build` exits non-zero **and** names the expected `DiagnosticCode`, where today it exits 0 / mis-builds. Add a positive test: a clean fixture still builds. Confirm every existing example remains buildable (they are analysis-clean).
- **Implementation**: a shared helper (e.g. `smelt_runtime::gate_diagnostics(db, workspace, &selected) -> Result<(), Vec<Diagnostic>>`) that collects `file_diagnostics` for each selected model + in-DAG deps, keeps `severity == Error`, returns the aggregated set. Call it at the top of `execute_project` and from the CLI run path, **replacing** the `UnknownSmeltFn`-only block at `crates/smelt-cli/src/commands/run.rs:677-701`. Report all errors with `file:line` + code at the boundary (Diagnostic range encoding rule). If a phase fixture's code is currently `Warning` but the test proves it must block, bump it to `Error` and note it in the commit.
- **Commit**: `feat(runtime): gate build on all Error-severity diagnostics via one shared CLI/UI helper (closes BUG-011/015/019/024/032)`

### P3 — BUG-013 nested `smelt.define` fixpoint
- **Tests (red-green)**: build+execute a nested-composition example (`examples/functions_demo/models/uses_nested_helpers.sql`, or a minimal 3-level chain) and assert correct output, where today DuckDB errors `Catalog "smelt" does not exist`. Remove the relevant entry from `KNOWN_UNBUILDABLE` if `functions_demo`'s only blockers were 013/018.
- **Implementation**: make function-call expansion reach a fixpoint — re-expand nested `SMELT_PATH_CALL` nodes left in a substituted body (thread the expanders through `substitute_params_with_named`, or loop the printer pass until stable). See `crates/smelt-dialect/src/printer.rs:205-214`, `crates/smelt-runtime/src/compile.rs:255-297`.
- **Commit**: `fix(dialect): expand nested smelt.define calls to a fixpoint (closes BUG-013)`

### P4 — BUG-018 block `PASSING` fragments
- **Tests (red-green)**: build+execute `examples/functions_demo/models/rollup_with_passing.sql` and assert correct output, where today the fragment is emitted as its bare name/default → DuckDB error. Remove `functions_demo` from `KNOWN_UNBUILDABLE`.
- **Implementation**: collect `path_call.passing_clauses()` in the printer, extend the `SmeltPathCallExpander` signature with a PASSING-binding slot, and merge PASSING bindings into substitution alongside named args. See `crates/smelt-runtime/src/compile.rs:134-146`, `crates/smelt-dialect/src/printer.rs:144-175`, `crates/smelt-parser/src/ast.rs:726`.
- **Commit**: `fix(dialect): bind block PASSING fragment arguments in substitution (closes BUG-018)`

### P5 — BUG-006a list spread executes at build
- **Spec**: `meta_language.md` Surface — list spread is evaluated at compile time and emits plain SQL columns/items.
- **Tests (red-green)**: build+execute `examples/meta_lists` and assert the expanded SELECT output (columns + rows). Negative `meta_lists_broken_*` fixtures continue to produce their diagnostics at the gate. Remove `meta_lists` from `KNOWN_UNBUILDABLE`.
- **Implementation**: a compile-time meta-expansion pass (CST→CST rewrite) run in `compile.rs` before printing, with a `MetaEvalContext` (type context + upstream schemas + config), starting with `LIST_SPREAD` of list literals/variables in SELECT lists. Reuse `smelt-db`'s spread validation (`type_inference/hof.rs:check_select_list_spreads`) for shape/typing; add the *expansion* it lacks.
- **Commit**: `feat(runtime): evaluate in-model list spread at compile time (closes BUG-006 lists)`

### P6 — BUG-006b HOFs/pipe/lambda + ternary + config.var
- **Spec**: `meta_language.md` Surface — HOFs (`map`/`filter`/`reduce`), pipe, lambdas, ternary, and `smelt.config.var` evaluate at compile time.
- **Tests (red-green)**: build+execute `examples/meta_hofs` and `examples/meta_polish`; assert outputs. Negative `*_broken_*` fixtures still gate. Remove both from `KNOWN_UNBUILDABLE`.
- **Implementation**: extend the meta-eval pass with lambda binding + fold/map/filter evaluation, pipe desugaring, ternary, and compile-time `config.var` resolution from `smelt.yml`.
- **Commit**: `feat(runtime): evaluate HOFs/pipe/lambda/ternary/config.var at compile time (closes BUG-006 hofs)`

### P7 — BUG-006c reflection + config.loader
- **Spec**: `meta_language.md` Surface — `smelt.columns_of`, `smelt.models.with_tag`, wide reflection, and `smelt.config.loader` evaluate at compile time.
- **Tests (red-green)**: build+execute `examples/meta_columns`, `examples/meta_workspace`, `examples/meta_config`; assert outputs. Negative fixtures still gate. Remove all three from `KNOWN_UNBUILDABLE`.
- **Implementation**: extend the meta-eval pass with schema/workspace reflection (materialize column metadata from upstream schemas; resolve `with_tag` against the workspace) and `config.loader` file resolution at compile time.
- **Commit**: `feat(runtime): evaluate reflection + config.loader at compile time (closes BUG-006 reflection/config)`

### P8 — Close-out
- **Tests**: `example_builds` `KNOWN_UNBUILDABLE` is empty except intentional `*_broken_*` markers; full suite + all gates green.
- **Docs**: remove the now-resolved drift bullet from `architecture.md` Known Divergences; update `docs/ROADMAP.md`; flip BUG-006/011/013/015/018/019/024/032 to `fixed` in `docs/bug-hunt/2026-05-30-findings.md` with their regression-test names.
- **Commit**: `docs(diag-parity): close out — examples all build, ledger + spec + roadmap updated`

## Verification
- Every table row `done`.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets` (no warnings), `cargo test`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-cli --test example_builds`, `cargo test -p smelt-lsp --test example_workspaces`, `cargo test -p smelt-runtime` all green.
- Each `Closes` bug has a red-green regression test (a `*_broken_*` build-gate assertion or a build+execute output assertion), confirmed red on `git stash` of the fix.
