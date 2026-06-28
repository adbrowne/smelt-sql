# Plan: Data-quality checks (`smelt.check`) + test-inlining diagnostics

**Date**: 2026-06-28
**Spec**: [`docs/specs/testing.md`](../specs/testing.md) (anchor) + [`cli.md`](../specs/cli.md), [`architecture.md`](../specs/architecture.md), [`diagnostics.md`](../specs/diagnostics.md)
**Spec diff**: commit `f95ac786` (branch `spec/data-checks`) — "spec: add smelt.check data-quality checks + test-inlining diagnostics"
**Tracking PR / branch**: `spec/data-checks` (create the PR off `main` before Phase 1)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/testing.md` §"Data checks — the `smelt.check` declaration", §"Check execution model", §Design, and the §"Diagnostic codes" table — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `spec/data-checks`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`: **Run pipeline parity** (the compile+execute pipeline lives only in `smelt-runtime`; the standing gate `cargo test -p smelt-runtime --test execute_parity` must stay green after Phase 4), **Salsa purity**, **Fail-loud discipline** (new diagnostics are anchored, never silent), and the **`MetadataError` exhaustiveness gate** if a new `MetadataError` variant is added.
- Build/test with system DuckDB (`DUCKDB_LIB_DIR` set); use `cargo test --quiet 2>&1 | tail -40` per the token-efficiency rules in `CLAUDE.md`.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. The specs are already updated by `f95ac786`; further spec edits during implementation describe behaviour, not phases. `docs-site/docs/...` edits read as if the feature has always existed.

---

## Context

The materialization-three-axes work left two follow-ups in `testing.md`. First, the deleted "singular tests" had no home: a data-quality assertion that runs against *real built data* (zero failing rows = pass) is a different execution model from `smelt.test`'s in-memory mocks, so it becomes its own `smelt.check` kind (`testing.md` §"Data checks", §"Check execution model"). Second, two whole-query test-inlining edges return raw-string errors instead of anchored diagnostics (`testing.md` §"Diagnostic codes" — `AmbiguousTestModel`, `NonStandaloneTestModel`). This plan drives the code to the spec. `smelt.check` mirrors `smelt.test`'s existing wiring (a `SMELT_*` parser node, a string-contains `classify_sql` arm, a parser-based `is_*()`), so each phase has a concrete template to follow.

## Scope

### In scope (spec coverage)
- `testing.md` §"Data checks" Surface — `smelt.check <name> AS (<select>)` grammar; `severity` frontmatter; `CheckHasTestClause`; no `#` in check bodies.
- `testing.md` §"Check execution model" — failing-rows semantics, configured-target execution, `CheckTargetNotBuilt`, capped violation sample, `severity: error|warn`.
- `cli.md` — `smelt check` command, exit codes, `smelt build` step 7 (downstream-blocking).
- `architecture.md` §Kind — `smelt.check` keyword-signalled kind.
- `diagnostics.md` — `AmbiguousTestModel`, `NonStandaloneTestModel`, `CheckHasTestClause`, `CheckTargetNotBuilt`.
- Example fixtures: re-express the two deleted singular tests as `smelt.check`s.

### Explicitly deferred
- **Project-wide `#` CTE addressing** — out of scope by spec decision (`testing.md` Known Divergences); `#` stays test-local.
- **Check thresholds** (`error_if`/`warn_if`) and **stored failures** (warehouse audit table) — `testing.md` Known Divergences; not in the v1 surface.
- **Generic / reusable parameterized checks** — `testing.md` Known Divergences; a check is a one-off failing-rows query.
- **Per-environment severity override** (block in CI, warn in dev) — `testing.md` Known Divergences.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 204b0f31 | 2026-06-28 |
| 2     | done     | 46f6358d | 2026-06-28 |
| 3     | done     | df18c355 | 2026-06-28 |
| 4     | done     |        | 2026-06-28 |
| 5     | pending  |        |      |

