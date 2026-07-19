# Plan: Production operability — secrets, state hardening, parallel execution, retry/resume, run reports

**Date**: 2026-07-19
**Spec**: [`docs/specs/smelt_yml.md`](../specs/smelt_yml.md), [`docs/specs/run_state.md`](../specs/run_state.md)
**Spec diff**: written by Phase 1 of this plan (env interpolation; state versioning/locking/per-target layout; resume + run-report semantics)
**Tracking PR / branch**: worktree-production
**Docs**: code+docs
**Master**: [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md) (sub-plan W2)
**Research basis**: [`docs/research/20260719-production-release-review.md`](../research/20260719-production-release-review.md) blockers #1, #2, #3, #4, #8, #10

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/smelt_yml.md` and `docs/specs/run_state.md` — they are the correctness oracle. Phase 1 amends them; from Phase 2 on, do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. Export `DUCKDB_LIB_DIR` and `LD_LIBRARY_PATH` (see CLAUDE.md) — unset means every DuckDB-backed equivalence leg skips green, which is a silent hole for Phases 5–8 in particular.
4. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:** real-fixture tests in `examples/`; red-green TDD; verification gate is `bash .claude/scripts/verify-phase.sh`; atomic per-phase commits with the phase's `Commit.` line verbatim; never `--no-verify`; don't widen scope; honor CLAUDE.md invariants — for this plan especially **run-pipeline parity** (`cargo test -p smelt-runtime --test execute_parity` must stay green in every phase that touches `execute.rs`) and **fail-loud discipline** (no silent fallback on missing env vars, locked state, or unknown state versions). **Timeless-oracle rule**: phase vocabulary stays in this file; spec and docs-site edits read as if the feature always existed.

---

## Context

The production-release review found that smelt's correctness core is release-grade but its operability is not: `smelt.yml` cannot reference secrets (`connect_url` is plaintext), models execute in a sequential `for` loop (`crates/smelt-runtime/src/execute.rs:691`), a failed run aborts at the first error with no retry and no way to resume, and `.smelt/` state is unversioned, unlocked, non-atomic plain JSON (`crates/smelt-state/src/file_store.rs` uses bare `std::fs::write`) shared across targets. This plan closes those gaps behind the two owning specs.

## Scope

### In scope (spec coverage)
- `smelt_yml.md` §Surface: `${VAR}` environment interpolation in string values, fail-loud on missing vars.
- `run_state.md` §Surface/§Semantics: state-schema versioning (`meta.json`), advisory locking, atomic writes, per-target state layout, per-model outcome records, run-report artifact, `--resume` semantics.
- `execute_project` scheduling: DAG-parallel wavefront + `--jobs`, bounded retry for transient backend errors.

### Explicitly deferred
- Full virtual environments / promotion / diffing (`virtual_environments.md`; the existing `state.mode` lattice in `crates/smelt-core/src/config.rs` is untouched) — per master decision D4, v0.5 ships per-target isolation only.
- Secret *providers* (vault, keychain): env interpolation only; providers can compose later.
- Cross-process distributed locking (advisory single-host file lock only).
- Spark-side auth/TLS surface — owned by sub-plan W4, which depends on Phase 2 landing here.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | (this commit) | 2026-07-19 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |
| 8     | pending  |        |      |

## Phase detail

### Phase 1: Spec diff — interpolation, state versioning/locking/layout, resume, run report

**Goal.** Land the normative surface this plan implements, so Phases 2–8 have an oracle.

**Pre-conditions.** None (docs-only).

**TDD tests to write first.** None (spec-only phase); `/smelt:validate smelt_yml` and `/smelt:validate run_state` drift reports are the check.

**Implementation shape.** Edit `docs/specs/smelt_yml.md`: §Surface gains "Environment interpolation" — `${VAR}` in any string value, resolved at config load; missing variable is a hard configuration error naming the variable and the YAML key path (never empty-string); `$$` escapes a literal `$`. Edit `docs/specs/run_state.md`: §"`.smelt/` directory layout" gains `meta.json` (`state_version`, current version 1, unknown-future-version = hard error, missing = legacy layout to migrate) and the per-target layout `.smelt/targets/<target>/{runs/,intervals.json,reconciliation.json,landed_deltas.json,snapshots.json,schemas/}`; §Semantics gains single-writer advisory locking (`.smelt/lock`, fail-loud "state locked by PID" error), atomic temp+rename writes, per-model outcome (`success`/`failed`/`skipped`) in the run manifest, the run-report artifact (`.smelt/targets/<target>/reports/<run_id>.json`), and `--resume` semantics (defined in Phase 7's terms: skip models whose latest outcome in the most recent incomplete run is `success` and whose definition hash is unchanged). Record open items under Known Divergences.

**Critical files (allowed to touch in this phase).**
- `docs/specs/smelt_yml.md`, `docs/specs/run_state.md`

**Docs touched.** Spec only in this phase; docs-site pages ride with the implementing phases.

**Review checklist** (material findings only):
- [ ] Every fail-loud rule is stated as MUST (no "should")
- [ ] Layout migration rule is deterministic (who adopts legacy state, when)
- [ ] Timeless: no phase vocabulary in spec body

**Commit.** `spec: env interpolation in smelt.yml; versioned/locked/per-target run state, resume + run reports`

### Phase 2: `${VAR}` environment interpolation in `smelt.yml`

**Goal.** Config values can reference secrets via the environment; a missing variable is a hard, named error. Unblocks W4's Spark `connect_url` secret handling.

**Pre-conditions.** Phase 1 merged.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs::env_interpolation_resolves_var` — `connect_url: sc://${SPARK_HOST}:15002` resolves against a set var.
- `crates/smelt-core/src/config.rs::env_interpolation_missing_var_is_error` — error message names both `SPARK_HOST` and the key path `targets.prod.connect_url`; asserts no silent empty string.
- `crates/smelt-core/src/config.rs::env_interpolation_double_dollar_escapes` — `$$` yields literal `$`, no lookup.
- `crates/smelt-cli/tests/example_diagnostics.rs` stays green (no example uses `${`).

