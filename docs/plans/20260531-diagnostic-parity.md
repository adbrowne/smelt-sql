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
- **User-authored planner-rule *registration*** (the extensibility feature: a project shipping its own planner rule). The diagnostic surface, including planner rules, is *in scope* — every built-in rule is surfaced into `file_diagnostics` via the uniform rule → diagnostics interface (P2b) and gated like any other `Error` (§"Diagnostic parity rule (analysis ↔ build)" Scope, §"Planner scope"). What is out of scope is the mechanism for registering *new, non-built-in* rules; when that lands it reuses the same interface and inherits parity for free. BUG-011 is therefore in scope (surface the cumulative classifier into `file_diagnostics`), not deferred.

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
| P2 | Shared `Error`-severity diagnostic gate, wired into both the CLI run path and `execute_project` | done | BUG-015, 019, 024 | feat(runtime): gate build on all Error-severity diagnostics via one shared CLI/UI helper | 2026-06-01 |
| P2b | Uniform planner rule → diagnostics interface; surface built-in rules (cumulative classifier; incremental batch-safety/bounds) into `file_diagnostics` | done | BUG-011 | feat(planner,db): surface built-in planner-rule diagnostics via a uniform rule→diagnostics interface in file_diagnostics (closes BUG-011) | 2026-06-01 |
| P2c | Source-diagnostics producer: surface malformed per-entity source YAML (`MalformedSource`) into the analyzer surface, gated like any `Error` and published by the LSP | done | BUG-032 | feat(db,runtime,lsp): surface malformed per-entity source YAML as MalformedSource diagnostics (closes BUG-032) | 2026-06-01 |
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
- **Tests (red-green)**: pick `*_broken_*` fixtures whose error is **not** `UnknownSmeltFn` — at least one each for the codes behind BUG-019 (`CteCycle`), BUG-024 (`MalformedTimeseries`), BUG-015 (a loader content-validation code), BUG-032 (a malformed per-entity source). Each test asserts `smelt build` exits non-zero **and** names the expected `DiagnosticCode`, where today it exits 0 / mis-builds. Add a positive test: a clean fixture still builds. Confirm every existing example remains buildable (they are analysis-clean). (The cumulative classifier codes behind BUG-011 are added to `file_diagnostics` in P2b, then flow through this same gate.)
- **Implementation**: a shared helper `smelt_runtime::gate_diagnostics(db, workspace, &files) -> Result<(), Vec<GateDiagnostic>>` (in `crates/smelt-runtime/src/gate.rs`) that aggregates the **full LSP surface** — `file_diagnostics` **plus** `check_type_diagnostics::accumulated` (matching `smelt-lsp`'s `diagnostics_for`) — over each file, keeps `severity == Error`, and resolves each range to 1-based `(line, col)` once via `line_index::LineIndex` (the build gate's single boundary; Diagnostic range encoding rule). `GateDiagnostic`/`format_gate_errors` emit `path:line:col: error[Code]: message`. Wired into **both** run paths: the CLI runs it *before* dependency validation (so a loader/timeseries/scoping defect is rejected by its real code, not a downstream "undefined ref" or DuckDB binder error) over a gate DB built from the **discovered** files — raw SQL models incl. `*.gen.sql` generator files (so generator-emitted refs resolve through `emitted_models`) plus function-definition files (so `CteCycle`-style function-body diagnostics are covered) — **replacing** the `UnknownSmeltFn`-only block at `run.rs`; `execute_project` calls the same helper at the top over its selected models. Side effect of parity: `per_cohort_union_broken_emission_body_collision_suppression` (a `ModelDefDuplicateName` `Error`) no longer builds clean — it moves out of `example_builds`' `BROKEN_BUILDS_CLEAN` into the `broken`-analyzer-Error category.
- **BUG-032 split out → P2c.** The plan assumed `MalformedSource` already surfaced in `file_diagnostics` (gating-only). It does not — `MalformedSource` has **zero producers**; per-entity source `.yml` files are not registered as `SourceFile`s, and `discover_source_infos` swallows parse errors (`crates/smelt-core/src/sources.rs:272` `Err(_) => continue`). Closing BUG-032 needs a *new* source-diagnostics producer, not just the gate, so it is its own phase (P2c) — mirroring the earlier P2/P2b split for BUG-011. The gate built here catches BUG-032 for free once P2c's producer emits the `Error`.
- **Commit**: `feat(runtime): gate build on all Error-severity diagnostics via one shared CLI/UI helper (closes BUG-015/019/024)`