---

### Phase 1: Parser — `smelt.check` declaration grammar

**Goal.** Parse `smelt.check <name> AS ( <select> )` into a first-class `SMELT_CHECK` node, mirroring `SMELT_TEST` but with no `PASSING`/`EXPECT` in the grammar proper. A trailing `PASSING`/`EXPECT` in check position is captured on the node (not dropped) so Phase 2 can diagnose it.

**Pre-conditions.** None — parser is independent.

**TDD tests to write first.**
- `crates/smelt-parser/tests/...::parse_smelt_check_basic` — `smelt.check <name> AS ( SELECT ... )` parses into a `SmeltCheck` node with name + body select; lossless round-trip.
- `crates/smelt-parser/tests/...::smelt_check_no_clauses` — a check body with no `PASSING`/`EXPECT` parses cleanly; `AstFile::checks()` yields exactly one `SmeltCheck`, `tests()` yields none.
- `crates/smelt-parser/tests/...::smelt_check_with_stray_test_clause_is_recoverable` — `smelt.check x AS (...) PASSING dep AS (...)` parses (does not panic) and the stray `PASSING` is recoverable on the node for later diagnosis (asserts the clause is reachable, not silently discarded).
- `crates/smelt-parser/tests/...::check_keyword_contextual` — `check` outside a `smelt.check` (column alias, CTE name) stays an ordinary identifier.

**Implementation shape.**
- `crates/smelt-parser/src/syntax_kind.rs` — add `SMELT_CHECK` node kind (mirror `SMELT_TEST` at ~line 342).
- `crates/smelt-parser/src/parser/smelt_ext.rs` — `at_smelt_check_trigger()` (copy `at_smelt_test_trigger`, match `"check"`); `parse_smelt_check()` parsing `<name>` then `AS ( <select> )`, then optionally consuming a trailing `PASSING`/`EXPECT` clause into the node for diagnosis. Register the trigger in `parse_file()`.
- `crates/smelt-parser/src/ast.rs` — `SmeltCheck` typed wrapper (name, body select, optional stray clause) following `SmeltTest` (~line 251); add `AstFile::checks()` iterator mirroring `tests()` (~line 38).

**Critical files.**
- `crates/smelt-parser/src/syntax_kind.rs`, `parser/smelt_ext.rs`, `ast.rs`.

**Docs touched.** Code-only at the parser layer (the user-facing grammar is in `testing.md`, already updated). No docs-site change this phase.

**Review checklist.**
- [ ] Parser TDD tests exist; lossless round-trip holds for a check declaration.
- [ ] A stray `PASSING`/`EXPECT` is recoverable on the node, not dropped (so Phase 2 can emit `CheckHasTestClause`).
- [ ] Error recovery: a malformed `smelt.check` produces a parse error, not a panic.
- [ ] No kind/runner changes leaked into this phase.

**Commit.** `feat(parser): parse smelt.check declarations (AS / failing-rows body)`

---

### Phase 2: Kind, severity frontmatter, discovery filtering, check diagnostics

**Goal.** Recognise `smelt.check` as `EntityKind::Check`; attach `severity: error|warn`; route every test-exclusion site so checks are likewise excluded from materialization/run/explain/docs/diff/select; emit `CheckHasTestClause` and keep `#` invalid in check bodies (`CteRefOutsideTest`).

**Pre-conditions.** Phase 1 (`SmeltCheck` node + `AstFile::checks()`).