**Implementation shape.** Interpolate after YAML parse, before validation, in `Config::load` (`crates/smelt-core/src/config.rs:877`): walk the deserialized `serde_yaml::Value` (or the typed struct's string fields via a small visitor) replacing `${NAME}`; collect *all* missing vars into one error. Use an injectable `env_lookup: &dyn Fn(&str) -> Option<String>` so tests don't mutate process env.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs` — interpolation pass + tests

**Docs touched.**
- `docs-site/docs/reference/smelt-yml.md` — "Environment interpolation" section with a secrets example (Spark `connect_url`).

**Review checklist**:
- [ ] No `std::env::var` panic path; missing var can never yield `""`
- [ ] All missing vars reported at once, with key paths
- [ ] Interpolation happens exactly once, in `Config::load`

**Commit.** `feat(config): ${VAR} environment interpolation in smelt.yml with fail-loud missing-var errors`

### Phase 3: State store hardening — atomic writes, advisory lock, versioned schema

**Goal.** `.smelt/` survives crashes and concurrent invocations, and future layout changes are detectable.

**Pre-conditions.** Phase 1 merged.

**TDD tests to write first.**
- `crates/smelt-state/src/file_store.rs::atomic_write_leaves_no_temp_files` — after `save_intervals`, directory contains only the final file; content round-trips.
- `crates/smelt-state/src/file_store.rs::second_lock_holder_gets_fail_loud_error` — two `FileStore` lock guards on one dir: second acquisition errors, message contains the holder PID.
- `crates/smelt-state/src/file_store.rs::future_state_version_is_hard_error` — `meta.json` with `state_version: 99` makes every load fail loudly.
- `crates/smelt-state/src/file_store.rs::missing_meta_json_is_legacy_and_upgraded` — pre-existing v0 layout gains `meta.json` on first locked open.

**Implementation shape.** Add `write_json_atomic(path, value)` (write `path.tmp`, fsync, rename) and route all seven `save_*` methods in `crates/smelt-state/src/file_store.rs` through it. Add `FileStore::lock() -> Result<StateLock>` using an exclusive advisory lock on `.smelt/lock` (e.g. `fs4`/`fs2` crate; write holder PID into the file for the error message); `execute_project` acquires it for the run's duration. Add `meta.json` load/validate/upgrade on `FileStore::new`→first access.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/file_store.rs`, `crates/smelt-state/src/lib.rs`, `crates/smelt-state/Cargo.toml`
- `crates/smelt-runtime/src/execute.rs` — acquire/release the lock around the run

**Docs touched.**
- `docs-site/docs/reference/cli.md` — note on the state lock error and what to do about it.

**Review checklist**:
- [ ] Every `std::fs::write` in `file_store.rs` replaced (grep is the check)
- [ ] Lock is held for the whole run, released on error paths too (RAII guard)
- [ ] Unknown-future version cannot be silently ignored (fail-loud gate)

**Commit.** `feat(state): atomic writes, single-writer advisory lock, versioned state schema in .smelt/`

### Phase 4: Per-target state partitioning

**Goal.** State for target `dev` can never contaminate target `prod` — the minimal v0.5 environments answer (master decision D4).

**Pre-conditions.** Phase 3 (versioning carries the layout migration).

**TDD tests to write first.**
- `crates/smelt-state/src/file_store.rs::stores_for_different_targets_are_disjoint` — writes via `FileStore::new(dir, "dev")` invisible to `FileStore::new(dir, "prod")`.
- `crates/smelt-state/src/file_store.rs::legacy_root_state_migrates_to_first_run_target` — v0 root-level `intervals.json` moves under `targets/<t>/` per the Phase 1 spec rule.
- `crates/smelt-cli/tests/incremental` (existing suite) green — proves the default single-target path is unchanged behaviourally.

**Implementation shape.** `FileStore::new(project_dir, target)` re-roots all paths at `.smelt/targets/<target>/`; migration runs under the Phase 3 lock during the v0→v1 upgrade. Thread the target name from `ExecuteRequest.target` (`crates/smelt-runtime/src/types.rs:26`) through the `FileStore` construction sites (`rg -n 'FileStore::new'` — runtime, cli, maintenance_conformance harness).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/file_store.rs`
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-cli/src/**` — construction sites only

**Docs touched.**
- `docs-site/docs/reference/smelt-yml.md` + `docs-site/docs/reference/cli.md` — state isolation per target.

**Review checklist**:
- [ ] No code path constructs a `FileStore` without a target
- [ ] `cargo test -p smelt-cli --test maintenance_conformance` green
- [ ] Migration is idempotent and lock-protected

**Commit.** `feat(state): per-target state partitioning under .smelt/targets/<target>/`

### Phase 5: DAG-parallel execution with `--jobs`

**Goal.** Replace the sequential model loop with a topological wavefront scheduler; wall-clock scales with DAG width, semantics unchanged.

**Pre-conditions.** Phase 3 (state lock exists; per-run state writes serialized).

**TDD tests to write first.**
- `crates/smelt-runtime/tests/parallel_execution.rs::jobs_1_report_identical_to_default_pipeline` — `--jobs 1` output = pre-change behaviour on `examples/timeseries`.
- `crates/smelt-runtime/tests/parallel_execution.rs::upstream_always_completes_before_downstream` — with `--jobs 4`, per-model completion timestamps respect every DAG edge.
- `crates/smelt-runtime/tests/parallel_execution.rs::report_order_deterministic_across_runs` — reporter events and manifest ordering equal across two `--jobs 4` runs.
- `cargo test -p smelt-runtime --test execute_parity` — unchanged, green (run-pipeline-parity invariant).

**Implementation shape.** In `crates/smelt-runtime/src/execute.rs`, keep the existing per-model planning (`model_plans`) exactly as-is; replace the loop at `execute.rs:691` with a scheduler: ready-set derived from `DependencyGraph` in-degrees (`crates/smelt-core/src/graph.rs::execution_order` generalized to expose deps), a `std::thread::scope` worker pool of `jobs` threads (backends are per-thread connections — audit `Backend` `Send` bounds first; if a backend is not `Send`, fall back to per-worker backend construction via the existing factory). Add `jobs: Option<usize>` to `ExecuteRequest` (`crates/smelt-runtime/src/types.rs`) + `--jobs` in the CLI run command (`crates/smelt-cli/src/commands/`). First failure stops *scheduling*; in-flight models drain; all outcomes recorded. Buffer per-model reporter events and flush in `execution_order` sequence so output stays deterministic.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-runtime/src/types.rs`
- `crates/smelt-core/src/graph.rs` — expose edge/in-degree view
- `crates/smelt-cli/src/commands/**` — `--jobs` flag

**Docs touched.**
- `docs-site/docs/reference/cli.md` — `--jobs` (default: available parallelism; `1` = serial).

**Review checklist**:
- [ ] `execute_parity` + `statement_parity` + `maintenance_conformance` green
- [ ] No new `pub` leaks from `smelt-runtime` internals (parity rule)
- [ ] Failure in one wavefront cannot orphan the state lock or skip manifest writes

**Commit.** `feat(runtime): DAG-parallel model execution with --jobs, deterministic reporting`

### Phase 6: Bounded retry for transient backend errors

**Goal.** A flaky connection does not fail a 2-hour run; a deterministic SQL/type error is never retried.

**Pre-conditions.** Phase 5 (retry wraps the per-model execution unit the scheduler dispatches).

**TDD tests to write first.**
- `crates/smelt-runtime/tests/retry.rs::transient_failure_retries_then_succeeds` — injected failing-twice backend wrapper; model succeeds; reporter shows 2 retry events.
- `crates/smelt-runtime/tests/retry.rs::sql_error_is_not_retried` — syntactically valid but semantically failing SQL (division via `CAST('x' AS INT)`) fails once, zero retries.
- `crates/smelt-runtime/tests/retry.rs::retries_exhausted_fails_model` — bounded at the configured max.

**Implementation shape.** Classify at the backend-error boundary: add `is_transient(&BackendError) -> bool` (connection/IO/timeout = transient; SQL/type/constraint = deterministic) where the backend trait's error type lives; retry loop (max 3, exponential backoff, jitter from run_id hash — no `Date::now` coupling in tests) around the statement-group execution for one model. `retry_max`/`retry_backoff_ms` on `ExecuteRequest` with defaults; test wrapper backend lives in `crates/smelt-runtime/tests/` support code.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs` (or the statement-execution seam it calls)
- backend error type crate (locate via `rg -n 'enum .*Error' crates/smelt-backend*/src crates/smelt-core/src`)

**Docs touched.**
- `docs-site/docs/reference/cli.md` — retry behaviour + how to disable (`retry_max: 0`).

**Review checklist**:
- [ ] Transient classification is an explicit match, not a string sniff, and fail-loud on new variants
- [ ] A retried model re-executes its *whole* statement group (no partial-write replay hazard); cite the maintenance-plan transactionality rule
- [ ] `maintenance_conformance` green

**Commit.** `feat(runtime): bounded retry with backoff for transient backend errors`

### Phase 7: `--resume` from partial failure

**Goal.** After a failed run, `smelt run --resume` re-executes only what didn't succeed, fail-loud when the premise doesn't hold.

**Pre-conditions.** Phases 4–5 (per-target manifests; scheduler records all outcomes). Extends `ModelRunRecord` (`crates/smelt-state/src/lib.rs:26`) with an `outcome` field (`success`/`failed`/`skipped`) — today only successes/skips are recorded and the run aborts before persisting the failure (`execute.rs:908`).

**TDD tests to write first.**
- `crates/smelt-cli/tests/resume.rs::resume_skips_previously_succeeded_models` — 3-model chain, middle model fails (injected), rerun with `--resume` executes only middle + downstream.
- `crates/smelt-cli/tests/resume.rs::resume_reruns_model_whose_definition_changed` — edit the succeeded upstream's SQL between runs; `--resume` re-executes it.
- `crates/smelt-cli/tests/resume.rs::resume_without_failed_run_is_error` — fail-loud when the latest run completed successfully or no manifest exists.
- `crates/smelt-state/src/file_store.rs::failed_run_manifest_persists_all_outcomes` — manifest written even when the run errors.

**Implementation shape.** Persist the manifest on the failure path (move `save_run` into a drop-guard/finally around the scheduler). Record a definition hash per model in `ModelRunRecord` (hash of compiled SQL — available at plan time). `--resume` on the CLI → `resume: bool` on `ExecuteRequest`; selection intersects with "not `success` in latest incomplete run, or hash changed, plus all downstreams of anything re-run". Interval-ledger writes already track materialized windows (`run_state.md` §Interval ledger) — resume must not double-apply them: a skipped model's intervals are untouched.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/lib.rs`, `crates/smelt-state/src/file_store.rs`
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-runtime/src/types.rs`
- `crates/smelt-cli/src/commands/**`

**Docs touched.**
- `docs-site/docs/reference/cli.md` — `--resume` semantics incl. when it refuses.

**Review checklist**:
- [ ] Resume + incremental interplay proven: a resumed run's final state equals a clean rerun's (add a `maintenance_conformance`-style assertion on the resumed fixture)
- [ ] Manifest schema change covered by Phase 3's `state_version` bump rule
- [ ] Refusal paths are errors, not warnings

**Commit.** `feat(cli): smelt run --resume — skip-succeeded rerun after partial failure`

### Phase 8: Run-report artifact, structured logs, failure summary

**Goal.** An orchestrator (Airflow/cron) can consume a machine-readable per-run report and JSON logs; a human gets a readable end-of-run failure summary.

**Pre-conditions.** Phase 7 (outcomes exist for every model).

**TDD tests to write first.**
- `crates/smelt-cli/tests/run_report.rs::report_written_on_success_and_on_failure` — `.smelt/targets/<t>/reports/<run_id>.json` exists in both cases; schema fields per `run_state.md` (models, outcomes, durations, row counts, error strings, retry counts).
- `crates/smelt-cli/tests/run_report.rs::log_format_json_emits_parseable_lines` — `--log-format json` → every stderr line parses as JSON.
- `crates/smelt-cli/tests/run_report.rs::failure_summary_lists_all_failed_models` — multi-failure run prints one summary block naming each failed model + first error line.

**Implementation shape.** Report is a serialization of the (now-complete) `RunManifest` plus request echo + versions — derive it in the reporter layer (`crates/smelt-runtime/src/reporter.rs` / `crates/smelt-cli/src/reporter.rs`), written by the same drop-guard as Phase 7's manifest. `--log-format {text,json}` selects the `tracing_subscriber` formatter in CLI init. Failure summary composes from recorded outcomes at run end (this is user-facing stdout in `smelt-cli` — println gate exempt there, per CLAUDE.md).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/reporter.rs`, `crates/smelt-cli/src/main.rs`, `crates/smelt-runtime/src/reporter.rs`

**Docs touched.**
- `docs-site/docs/reference/cli.md` — report schema + `--log-format`; groundwork the W6 deployment guide links to.

**Review checklist**:
- [ ] Report schema documented in `run_state.md` §Surface matches emitted JSON field-for-field
- [ ] No `println!` added to library crates (hardening gate)
- [ ] Report written even on panic-free early errors (compile failure)

**Commit.** `feat(cli): per-run JSON report artifact, --log-format json, end-of-run failure summary`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test execute_parity` and `--test statement_parity` — run-pipeline + statement-emission parity unchanged
- `cargo test -p smelt-cli --test maintenance_conformance` — equivalence invariant survives parallel/retry/resume scheduling
- `examples/timeseries`: `smelt run --jobs 4`, kill a model mid-run (injected failure), `smelt run --resume`, then compare against a fresh full build — identical state
- `/smelt:validate smelt_yml` and `/smelt:validate run_state` report zero drift