### P2c — Source-diagnostics producer (BUG-032)
- **Spec**: `architecture.md` §"Diagnostic parity rule" (analysis must see what the build's source discovery sees) + §"Unknown-key doctrine" (per-entity source YAML is strict). Per-entity source discovery is the lazy Salsa `project_sources` path (§"Workspace loading parity rule").
- **Context**: `MalformedSource` (`diagnostics_types.rs:21`) has no producer; `parse_source_yaml` (`sources.rs:157`) can fail with `Io / YamlParse / MissingColumns / MaterializationForbidden / UnknownType / InvalidNameOverride`, all discarded by `discover_source_infos` at `sources.rs:272`. A malformed per-entity source with no consumer builds at exit 0 today (LSP-clean); with a consumer it fails downstream with a misleading "schema does not exist".
- **Tests (red-green)**: a `*_broken_*` per-entity-source fixture (bad/missing type, missing `columns:`, `materialization:` present) produces a `MalformedSource`-mapped `Error` via `file_diagnostics`/the diagnostics API where today it is diagnostic-clean; the same fixture then fails `smelt build` at the P2 gate naming the code (parity end-to-end); a valid per-entity source stays clean in both surfaces.
- **Implementation**: a new analysis-pure source-diagnostics producer in `smelt-db` (a project-scoped query over the per-entity source YAML the workspace already discovers, mapping each `SourceError` variant to a `MalformedSource` diagnostic with a range anchored in the offending file) merged into the analyzer surface the gate and LSP consume. Keep the parse/validation logic in `smelt-core::sources`; the query only aggregates (Salsa purity rule).
- **As-built notes**:
  - **smelt-core**: factored the source/sidecar disambiguation walk into a private `candidate_source_yaml_files` shared by `discover_source_infos` (parses → `SourceInfo`) and the new `discover_source_errors` (collects `(PathBuf, SourceError)` for files that fail `parse_source_yaml`). The walk, sibling-`.csv` skip, address recomputation, and sort order are byte-for-byte the prior behaviour.
  - **smelt-db**: new `#[salsa::tracked] project_source_diagnostics(project) -> Arc<Vec<SourceDiagnostic>>` (keyed on `ProjectInput`, restart-scoped like `project_sources`). It only aggregates — validation stays in `smelt-core`. **Code mapping honours the spec, not the plan's "all → MalformedSource":** `sources.md` §"Diagnostic codes" splits an unrecognised column type out as `SourceTypeError`, so `SourceError::UnknownType → SourceTypeError`; every other variant (`Io`/`YamlParse`/`MissingColumns`/`MaterializationForbidden`/`InvalidNameOverride`) → `MalformedSource`. Range anchored at the file head (offset 0).
  - **Gate (smelt-runtime)**: `gate_diagnostics` now also gates the sources of every project that owns at least one gated file (so `--select` of a model enforces its project's sources, but an unrelated project in a multi-project workspace is not dragged in). Offset-0 ranges resolve to `(1, 1)` unconditionally.
  - **LSP parity (smelt-lsp)**: the editor half of the parity rule needed wiring — `.yml` source files are not tracked `SourceFile` inputs, so they never flowed through the per-file `diagnostics_for` path and a malformed source was build-refused but editor-green (the inverse parity gap §"Diagnostic parity rule" forbids). Added `Backend::publish_source_diagnostics`, published project-scoped to each offending `.yml`'s own URI at `initialized` (**before** the `register_capability` round-trip, which awaits a client reply). Source discovery is restart-scoped, so a single startup publish is its lifecycle.
  - **Fixture cleanup**: the producer surfaced a genuinely-malformed, **unused** file — `examples/meta_config/models/sources/sources.yml` used the obsolete aggregate `sources:`/`tables:`/`schema:` format (no top-level `columns:`) and was referenced by nothing. Removed it (the empty `sources/` dir with it); `meta_config` is clean again on both surfaces.
  - **Tests**: `crates/smelt-cli/tests/source_diagnostics.rs` (analysis-layer query + end-to-end `smelt build` refusal naming the code + a valid-source clean case) and `sources_broken_malformed_surfaces_via_lsp` in `crates/smelt-lsp/tests/example_workspaces.rs` (LSP publishes `malformed-source` on the `.yml` URI). New fixture `examples/sources_broken_malformed/` (a `materialization:`-bearing source + a trivial probe model).
- **Commit**: `feat(db,runtime,lsp): surface malformed per-entity source YAML as MalformedSource diagnostics (closes BUG-032)`

### P2b — Uniform planner rule → diagnostics interface
- **Spec**: `architecture.md` §"Planner scope" (rule → diagnostics interface) + §"Diagnostic parity rule" Scope (both already added) are the oracle.
- **Context**: the cumulative classifier is `smelt_planner::classify_cumulative` (`crates/smelt-planner/src/rules/cumulative.rs:227`), returning `Result<CumulativeClassification, Vec<CumulativeDiagnostic>>`. `smelt-db` already depends on `smelt-planner` (compile-time edge; the reverse is dev-only), so `file_diagnostics` can call it directly — no new dependency. Today the classifier runs only on the build/dispatch path (`smelt-runtime` via `smelt_planner::classify_cumulative`), so a malformed cumulative model is LSP-clean (BUG-011).
- **Tests (red-green)**: (1) a `cumulative_*_broken_*` fixture (e.g. missing GROUP BY / forbidden aggregate) now produces its `CumulativeDiagnostic`-mapped code via `smelt_db::file_diagnostics` — assert in an `smelt-db`/`example_diagnostics`-style test where today the model is diagnostic-clean. (2) The same fixture fails `smelt build` at the P2 gate naming that code (parity end-to-end). (3) A valid cumulative example stays clean in both surfaces.
- **Implementation**: introduce a uniform rule → diagnostics interface in `smelt-planner` (a `detect`-shaped function/trait each built-in rule implements, returning `Vec<Diagnostic>` or rule-native diagnostics mappable to `smelt_db::Diagnostic`). Route the cumulative classifier and the incremental batch-safety/bounds analyzers through it. Have `file_diagnostics` invoke the built-in rules over each model and merge their diagnostics, mapping each to its correct severity (the cumulative classifier's reject codes are `Error`; advisory incremental findings stay `Warning`). Keep the checks in the rule (analysis-pure); `file_diagnostics` only aggregates. The runtime continues to call the same classifier, so the build/dispatch verdict is unchanged — it is now *also* visible to the editor.
- **As-built notes**:
  - New `smelt_planner::rule_diagnostics` module: `PlannerRule` trait + `RuleContext`/`RuleDiagnostic`/`RuleDiagnosticCode`/`RuleSeverity` + `detect_builtin_rules` + the shared `collect_path_refs` (moved out of `smelt-runtime::cumulative` so the runtime dispatch and the analysis gate share one ref scanner). `CumulativeRule` → `Error`; `IncrementalRule` → `Warning` (the build path uses `analyze_batch_safety`, which never hard-refuses, so advisory `Warning` is the parity-correct severity — `incremental::detect` is not a build gate). 9 new `DiagnosticCode` variants + LSP code-string arms.
  - `file_diagnostics` builds the cumulative driving-source `smelt.<path> → timeseries` map by resolving each ref (handles single / multi-section / generator-emitted models, mirroring the runtime's graph-derived map) to avoid a spurious `CumulativeNoDrivingSource`.
  - **Gate-scoping correction (CLI `run.rs`)**: surfacing the cumulative `Error` exposed that the P2 CLI gate ran over the *whole* discovered project, so `--select`-ing a clean model was refused by an unrelated broken model. Per §"Diagnostic parity rule" ("selected models + in-DAG deps"), the gate was moved to after `execution_order` is computed and scoped to that set (+ all function-definition and `*.gen.sql` generator files, always); `graph.validate()` now runs *after* the gate so an analyzer `Error` is still reported by its real code before any downstream dependency error (preserves BUG-015/019). `smelt build` (no `--select`) is unchanged (all models selected). `execute_project` (UI path) already scoped to selected models.
  - Known limitation (deferred): materialization detection keys off frontmatter (`metadata.materialization` / `metadata.incremental`); a `cumulative_aggregate`/`incremental` set only in `smelt.yml` is not yet surfaced in the editor (under-reporting, not a false positive).
- **Commit**: `feat(planner,db): surface built-in planner-rule diagnostics via a uniform rule→diagnostics interface in file_diagnostics (closes BUG-011)`

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
