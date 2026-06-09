# Plan: Silent Failures & Code-Health Hardening

**Date**: 2026-06-08
**Spec**: n/a as a single feature — this is a code-health hardening plan. It touches the invariants in [`docs/specs/architecture.md`](../specs/architecture.md) ("Salsa purity rule", single-source invariants) and authors a new [`docs/specs/diagnostics.md`](../specs/diagnostics.md) for the diagnostic codes it introduces. The discipline itself is roadmap [What's Next #1](../ROADMAP.md#1-silent-failures--code-health-hardening).
**Spec diff**: new spec (`docs/specs/diagnostics.md`) + invariant additions to `architecture.md` / `CLAUDE.md` landed within this plan.
**Tracking PR / branch**: `worktree-test_features` (PR # TBD)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. There is no single feature spec; the correctness oracle for each phase is the **classification rule it states** (legitimate vs error `Unknown`; recoverable vs invariant panic) plus the architectural invariants in `CLAUDE.md` (Salsa purity in `type_inference.rs`; single-source invariants).
2. Confirm you are on branch `worktree-test_features`. If not, ask the user.
3. Find the next phase whose status is `pending` in the Progress tracking table. If every phase is `done`, run the post-implementation verification and stop.

**For each phase, run the per-phase loop in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- A classification is genuinely ambiguous (is this `Unknown` legitimate conservatism or a swallowed error?) and the wrong call would emit a false-positive diagnostic on a valid example.
- A panic turns out to guard a real invariant whose violation is unreachable from user input — converting it to `Result` would add noise without value.
- `cargo test` / `cargo clippy` surfaces a pre-existing failure unrelated to this plan.

**Conventions every phase:**
- Red-green TDD: a failing test before any implementation. Real-fixture coverage in `examples/` (the broken cases go in `examples/broken/`).
- The diagnostic phases must keep `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` green — a new diagnostic must fire **only** on the new broken fixture, never on a valid example.
- Atomic per-phase commits with the phase's `Commit.` line verbatim. Never `--no-verify`, never force-push.
- Don't widen scope: a phase may not reach into a later phase's files.
- Honor `type_inference.rs` purity: analysis stays pure functions; diagnostics are returned, not emitted as side effects from inside a Salsa query body.

---

## Context

A recurring source of hard-to-diagnose bugs is failure that is *swallowed* rather than surfaced (roadmap [What's Next #1](../ROADMAP.md#1-silent-failures--code-health-hardening)). The March 28 hardening sweep was one-shot and has regressed. This plan makes "fail loud, or handle it" a **tracked, ratcheted discipline**: it freezes the current debt with CI gates so it cannot grow, then pays down the highest-value cases. The four fronts are silent `Unknown`/fallback-without-diagnostic, swallowed errors, panic/`unwrap`/`println!` debt, and divergent duplicate implementations (the `build_fn_body_map` straggler — the large CLI↔runtime case was already closed by the runtime migration).

**Counting rule (applies to every number in this plan).** Debt is counted in *production code only*: test code (`#[cfg(test)]` modules, `tests.rs` files, `tests/` directories) is excluded — `unwrap`/`panic!`/`println!` are idiomatic in tests. Match `Result::expect` as `.expect("` (string-literal argument), **not** bare `.expect(` — the parser's own `self.expect(TOKEN)` token-expectation method otherwise dominates the count with false positives. Under this rule the production debt is modest and spread out: the worst single-file `unwrap`/`expect` sites are `crates/smelt-planner/src/plan_printer.rs` (15), `crates/smelt-backend-duckdb/src/lib.rs` (15), and `crates/smelt-db/src/lib.rs` (10); production `panic!`s are confined to `crates/smelt-datagen/src/generic.rs` (3, input-driven) and `crates/smelt-db/src/diagnostics_types.rs` (7, internal invariant guards); production library-crate `println!` is near zero (the raw 206 figure is dominated by `smelt-cli` user-facing output and test code). The raw all-code greps (~2,000 `unwrap`/`expect`) are not the worklist — the ratchet freezes production counts, and the paydown phases target the production sites above.

## Scope

### In scope
- **Ratchets first.** CI gates that freeze `unwrap`/`expect`, `println!`, and silent-`Unknown` counts at a committed baseline so new code cannot add to the debt.
- **Front 1 (highest value).** Split *legitimate* `Unknown` (meta-language `Any`, deliberate conservatism) from *error* `Unknown` (parse failure, missing annotation, unresolved ref) on the inference + resolution paths, and emit a diagnostic for the latter. New diagnostic codes documented in a new `docs/specs/diagnostics.md`.
- **Front 2.** Swallowed errors (`let _ =`, `.ok()`, `Err(_) =>`) in the type-inference + LSP layers that hide a reportable failure.
- **Front 3.** Convert the input-driven `panic!`s in datagen to typed errors (and annotate the internal-invariant panics in `smelt-db`); migrate the residual production library-crate `println!` to `tracing` and gate it at zero; a concrete `unwrap` down-payment in the worst production hotspots (`plan_printer.rs`, `smelt-backend-duckdb`, `smelt-db/src/lib.rs`).
- **Front 4.** Single-source `build_fn_body_map` vs `build_fn_body_map_from_model_files`.
- Land the new ratchet invariants in `CLAUDE.md` + `architecture.md`.

### Explicitly deferred
- **Mass `unwrap` conversion.** Converting every production `unwrap`/`expect` is multi-plan work; this plan installs the ratchet (no growth) and pays down only the worst production hotspots (Phase 9). The rest is tracked by the ratchet's baseline. (Note: `smelt-cli/src/python.rs`, previously named here as the worst hotspot, has only **1** production unwrap — its ~145 others are in its `#[cfg(test)]` module. The raw all-code count misidentified it.)
- **Full diagnostic catalogue.** Documenting all ~70 existing codes (BUG-052) is its own docs task; this plan creates `diagnostics.md` and documents the codes it adds, leaving the back-catalogue as a tracked Deferred-Work item.
- The other sweep `needs-review` leftovers (BUG-067/068/070/071) — unrelated fronts, tracked in the Deferred-Work Backlog.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1 — `unwrap`/`println` baseline ratchet (CI gate)        | done     | (below) | 2026-06-10 |
| 2 — `Unknown`-emission census + guard                    | done     | (below) | 2026-06-10 |
| 3 — Front 1: struct-field parse failure diagnostic       | done     | (below) | 2026-06-10 |
| 4 — Front 1: db-query inference paths                    | done     | (below) | 2026-06-10 |
| 5 — Front 1: types / dialect / runtime / state paths     | done     | (below) | 2026-06-10 |
| 6 — Front 2: swallowed errors (type-inference + LSP)     | done     | (below) | 2026-06-10 |
| 7 — Front 3: datagen panics → typed errors; annotate invariant panics | done     | (below) | 2026-06-10 |
| 8 — Front 3: zero-`println!` gate for library crates     | done     | (below) | 2026-06-10 |
| 9 — Front 3: production `unwrap` down-payment            | done     | (below) | 2026-06-10 |
| 10 — Front 4: single-source `build_fn_body_map`          | done     | (below) | 2026-06-10 |
| 11 — Tighten ratchets + land invariants + verify         | done     | (below) | 2026-06-10 |

---

### Phase 1: `unwrap`/`println` baseline ratchet (CI gate)

**Goal.** Freeze the current `unwrap`/`expect` and `println!` debt at a committed baseline so new code cannot grow it. This is the durable mechanism the rest of the plan reduces against.

**Pre-conditions.** None.

**TDD tests to write first.**
- `.claude/scripts/hardening-budget.sh` is exercised by a self-test fixture: `crates/smelt-core/tests/hardening_budget.rs::gate_detects_regression` — runs the script against a temp tree with one injected `.unwrap()` over baseline and asserts non-zero exit; runs it against the committed tree and asserts zero exit.
- The script writes per-crate counts to `.claude/hardening-baseline.txt`; the test asserts the committed baseline matches the current tree (so a drop without updating the baseline also fails — ratchet is two-sided).

**Implementation shape.** A `rg`-based counter per `crates/*/src`, compared to `.claude/hardening-baseline.txt`. New `Run hardening budget` step in `.github/workflows/test.yml` after the clippy step (install `rg` on the runner or fall back to plain `grep` — ubuntu runners don't ship ripgrep). The counter **must** apply the plan's counting rule: match `.unwrap()` and `.expect("` (string-literal arg — bare `.expect(` false-positives on the parser's `self.expect(TOKEN)` method), and exclude test code (everything after a file's first `#[cfg(test)]`, plus `tests.rs` files and `tests/` dirs). The exclusion is line-boundary-based and approximate for files with interleaved test/prod code — document that caveat in the script header, but production-only counting is mandatory, not best-effort: the raw all-code counts misidentified the paydown targets this plan originally named.

**Critical files.**
- `.claude/scripts/hardening-budget.sh` — the gate.
- `.claude/hardening-baseline.txt` — committed baseline.
- `.github/workflows/test.yml` — wire the step in.
- `crates/smelt-core/tests/hardening_budget.rs` — self-test.

**Docs touched.** Internal hardening — no user-visible surface. (Invariant text lands in Phase 11.)

**Review checklist:**
- [ ] Self-test asserts both directions (over-baseline fails; under-baseline-without-update fails)
- [ ] CI step runs on the same job as clippy, non-flaky; `rg` (or fallback) available on the runner
- [ ] Counter matches `.expect("` not bare `.expect(` (no parser `self.expect(TOKEN)` false positives), and excludes test code; script header documents the line-boundary approximation
- [ ] No scope creep into actual `unwrap` reduction (Phase 9)

**Commit.** `ci(hardening): freeze unwrap/expect + println debt at a baseline gate`

### Phase 2: `Unknown`-emission census + guard

**Goal.** Make silent `Unknown` *measurable*: enumerate every `DataType::Unknown` **construction** site in production code, classify each as `legitimate` (meta-language `Any`, deliberate conservatism) or `error` (must eventually emit a diagnostic), and gate new unclassified sites.

**What counts as a construction site.** A line that *produces* the value — `DataType::Unknown` as an expression result, function return, struct/enum field value, `unwrap_or`/`map_or` fallback, etc. **Excluded:** pattern-match arms (`DataType::Unknown =>`), comparisons (`== DataType::Unknown`, `matches!(…)`), and test code per the plan's counting rule. The raw grep shows 166 lines mentioning `DataType::Unknown`, but ~29 are match/compare uses and ~28 are test code — expect roughly **110** production construction sites in the census. The census script must apply these exclusions; the allowlist covers exactly what the script finds.

**Pre-conditions.** Phase 1 (gate harness pattern established).

**TDD tests to write first.**
- `crates/smelt-types/tests/unknown_census.rs::every_unknown_site_is_classified` — parses an allowlist `.claude/unknown-census.toml` (site → `legitimate`/`error` + one-line reason) and asserts it covers exactly the `DataType::Unknown` sites the census script finds; an unclassified new site fails the test.

**Implementation shape.** A census script (`grep`-driven, like Phase 1) lists production `DataType::Unknown` construction sites with `file:line`, applying the construction-vs-match and test-code exclusions above. The allowlist records every found site (~110 expected) with a classification + reason. The test is the guard. This produces the worklist Phases 3–5 burn down (the `error`-classified entries). Note the test lives in `crates/smelt-types/tests/` but scans the whole workspace — resolve the repo root via `CARGO_MANIFEST_DIR/../..`.

**Critical files.**
- `.claude/scripts/unknown-census.sh`
- `.claude/unknown-census.toml` — classified site list.
- `crates/smelt-types/tests/unknown_census.rs`

**Docs touched.** Internal hardening — no user-visible surface.

**Review checklist:**
- [ ] Every census site classified with a real reason, not a rubber-stamp
- [ ] Census excludes match/compare uses and test code (spot-check a handful of entries are genuine constructions)
- [ ] Worst case `crates/smelt-types/src/signatures.rs` struct-field site classified `error` (fixed in Phase 3)
- [ ] Guard fails on a synthetic new unclassified site

**Commit.** `chore(hardening): census + guard every DataType::Unknown construction site`

### Phase 3: Front 1 — struct-field parse failure diagnostic

**Goal.** The worst confirmed silent `Unknown`: `crates/smelt-types/src/signatures.rs` turns an unparseable struct-field type into `Unknown` with no diagnostic (`parse_type(inner_text.trim()).unwrap_or(DataType::Unknown)`). Emit a real diagnostic instead.

**Pre-conditions.** Phase 2 (census marks this site `error`).

**TDD tests to write first.**
- `crates/smelt-db/tests/struct_field_type.rs::unparseable_struct_field_emits_diagnostic` — a model with a struct field of an unrecognised type name; assert `file_diagnostics` returns the new `DiagnosticCode::UnknownStructFieldType` at the field's range.
- `examples/broken/` fixture exercising it; assert `example_diagnostics` flags it.
- Regression: a valid struct-field model in `examples/` stays diagnostic-free.

**Implementation shape.** Add `UnknownStructFieldType` to `DiagnosticCode` (`crates/smelt-db/src/diagnostics_types.rs`). Per Salsa purity, the pure parse helper returns `Result<DataType, FieldTypeError>` carrying the field range; the Salsa diagnostics query maps it to a diagnostic. Remove the `unwrap_or(DataType::Unknown)` swallow. Update the census entry → `legitimate`/fixed.

**Critical files.**
- `crates/smelt-types/src/signatures.rs` — return the error instead of swallowing.
- `crates/smelt-db/src/diagnostics_types.rs` — new code.
- `crates/smelt-db/src/queries/*` — surface it in `file_diagnostics`.
- `examples/broken/...` — fixture.

**Docs touched.**
- `docs/specs/diagnostics.md` — **new spec**; introduce the catalogue format and document `UnknownStructFieldType` (other codes filled in over time; back-catalogue tracked as a Deferred-Work item, BUG-052).
- `docs-site/docs/` — diagnostics reference page stub linking the new code.

**Review checklist:**
- [ ] New diagnostic fires only on the broken fixture; valid examples stay clean
- [ ] `signatures.rs` stays a pure function (no Salsa side effects)
- [ ] `diagnostics.md` is timeless (no phase vocabulary)
- [ ] Census entry updated

**Commit.** `feat(diag): emit UnknownStructFieldType instead of silently inferring Unknown`

### Phase 4: Front 1 — db-query inference paths

**Goal.** Burn down the `error`-classified `Unknown` sites in the db query layer (`crates/smelt-db/src/queries/schema.rs`, `check_types.rs`, `function_diagnostics.rs`, `function_body_check.rs`): each error-Unknown either emits an existing/new diagnostic or is re-classified `legitimate` with justification.

**Pre-conditions.** Phases 2–3.

**TDD tests to write first.**
- One red-green test per *distinct* error-Unknown cause found in these files (e.g. unresolved column ref → diagnostic; missing function-return annotation → diagnostic), in `crates/smelt-db/tests/inference_diagnostics.rs`, each with a real `examples/broken/` fixture.
- Regression: `example_diagnostics` + `example_workspaces` stay green on valid examples.

**Implementation shape.** For each site, decide emit-diagnostic vs justified-legitimate. Reuse existing `DiagnosticCode` variants where they fit; add new ones only when no existing code expresses the cause. Update `.claude/unknown-census.toml` as sites are resolved.

**Critical files.**
- `crates/smelt-db/src/queries/schema.rs`, `check_types.rs`, `function_diagnostics.rs`
- `crates/smelt-db/src/function_body_check.rs`
- `examples/broken/...`

**Docs touched.**
- `docs/specs/diagnostics.md` — document any new codes added.

**Review checklist:**
- [ ] Every error-Unknown in these files is resolved (diagnostic) or re-justified (census reason)
- [ ] No new false positive on valid examples
- [ ] Salsa purity preserved
- [ ] New codes documented

**Commit.** `feat(diag): surface error-Unknown on db inference paths`

### Phase 5: Front 1 — types / dialect / runtime / state paths

**Goal.** Same burn-down for the remaining error-Unknown clusters: `crates/smelt-types/src/{signatures,lib}.rs`, `crates/smelt-dialect/src/type_conformance.rs`, `crates/smelt-runtime/src/compile.rs`, `crates/smelt-state/src/schema_tracking.rs`.

**Pre-conditions.** Phase 4 (pattern + any shared new codes established).

**TDD tests to write first.**
- One red-green test per distinct cause across these crates (e.g. unrecognised dialect type → diagnostic rather than silent `Unknown`), with real fixtures.
- Regression suite stays green.

**Implementation shape.** As Phase 4, across the type/dialect/runtime/state layer. After this phase the census should contain **zero** unresolved `error` entries — the guard from Phase 2 enforces it.

**Critical files.**
- `crates/smelt-types/src/signatures.rs`, `lib.rs`
- `crates/smelt-dialect/src/type_conformance.rs`
- `crates/smelt-runtime/src/compile.rs`
- `crates/smelt-state/src/schema_tracking.rs`

**Docs touched.**
- `docs/specs/diagnostics.md` — document any new codes.

**Review checklist:**
- [ ] Census has zero unresolved `error` entries after this phase
- [ ] No false positives; Salsa purity preserved
- [ ] New codes documented

**Commit.** `feat(diag): surface error-Unknown on types/dialect/runtime/state paths`

### Phase 6: Front 2 — swallowed errors (type-inference + LSP)

**Goal.** Find `let _ =`, `.ok()`, `.ok()?`, and no-op `Err(_) =>` arms in the type-inference and LSP layers that drop a real, reportable failure reason, and surface it (diagnostic or `tracing::warn`).

**Pre-conditions.** Phases 3–5 (diagnostic plumbing in place).

**TDD tests to write first.**
- A red-green test per fixed site where a previously-lost reason is now observable (diagnostic emitted, or a `tracing` capture asserts the warning), in `crates/smelt-db/tests/swallowed_errors.rs` / `crates/smelt-lsp/tests/diagnostics.rs`.

**Implementation shape.** Triage only the cases that hide a reportable failure — leave legitimately-ignored results alone (annotate with a brief `// intentionally ignored: …`). Concentrate on the type-inference and LSP layers per the roadmap.

**Critical files.**
- `crates/smelt-db/src/**` (inference + resolution)
- `crates/smelt-lsp/src/**`

**Docs touched.** Internal hardening — no user-visible surface (any new user-visible diagnostic documented in `diagnostics.md`).

**Review checklist:**
- [ ] Each change exposes a genuinely-reportable failure; no noise on valid inputs
- [ ] Intentionally-ignored results carry a one-line justification
- [ ] No scope creep into panic/println work

**Commit.** `fix(hardening): surface swallowed errors on inference + LSP paths`

### Phase 7: Front 3 — datagen panics → typed errors; annotate invariant panics

**Goal.** Convert the input-driven `panic!`s to recoverable errors. The workspace's only production panics are: the three config-driven `linked_choice` cases in `crates/smelt-datagen/src/generic.rs` (lines ~401–414: pool not sampled / pool not built / field missing in pool) — convert these; and the seven message-formatter guard panics in `crates/smelt-db/src/diagnostics_types.rs` (`*_diagnostic_message called with non-… code`) — these guard an internal invariant reachable only by programmer error, so annotate with a justifying comment, don't convert. (An earlier draft targeted `ddl_spark` validation panics in `crates/smelt-state/src`; those all turned out to be `#[cfg(test)]` assertion arms — there is no production panic in `smelt-state`.)

**Pre-conditions.** Phase 1 (so the count drop is reflected in the baseline at Phase 11).

**TDD tests to write first.**
- `crates/smelt-datagen/tests/invalid_config.rs::missing_linked_choice_field_returns_err` — malformed datagen config returns `Err`, not a panic (one test per converted panic cause).

**Implementation shape.** Thread `Result` through the affected datagen entry points. The `diagnostics_types.rs` guards stay panics with a `// invariant:` comment explaining why they're unreachable from user input.

**Critical files.**
- `crates/smelt-datagen/src/generic.rs`
- `crates/smelt-db/src/diagnostics_types.rs` (annotations only)

**Docs touched.** Internal hardening — no user-visible surface.

**Review checklist:**
- [ ] The three datagen panics now return typed errors naming the offending config input
- [ ] The seven `diagnostics_types.rs` invariant panics carry justifying comments, unconverted
- [ ] No behavior change on valid input

**Commit.** `fix(hardening): convert input-driven datagen panics to typed errors`

### Phase 8: Front 3 — zero-`println!` gate for library crates

**Goal.** Gate library-crate production `println!` at **zero** and migrate the residual handful to `tracing`. The raw 206 workspace count is dominated by `smelt-cli` user-facing stdout (~106) and test code (e.g. all of `smelt-parser`'s and `smelt-db`'s are in test modules) — the actual production library debt is a small residual (survey `smelt-datagen`, `smelt-core`, `smelt-state`, `smelt-runtime`, `smelt-backend-*`, `smelt-logical`, `smelt-parser-compat` at implementation time). The durable value of this phase is the zero-gate, not the migration volume. Legitimate CLI user-facing stdout (`smelt-cli` command output, `smelt-ui`) stays — that's intentional output, not logging.

**Pre-conditions.** Phase 1 baseline.

**TDD tests to write first.**
- `crates/smelt-core/tests/hardening_budget.rs::no_println_in_libraries` — asserts zero production `println!` (per the plan's counting rule) in the designated library crates (`smelt-db`, `smelt-types`, `smelt-parser`, `smelt-planner`, `smelt-logical`, `smelt-runtime`, `smelt-dialect`, `smelt-state`, `smelt-datagen`, `smelt-core`, `smelt-backend-duckdb`, `smelt-backend-spark`, `smelt-backend`, `smelt-parser-compat`).

**Implementation shape.** Replace the residual production sites with `tracing::{debug,info,warn}` at the appropriate level. Lower the `println!` library budget in `.claude/hardening-baseline.txt` to 0 for those crates (CLI/UI budgets unchanged).

**Critical files.**
- The library crates above.
- `.claude/hardening-baseline.txt`

**Docs touched.** Internal hardening — no user-visible surface.

**Review checklist:**
- [ ] Zero production `println!` in library crates; CLI/UI output and test code untouched
- [ ] Levels are sensible (no `info!` spam in hot paths)
- [ ] Baseline lowered so it cannot regress

**Commit.** `refactor(hardening): gate library-crate println! at zero, migrate residual to tracing`

### Phase 9: Front 3 — production `unwrap` down-payment

**Goal.** A concrete reduction in the worst *production* hotspots: `crates/smelt-planner/src/plan_printer.rs` (15), `crates/smelt-backend-duckdb/src/lib.rs` (15), `crates/smelt-db/src/lib.rs` (10). For each site, classify: **input-driven** (reachable from user data / DB results — convert to `?`/typed error, with a red-green test) vs **infallible-by-construction** (e.g. `write!` into a `String` — annotate or switch to a non-panicking form, no test needed). Then ratchet the baseline down so they cannot return. (An earlier draft targeted `smelt-cli/src/python.rs` "~125 unwraps" — all but one are in its `#[cfg(test)]` module; the raw count misidentified it.)

**Pre-conditions.** Phase 1 baseline.

**TDD tests to write first.**
- One red-green test per *input-driven* conversion, e.g. `crates/smelt-backend-duckdb/tests/unwrap_errors.rs::malformed_db_result_returns_err` — a failure on the DB boundary surfaces a typed error instead of panicking. (Exact tests depend on the per-site classification; infallible sites get annotations, not tests.)

**Implementation shape.** Survey the three files first; convert the input-driven `unwrap`s to `Result`, annotate the rest. Update `.claude/hardening-baseline.txt` to the new lower per-crate counts.

**Critical files.**
- `crates/smelt-planner/src/plan_printer.rs`
- `crates/smelt-backend-duckdb/src/lib.rs`
- `crates/smelt-db/src/lib.rs`
- `.claude/hardening-baseline.txt`

**Docs touched.** Internal hardening — no user-visible surface.

**Review checklist:**
- [ ] Every site in the three files classified input-driven vs infallible; conversions tested, the rest annotated
- [ ] Baseline lowered by the count removed
- [ ] No behavior change on valid input

**Commit.** `fix(hardening): convert recoverable unwraps in planner/duckdb/db production hotspots`

### Phase 10: Front 4 — single-source `build_fn_body_map`

**Goal.** Collapse the duplicated default-extraction logic: `build_fn_body_map` (Salsa path) and `build_fn_body_map_from_model_files` (non-Salsa path), both in `crates/smelt-runtime/src/fn_bodies.rs`, must share one implementation so they cannot drift.

**Pre-conditions.** None (independent of the diagnostic work).

**TDD tests to write first.**
- `crates/smelt-runtime/tests/fn_body_parity.rs::both_paths_agree` — for a shared workspace fixture, assert `build_fn_body_map(db, ws)` and `build_fn_body_map_from_model_files(files)` produce an identical `FnBodyMap` (the property that protects against future drift).

**Implementation shape.** Extract the shared default-extraction core into one private function over a common input (`&[ModelFile]`); have the Salsa wrapper collect `ModelFile`s and delegate. No behavior change — pure de-duplication.

**Critical files.**
- `crates/smelt-runtime/src/fn_bodies.rs`

**Docs touched.** Internal refactor — no user-visible surface.

**Review checklist:**
- [ ] One implementation; both entry points delegate to it
- [ ] Parity test asserts identical output
- [ ] Removed from the Deferred-Work Backlog list in `ROADMAP.md`

**Commit.** `refactor(runtime): single-source build_fn_body_map across Salsa + file paths`

### Phase 11: Tighten ratchets + land invariants + verify

**Goal.** Make the discipline permanent: record the new ratchet invariants where future changes will see them, confirm the final baselines, and verify the whole suite.

**Pre-conditions.** All prior phases.

**TDD tests to write first.** None new — this phase asserts the gates from Phases 1, 2, and 8 all pass at the tightened baselines.

**Implementation shape.** Add the ratchet invariants to `CLAUDE.md` (§ Silent failures discipline) and `docs/specs/architecture.md` (a "Fail-loud invariants" subsection): (1) no new silent `error`-Unknown (Phase 2 guard), (2) no growth in `unwrap`/`expect` (Phase 1 gate), (3) zero `println!` in library crates (Phase 8 gate). Per the "architectural-decisions-into-specs" rule, these invariants must live in the spec/CLAUDE.md, not only in the CI scripts.

**Critical files.**
- `CLAUDE.md`
- `docs/specs/architecture.md`
- `.claude/hardening-baseline.txt` (final values)

**Docs touched.**
- `docs/specs/architecture.md` — fail-loud invariants subsection (timeless).
- `CLAUDE.md` — discipline + how the gates work.

**Review checklist:**
- [ ] Invariants recorded in both `architecture.md` and `CLAUDE.md`
- [ ] All three gates green at the tightened baselines
- [ ] `diagnostics.md` cross-linked from `architecture.md`

**Commit.** `docs(hardening): land fail-loud invariants in architecture.md + CLAUDE.md`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle here.)

- Mass `unwrap`/`expect` conversion beyond the Phase 9 hotspots (`plan_printer.rs`, `smelt-backend-duckdb/src/lib.rs`, `smelt-db/src/lib.rs`) — the ratchet prevents growth; bulk paydown is future per-crate work.
- Full diagnostic catalogue in `diagnostics.md` (all ~70 existing codes — BUG-052).

## Verification

How to confirm the discipline holds at the end:
- `cargo test -p smelt-cli --test example_diagnostics` — valid examples stay diagnostic-free.
- `cargo test -p smelt-lsp --test example_workspaces` — real-LSP-backend parity, no new false positives.
- `cargo test -p smelt-types --test unknown_census` — zero unresolved `error`-Unknown sites.
- `cargo test -p smelt-core --test hardening_budget` — `unwrap`/`println` baselines hold.
- `bash .claude/scripts/hardening-budget.sh` and `bash .claude/scripts/unknown-census.sh` exit 0.
- `cargo clippy --all-targets` clean; `cargo fmt --all -- --check` clean.
