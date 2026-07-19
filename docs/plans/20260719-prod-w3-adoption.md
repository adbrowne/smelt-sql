# Plan: Production readiness W3 — adoption surface (init, declarative tests, list/clean, failure UX)

**Date**: 2026-07-19
**Spec**: [`docs/specs/data_tests.md`](../specs/data_tests.md) (new, Phase 1), [`docs/specs/cli.md`](../specs/cli.md), [`docs/specs/testing.md`](../specs/testing.md)
**Spec diff**: new spec + `cli.md` Surface additions (written in Phase 1 of this plan)
**Tracking PR / branch**: `worktree-production`
**Docs**: code+docs
**Master**: [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md) — W3. Research basis: [`docs/research/20260719-production-release-review.md`](../research/20260719-production-release-review.md) blockers #5, #6, #10 + secondary "no `smelt clean`/`list` commands".

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/data_tests.md` (exists after Phase 1), `docs/specs/cli.md`, and `docs/specs/testing.md` — they are the correctness oracle. Do not re-open settled spec decisions; the derived-property-aware test semantics is decision D3 in the master plan and is settled.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
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
- Verification gate is `bash .claude/scripts/verify-phase.sh`.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (fail-loud discipline; run-pipeline parity — new CLI commands consume `smelt-runtime`/`smelt-core` public surface only).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in this plan file only.

---

## Context

The production-release review found no onboarding path (`smelt init` is hinted at in `crates/smelt-cli/src/errors.rs:9` but does not exist), no declarative column tests (custom-SQL `smelt check` only), no `smelt list`/`clean`, and terse single-line failure output. This plan lands those four adoption surfaces. The declarative-test design is smelt-native: dbt-familiar vocabulary (`not_null`, `unique`, `accepted_values`, `relationships`) whose semantics consult the derived properties first — a test the type system or grain proofs already prove compiles to a compile-time verdict and emits no scan; only unproven tests lower to SQL through the existing `smelt check` machinery ("derive, don't declare").

## Scope

### In scope (spec coverage)
- `data_tests.md` (all of it — new spec): `columns.<c>.tests` surface, proven-verdict semantics, scan lowering, reporting.
- `cli.md` §Surface: `smelt init`, `smelt list`, `smelt clean` subcommands.
- `models.md` §"`columns:` — column metadata": the `tests` key joins the canonical `columns:` grammar (resolving the deferred grammar-ownership note in §Known Divergences).
- `testing.md`: cross-reference from checks to declarative tests (which lower into the same machinery).
- Failure-summary UX on `smelt run`/`build` (research blocker #10, CLI-side slice only).

### Explicitly deferred
- Run-report artifact / structured logs — W2 (operability) owns the reporter-side run report; Phase 6 here is CLI presentation only.
- Custom/user-defined test macros (dbt `generic tests`) — post-0.5; the four built-ins cover the adoption case.
- Severity levels (`warn`) on declarative tests — `smelt check` severity exists (`CheckSeverity`); declarative tests are error-severity in this plan; spec records the extension point.
- `smelt state` inspection command — W2 owns state-store changes; a read-only `state` command should follow the versioned store, not precede it.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | TBD    | 2026-07-20 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |

## Phase detail

### Phase 1: Spec — `data_tests.md` + CLI surface additions

**Goal.** Write the normative spec for declarative column tests and add `init`/`list`/`clean` to the CLI spec, so Phases 2–6 have an oracle. Docs-only phase (no code).

**Pre-conditions.** None.

**TDD tests to write first.** None (docs-only). The spec's §Surface examples must be real YAML that later phases turn into fixtures verbatim.

**Implementation shape.** New `docs/specs/data_tests.md` from `docs/specs/SPEC_TEMPLATE.md`:
- §Surface: `columns.<name>.tests` accepts a list; entries are `not_null`, `unique`, `{accepted_values: [..]}`, `{relationships: {to: <model>, field: <column>}}`. Unknown test kinds and tests on columns absent from the inferred schema are hard diagnostics (fail-loud; contrast the silent-drop rule for other `columns:` metadata in `models.md`).
- §Semantics: resolution order — (1) consult derived properties: `not_null` proven by nullability inference, `unique` proven when the column set is the declared grain key (`unique_key`) or a proven grain/FD key; proven ⇒ compile-time verdict `proven — no scan emitted`, surfaced in `smelt test`/`check` output and the data catalog; (2) unproven ⇒ lower to a failing-rows SELECT executed by the `smelt check` machinery (zero rows = PASS). A proof may only *remove* a scan, never suppress a failure: if the proof engine cannot decide, the scan runs.
- §Design: rationale — why derived-property-aware (declared tests drift, proven properties can't; scans cost warehouse time dbt spends re-proving what a type system knows); rejected alternative — pure dbt-clone runtime scans (recorded with reasons); rejected — a separate `tests:` top-level block (column tests belong to the column, per `models.md` canonical-home rule).
- §References: `models.md` §"`columns:` — column metadata", `testing.md`, `cli.md`.

Edits: `docs/specs/cli.md` §Surface gains `smelt init` (non-interactive scaffolder), `smelt list`, `smelt clean` (+ §Semantics entries incl. exit codes and the clean-never-touches-state rule); `docs/specs/models.md` §"`columns:` — column metadata" adds the `tests` key row pointing at `data_tests.md` as normative owner, and its §Known Divergences deferred-grammar note is resolved; `docs/specs/testing.md` §References links `data_tests.md`.

**Critical files (allowed to touch in this phase).**
- `docs/specs/data_tests.md` — new
- `docs/specs/cli.md`, `docs/specs/models.md`, `docs/specs/testing.md` — additions above

**Docs touched.** Spec files only this phase; docs-site pages ride with the implementing phases.

**Review checklist** (material findings only):
- [ ] Spec follows SPEC_TEMPLATE.md; Design section records rationale AND rejected alternatives
- [ ] Proven-verdict rule is stated fail-safe (undecidable ⇒ scan runs)
- [ ] `models.md` grammar-ownership note resolved, not duplicated
- [ ] Timeless — no phase vocabulary in any spec body

**Commit.** `spec: declarative column tests (derived-property-aware) + init/list/clean CLI surface`

### Phase 2: `smelt init`

**Goal.** Make the existing error hint true: `smelt init [DIR]` scaffolds a minimal working project that builds green against DuckDB.

**Pre-conditions.** Phase 1 (`cli.md` defines the scaffold contents).

**TDD tests to write first.**
- `crates/smelt-cli/tests/init_command.rs::init_scaffolds_project_that_builds` — run `smelt init` in a temp dir; assert `smelt.yml`, `models/`, one example model, one seed CSV, `.gitignore` (ignoring `.smelt/` and the database file) exist; then run `smelt build` in the scaffold and assert exit 0 (same harness style as `crates/smelt-cli/tests/check_command.rs`).
- `crates/smelt-cli/tests/init_command.rs::init_refuses_nonempty_dir_without_force` — `smelt init` in a dir already containing `smelt.yml` exits non-zero with a message naming the conflict; `--force` is deliberately absent (re-run guidance instead).
- `crates/smelt-cli/tests/init_command.rs::init_scaffold_has_no_diagnostics` — load the scaffolded workspace via `smelt_core::workspace::load_workspace` and assert zero diagnostics.

**Implementation shape.** New `Init(InitArgs)` variant in `enum Commands` (`crates/smelt-cli/src/main.rs`); `crates/smelt-cli/src/commands/init.rs` writes embedded template files (include_str! from a `templates/init/` dir in the crate). Template content mirrors the quickstart page so docs and scaffold cannot drift.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/main.rs` — subcommand
- `crates/smelt-cli/src/commands/init.rs`, `crates/smelt-cli/src/commands/mod.rs` — new command
- `crates/smelt-cli/templates/init/*` — scaffold templates
- `crates/smelt-cli/tests/init_command.rs` — new