**TDD tests to write first.**
- `crates/smelt-core/src/resolver.rs` (unit) `classify_sql_check_is_check` — `smelt.check x AS (SELECT ...)` classifies as `EntityKind::Check` (mirror `classify_sql_test_is_test`).
- `crates/smelt-core/tests/...::is_check_via_parser` — `is_check()` returns true for a check file, false for a model/test; `is_test()` stays false for a check.
- `crates/smelt-core/tests/...::check_severity_parses` — `severity: warn` frontmatter on a `smelt.check` deserializes; default is `error`; an unknown `severity` value rejects loudly (strict user-authored frontmatter).
- `crates/smelt-cli/tests/example_diagnostics.rs` (or a smelt-db test) `check_with_passing_diagnoses` — a `smelt.check` carrying `PASSING`/`EXPECT` yields `CheckHasTestClause`, anchored at the clause.
- `crates/smelt-db/tests/...::hash_cte_ref_in_check_is_outside_test` — a `smelt.<m>#<cte>` ref inside a `smelt.check` body yields `CteRefOutsideTest` (a check body is not a test body).
- `crates/smelt-cli/tests/...::check_excluded_from_run_and_explain` — a workspace containing a `smelt.check` builds/explains with the check excluded (not materialized, absent from `explain`/catalog), via a real `examples/` fixture.

**Implementation shape.**
- `crates/smelt-core/src/resolver.rs` — add `EntityKind::Check`; add a `content.contains("smelt.check")` arm to `classify_sql()` (before the `Model` default). Ensure the test/check arms don't collide (a check file contains `smelt.check`, not `smelt.test`).
- `crates/smelt-core/src/discovery.rs` — `is_check()` mirroring `is_test()` (`AstFile::checks().next().is_some()`).
- Severity — a `CheckSeverity { Error, Warn }` enum (`#[serde(rename_all = "lowercase")]`, default `Error`) attached to the check declaration's metadata, mirroring how `check_order`/`cases` attach to a test. Register the `severity` key for the check declaration kind in `frontmatter.rs`.
- Filtering sites (mirror `is_test()` handling): `smelt-cli` `run_setup.rs:45`, `explain.rs:55`, `diff.rs:48`, `docs.rs` (130/143); `smelt-runtime/src/select.rs:127`; `smelt-ui/src/build.rs` (node type); `smelt-core/src/resolver.rs:398` emitted-name collision pre-filter; `smelt-lsp/src/backend.rs` test-collection. Introduce a shared `is_assertion()` (`is_test() || is_check()`) where a site means "exclude non-materialised declarations," to avoid scattering `|| is_check()`.
- `CheckHasTestClause` — detected where declaration diagnostics are produced (`smelt-db/src/lib.rs` analysis path), reading the stray clause captured in Phase 1.

**Critical files.**
- `crates/smelt-core/src/resolver.rs`, `discovery.rs`, `metadata.rs`/`config.rs` (severity), `frontmatter.rs`.
- `smelt-db/src/lib.rs` (`CheckHasTestClause`; `map_metadata_error_to_diagnostic` exhaustiveness only if a new `MetadataError` variant is introduced).
- Filtering sites in `smelt-cli`, `smelt-runtime`, `smelt-ui`, `smelt-lsp`.

**Docs touched.**
- `docs-site/docs/guide/testing.md` — introduce the `smelt.check` kind and `severity` as existing surface (no command details yet).

**Review checklist.**
- [ ] `EntityKind::Check` + `classify_sql`/`is_check` mirror the test wiring; `is_test()`/`is_check()` are mutually exclusive on a given file.
- [ ] Every test-exclusion site also excludes checks (no check ever materializes or appears in `explain`/catalog).
- [ ] `severity` parses with `error` default; unknown value fails loud.
- [ ] `CheckHasTestClause` and `#`-in-check (`CteRefOutsideTest`) anchored diagnostics fire per `testing.md`.
- [ ] User docs introduce the kind; spec/docs-site edits are timeless.

**Commit.** `feat(core): smelt.check kind, severity frontmatter, discovery exclusion`

---

### Phase 3: `smelt check` command — run checks against the configured target

**Goal.** A standalone `smelt check [--select <expr>]` that compiles each `smelt.check`'s failing-rows query (resolving `smelt.<path>` → built relations), executes it against the configured target, and reports PASS/FAIL/WARN with a violation count and capped sample, exiting per `cli.md` §"Exit codes". A referenced unbuilt model yields `CheckTargetNotBuilt`.

**Pre-conditions.** Phase 2 (`EntityKind::Check`, severity).

**TDD tests to write first.**
- `crates/smelt-cli/tests/...::check_passes_on_clean_data` — build a real `examples/` workspace, then `smelt check` a check whose failing-rows query returns zero rows → PASS, exit 0.
- `crates/smelt-cli/tests/...::check_fails_on_violation` — a check that returns rows → FAIL, exit 1, report shows the violation count and a capped row sample.
- `crates/smelt-cli/tests/...::warn_severity_does_not_fail` — same violation under `severity: warn` → WARN, exit 0.
- `crates/smelt-cli/tests/...::check_on_unbuilt_model_is_loud` — `smelt check` before building the referenced model → `CheckTargetNotBuilt`, exit 1 (never a silent pass).
- `crates/smelt-cli/tests/...::check_select_substring` — `--select` narrows by check name consistent with `smelt test --select`.

**Implementation shape.**
- `crates/smelt-cli/src/main.rs` — add `Check(CheckArgs)` to the `Commands` enum (~line 53) and dispatch (`commands::check::run_checks`); `CheckArgs` mirrors `TestArgs` (project_dir, select, target, database, verbose, json).
- `crates/smelt-cli/src/commands/check.rs` (new) + `mod.rs` registration — discover checks, apply selection, and per check: compile the body via the sanctioned `CompilerRegistry::get(...).compile_with_sql_and_ephemerals(...)` path (the check body is a SELECT over `smelt.<path>` built relations — no mocking, no inlining), instantiate the target backend via `backend_factory::create_backend`, run `Backend::execute_sql`, count rows across the returned `RecordBatch`es, and sample the first N via the `batches_to_rows` pattern.
- `CheckTargetNotBuilt` — when the compiled query references a relation absent from the target (catch the backend "table not found" error and re-key it to the anchored diagnostic at the offending ref), or pre-check existence before executing.
- Reporting/exit — `error`-severity violation → nonzero; `warn` → zero; reuse the test command's result-printing helpers where they fit.

**Critical files.**
- `crates/smelt-cli/src/main.rs`, `commands/check.rs`, `commands/mod.rs`.
- `crates/smelt-cli/src/backend_factory.rs` (reuse), a check-compile helper (new, in `commands/check.rs` or a `check_compiler.rs`).

**Docs touched.**
- `docs-site/docs/guide/testing.md` — document `smelt check`, failing-rows semantics, and `severity` as the running surface.
- `docs-site/docs/reference/cli.md` — `smelt check` flags + exit codes.
- `examples/` — re-express the deleted singular tests as checks: `examples/ephemeral_demo/.../daily_revenue_non_negative.sql` (`SELECT ... WHERE total_revenue < 0`) and the `examples/per_cohort_union` count-invariant rewritten to return rows only on violation. These are the real fixtures the TDD tests above exercise.

**Review checklist.**
- [ ] All command TDD tests green against a real built `examples/` workspace.
- [ ] Check bodies compile through the sanctioned `CompilerRegistry` path (run-pipeline-parity respected — no private compiler constructor use).
- [ ] Zero rows = PASS; rows = violation; `warn` never sets a nonzero exit; `CheckTargetNotBuilt` is loud.
- [ ] Violation report shows count + capped sample; no warehouse persistence.
- [ ] User docs (testing guide + cli reference) match Surface; examples migrated; edits timeless.

**Commit.** `feat(cli): smelt check runs data-quality checks against the configured target`

---

### Phase 4: Build integration — run checks during `smelt build`, block downstream

**Goal.** During `smelt build`, after a model materializes, run the `smelt.check`s that reference it against the just-written data; an `error`-severity violation skips every model downstream of the checked model and fails the build, while `warn` reports and continues (`cli.md` §"`smelt build` lifecycle" step 7; `testing.md` §"Check execution model" — Build integration). Lives in `smelt-runtime` to honor run-pipeline parity.