**Docs touched.**
- `docs-site/docs/getting-started/*` — quickstart begins with `smelt init`
- `docs-site/docs/reference/cli.md` — `init` entry

**Review checklist** (material findings only):
- [ ] Scaffold builds green end-to-end in the test (not just files-exist)
- [ ] Non-empty-dir refusal is fail-loud per spec
- [ ] Quickstart and template content agree
- [ ] No scope creep into Phase 5's commands

**Commit.** `feat(cli): smelt init — scaffold a minimal working project`

### Phase 3: Declarative tests — parsing + proven-property short-circuit

**Goal.** Parse `columns.<c>.tests`, reject unknown kinds/columns loudly, and land the proven-verdict path for `not_null` and `unique`.

**Pre-conditions.** Phase 1.

**TDD tests to write first.**
- `crates/smelt-core/src/metadata.rs` unit tests: `parses_column_tests_list` (all four kinds, incl. parameterized forms), `unknown_test_kind_is_metadata_error` (fail-loud; new `MetadataError` variant — the exhaustiveness gate in `smelt-db/src/lib.rs` forces the diagnostic mapping arm).
- `crates/smelt-cli/tests/data_tests.rs::test_on_unknown_column_is_diagnostic` — fixture model declares `tests` on a column not in the inferred schema; `smelt run --dry-run` surfaces a diagnostic (contrast: other `columns:` metadata silently drops).
- `crates/smelt-cli/tests/data_tests.rs::proven_not_null_emits_no_scan` — fixture where the column is inferred NOT NULL; `smelt test` (or `check`, per Phase 1's spec decision on which command hosts them) reports `proven` for it and the executed-statement log contains no scan for that test.
- `crates/smelt-cli/tests/data_tests.rs::proven_unique_via_grain_key` — incremental model with `unique_key: [id]`; `unique` test on `id` reports `proven`.

**Implementation shape.** `ColumnTest` enum + `tests: Vec<ColumnTest>` on `ColumnMetadata` (`crates/smelt-core/src/metadata.rs:105`); validation alongside the existing frontmatter rules. Proof consultation is a pure function in `smelt-logical` (or `smelt-db` query calling a pure fn, per Salsa-purity rule) taking the inferred schema's nullability + the model's declared/proven key set → `TestVerdict::Proven | NeedsScan`. Reporting through the check-run output path.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/metadata.rs` — surface + validation
- `crates/smelt-db/src/lib.rs` — `MetadataError` mapping arm
- `crates/smelt-logical/src/...` (new small module) — verdict derivation
- `crates/smelt-cli/src/commands/check.rs` (or `test.rs` per spec) — proven reporting
- `crates/smelt-cli/tests/data_tests.rs`, `examples/` fixture additions

**Docs touched.**
- `docs-site/docs/guide/testing.md` — declarative tests section (proven vs scanned)
- `docs/specs/data_tests.md` — Known Divergences: `accepted_values`/`relationships` scans not yet lowered (behavioural terms)

**Review checklist** (material findings only):
- [ ] Unknown kind/column paths are diagnostics, not silent drops (fail-loud gates pass)
- [ ] Verdict derivation is a pure function; no Salsa-impurity
- [ ] Proven verdict is fail-safe: undecidable nullability/keys ⇒ `NeedsScan`
- [ ] Unknown-census / hardening ratchets untouched or justified

**Commit.** `feat(tests): columns.<c>.tests surface + proven-property verdicts for not_null/unique`

### Phase 4: Declarative tests — scan lowering via check machinery

**Goal.** Lower every unproven test to a failing-rows SELECT executed by the existing `smelt check` runner; failures name model/column/test and exit 1.

**Pre-conditions.** Phase 3.

**TDD tests to write first.**
- `crates/smelt-cli/tests/data_tests.rs::unproven_not_null_scan_fails_on_nulls` — fixture with a nullable column containing NULLs: exit 1, output names `<model>.<column> not_null` and the failing-row count.
- `crates/smelt-cli/tests/data_tests.rs::accepted_values_pass_and_fail` — both directions against a built example.
- `crates/smelt-cli/tests/data_tests.rs::relationships_orphan_fails` — child row referencing a missing parent key: exit 1; intact referential integrity: exit 0.
- `crates/smelt-cli/tests/data_tests.rs::generated_sql_is_emitter_authored` — the lowered SELECTs come from a pure emitter fn (unit-testable, per-backend quoting via the existing compile path), not string concat in the command layer.

**Implementation shape.** Pure lowering fn `ColumnTest -> CheckBody` (SELECT returning failing rows) in the same `smelt-logical`/core module as Phase 3's verdicts; the CLI enumerates model column tests, filters `NeedsScan`, and feeds them through `smelt_runtime::run_single_check` exactly like `smelt.check` models (`crates/smelt-cli/src/commands/check.rs`). `relationships` compiles a NOT-EXISTS anti-join against the referenced model's relation.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/...` — lowering
- `crates/smelt-cli/src/commands/check.rs` — enumeration + execution + reporting
- `crates/smelt-cli/tests/data_tests.rs`, `examples/` fixtures

**Docs touched.**
- `docs-site/docs/guide/testing.md` — full four-kind reference with output examples
- `docs/specs/data_tests.md` — drop the Phase-3 divergence entry

**Review checklist** (material findings only):
- [ ] Scans execute through `run_single_check` (run-pipeline parity; no second execution path)
- [ ] Exit codes match `cli.md` semantics
- [ ] `relationships` handles the referenced model not being built (same `CheckTargetNotBuilt` behaviour as checks)
- [ ] Lowered SQL asserted per-backend via the existing compile path, not raw strings

**Commit.** `feat(tests): lower unproven column tests to failing-rows scans via smelt check`

### Phase 5: `smelt list` + `smelt clean`

**Goal.** `smelt list` prints each model with materialization, refresh kind, and target; `smelt clean` removes derived artifacts and never touches state without `--state`.

**Pre-conditions.** Phase 1.

**TDD tests to write first.**
- `crates/smelt-cli/tests/list_clean.rs::list_shows_all_models_with_kinds` — against `examples/timeseries`: every model listed once with materialization + refresh kind; `--select` filters (reuse the model-selection surface from `model_selection.md`).
- `crates/smelt-cli/tests/list_clean.rs::list_json_output` — `--format json` parses and round-trips the same fields (orchestrator use).
- `crates/smelt-cli/tests/list_clean.rs::clean_removes_artifacts_preserves_state` — after a build, `smelt clean` removes generated artifacts, leaves `.smelt/` state files; `smelt clean --state` removes state too; both print what they deleted.

**Implementation shape.** `List(ListArgs)` + `Clean(CleanArgs)` in `enum Commands`; `commands/list.rs` walks `load_workspace` output (workspace-loading parity — no private re-discovery); `commands/clean.rs` deletes only paths smelt itself created (enumerated, printed, no globbing outside the project's derived dirs).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/main.rs`, `crates/smelt-cli/src/commands/{list,clean}.rs`, `commands/mod.rs`
- `crates/smelt-cli/tests/list_clean.rs`

**Docs touched.**
- `docs-site/docs/reference/cli.md` — `list`, `clean` entries

**Review checklist** (material findings only):
- [ ] `list` consumes `load_workspace` (parity rule), not its own discovery
- [ ] `clean` provably cannot delete user files — deletion set enumerated from known derived paths only
- [ ] `--state` gating matches spec exactly

**Commit.** `feat(cli): smelt list and smelt clean`

### Phase 6: Failure-summary UX

**Goal.** A failed `smelt run`/`build` ends with a grouped summary — per failed model: one-line cause and a hint — instead of only the first terse error.

**Pre-conditions.** None hard; if W2's run-report phases have landed, consume their structured result; otherwise build on `crates/smelt-runtime/src/reporter.rs` as-is.

**TDD tests to write first.**
- `crates/smelt-cli/tests/failure_summary.rs::multi_model_failure_grouped_summary` — fixture (extend `examples/broken/` or temp-copy variant) where two models fail for different reasons: output contains a summary block listing both models, each with a one-line cause; exit code unchanged.
- `crates/smelt-cli/tests/failure_summary.rs::success_run_prints_no_failure_block` — green run emits no summary block.
- `crates/smelt-runtime/src/reporter.rs` unit test: reporter accumulates per-model failure records with cause classification (compile vs execute vs check).

**Implementation shape.** Reporter accumulates `ModelFailure { model, stage, cause_line, hint }`; CLI prints the block after the run loop. Hints map the existing `CliError` variants (`crates/smelt-cli/src/errors.rs`) to next actions (e.g. `ParseError` → file:line, `CheckTargetNotBuilt` → `smelt build`).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/reporter.rs` — accumulation
- `crates/smelt-cli/src/commands/{run,build}.rs` — presentation
- `crates/smelt-cli/tests/failure_summary.rs`

**Docs touched.**
- `docs-site/docs/reference/cli.md` — failure output description
- `docs/specs/cli.md` §Semantics — summary contract (timeless)

**Review checklist** (material findings only):
- [ ] Reporter change is presentation-side; `execute_project` contract untouched (run-pipeline parity)
- [ ] Exit codes unchanged by summary printing
- [ ] No duplicate printing of the same error (summary replaces nothing, it aggregates)

**Commit.** `feat(cli): grouped failure summary at end of run/build`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `smelt init` in an empty temp dir, then `smelt build && smelt list && smelt check && smelt clean` all behave per `cli.md`.
- `examples/` fixture with all four test kinds: proven tests report `proven`, unproven scans pass/fail correctly with exit codes per spec.
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate data_tests` and `/smelt:validate cli` report zero drift.