**Pre-conditions.** Phase 3 (check compile + execute helper proven).

**TDD tests to write first.**
- `crates/smelt-cli/tests/...::build_error_check_skips_downstream` — a workspace where model A has an `error`-severity check that fails and model B depends on A: `smelt build` materializes A, the check fails, B is **skipped**, build exits nonzero. Assert B's relation is absent / unchanged.
- `crates/smelt-cli/tests/...::build_warn_check_does_not_skip` — same shape with `severity: warn`: B still builds, build exits 0, WARN reported.
- `crates/smelt-cli/tests/...::build_passing_check_is_transparent` — a passing check leaves the build identical to no check.
- `crates/smelt-runtime/tests/execute_parity` — stays green (the parity gate must not regress).

**Implementation shape.**
- `crates/smelt-runtime/src/execute.rs` — at the post-materialize / pre-manifest seam of each execution path (the cumulative, incremental, and full-refresh arms), invoke the checks attached to the just-built model. A check's guarded model set is derived from its `smelt.<path>` refs (a check guards every model it reads). On an `error`-severity violation, mark the checked model's downstream closure as skipped for the remainder of the run and record the failure via the reporter; `warn` only reports.
- Reuse the Phase 3 check-compile + execute-and-count helper; factor it so both `smelt check` (CLI) and the build seam (runtime) call one implementation (the helper belongs in `smelt-runtime` so the runtime can call it without depending on `smelt-cli`; the CLI command calls into it — same run-pipeline-parity reasoning as `execute_project`).
- Downstream skip — extend the execution loop's per-model state with a `Skipped(reason)` outcome and have topological iteration consult an accumulating skip set; surface skipped models in the run summary.

**Critical files.**
- `crates/smelt-runtime/src/execute.rs` (build seam + skip set), runtime check-runner helper.
- `crates/smelt-cli/src/commands/check.rs` — refactor to call the shared runtime helper.

**Docs touched.**
- `docs-site/docs/guide/testing.md` — document build-time check behavior and downstream blocking as existing surface.

**Review checklist.**
- [ ] `error`-severity violation skips the checked model's downstream closure and fails the build; `warn` does neither.
- [ ] Check execution + downstream-skip live in `smelt-runtime`; `cargo test -p smelt-runtime --test execute_parity` green (run-pipeline parity intact).
- [ ] The check-runner helper is shared between the CLI command and the build seam (no duplicated execute logic).
- [ ] Skipped models surfaced in the run summary; `smelt run` still does not run checks.
- [ ] User docs updated; timeless.

**Commit.** `feat(runtime): run smelt.check during build; error-severity blocks downstream`

---

### Phase 5: Harden test whole-query inlining diagnostics (`AmbiguousTestModel`, `NonStandaloneTestModel`)

**Goal.** Promote the two raw-string edges in the `smelt.test` whole-query inlining path to anchored, fail-loud diagnostics with codes (`diagnostics.md`, `testing.md` §"Diagnostic codes"). No auto-resolution. Independent of Phases 1–4.

**Pre-conditions.** None (touches the existing test path).

**TDD tests to write first.**
- `crates/smelt-cli/tests/...::ambiguous_single_segment_ref_diagnoses` — a `smelt.test` with a single-segment `smelt.<leaf>` ref where two models share that leaf → `AmbiguousTestModel`, listing both candidate addresses, advising the full dotted address; anchored at the ref.
- `crates/smelt-cli/tests/...::non_standalone_upstream_diagnoses` — a whole-query test inlining an upstream model that cannot compile standalone (relies on a per-model config var / incremental construct) and is not mocked via `PASSING` → `NonStandaloneTestModel` with the "mock this dependency via `PASSING`" hint; anchored at the offending ref.
- `crates/smelt-cli/tests/...::mocking_the_dep_resolves_non_standalone` — the same test with the offending dep mocked via `PASSING` compiles and runs (the hint is actionable).

**Implementation shape.**
- `crates/smelt-cli/src/test_compiler.rs` — change `resolve_model_body` / `inline_unmocked_model_refs` / `compile_whole_query_test` error returns from bare `String` to a typed error carrying a diagnostic **code** + the offending ref's byte range (the refs already expose ranges via `find_plain_model_refs_in_body`). The ambiguous arm (~line 543) becomes `AmbiguousTestModel`; detect the non-standalone-inline failure (an inlined upstream body that fails to compile and was not mocked) and emit `NonStandaloneTestModel` instead of letting a raw backend error surface.
- Thread the typed error up through `commands/test.rs` so it prints as an anchored diagnostic (reuse the test-error → diagnostic surface), not a `CompilationError(String)`.

**Critical files.**
- `crates/smelt-cli/src/test_compiler.rs`, `commands/test.rs`, `test_runner.rs` (`TestError` carrying a code + range).

**Docs touched.** Code-only at the diagnostics layer (`diagnostics.md` and `testing.md` already list both codes). No docs-site change this phase.

**Review checklist.**
- [ ] Both diagnostics are anchored at the offending ref and carry the spec'd codes/messages.
- [ ] Fail-loud: no auto-disambiguation, no auto-mocking — the user is told to use a full address / `PASSING`.
- [ ] The `NonStandaloneTestModel` hint is actionable (the mock-the-dep test passes).
- [ ] No regression to existing passing `smelt.test` fixtures.

**Commit.** `feat(cli): anchored AmbiguousTestModel + NonStandaloneTestModel test diagnostics`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 3 — `CheckTargetNotBuilt` skip-list is narrower than the graph dep-exclusion list.** The `smelt check` pre-check (`commands/check.rs`) skips only `sources`/`functions` refs before calling `backend.table_exists`, whereas `DependencyGraph::build` also excludes `seeds`, `config`, and the `models.with_tag`/`models.all` meta-accessors. A check body that directly references `smelt.seeds.*`, `smelt.config.var(...)`, or `smelt.models.*` could surface a spurious `CheckTargetNotBuilt`. Not triggered by any current fixture; align the skip-list with `graph.rs` in a follow-up.
- **Phase 3 — single `smelt.check` per file.** The `smelt check` loop processes only the first check declaration in a file (`AstFile::checks().next()`), unlike the `smelt.test` runner which loops over all. The spec does not address multi-check files; if supported later, loop over all `checks()`.
- **Phase 4 — a check guarding multiple models runs before all of them are built.** During `smelt build`, `checks_by_model` keys a check under every model it references, so a check reading `smelt.a` and `smelt.b` (build order A→B) runs after A materialises while B is still unbuilt → a spurious `CheckTargetNotBuilt`/skip. All current spec examples and fixtures use single-model checks. Fix later by running a check only once all its referenced models are built (e.g. register it under the last of its refs in topological order, or gate on all-refs-built).

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test --quiet 2>&1 | tail -40` — full workspace green.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` — examples clean via both the Salsa-direct and real-LSP paths (checks excluded from materialization).
- `cargo test -p smelt-runtime --test execute_parity` — run-pipeline parity intact after Phase 4.
- `cargo clippy --all-targets` — no warnings.
- A real `examples/` check: `smelt build` then `smelt check` — a clean check PASSes; a violating `error`-severity check FAILs (exit 1) and, during `build`, skips downstream; a `warn` check WARNs without blocking; a check on an unbuilt model is `CheckTargetNotBuilt`.
- A `smelt.test` with an ambiguous single-segment ref reports `AmbiguousTestModel`; a non-standalone inlined upstream reports `NonStandaloneTestModel`.
- `/smelt:validate testing`, `/smelt:validate cli` — zero drift.
