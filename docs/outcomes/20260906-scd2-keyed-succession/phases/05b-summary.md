# Phase 5b summary — Runtime dispatch of the succession cell

## Shipped

- `crates/smelt-runtime/src/maintenance_driver/succession/mod.rs` — `SuccessionCell`
  struct + `resolve_live_succession_cell(...)` (derives the plan, finds the live
  `Technique::SuccessionPatch` cell, resolves the driving source's physical table via
  `SourceInfo::db_name_for_target` and its `timeseries.partition_column`/`granularity`)
  and `build_succession_source_refs`.
- `crates/smelt-runtime/src/maintenance_driver/succession/execute.rs` —
  `execute_succession_maintenance`: per-step event-delta `SELECT`, idempotent ensure-DDL
  (tombstone ledger + presented shell) before the read-only clock-tie probe (bails
  `SuccessionClockTie` naming key/clock/violation_count/sample_keys on `Violated`), then
  the ledger-upsert + patch group through `Backend::execute_write_with_bookkeeping` in one
  transaction.
- `crates/smelt-runtime/src/maintenance_driver/succession/tests.rs` — 9 unit tests
  (bootstrap, refold idempotence, either-order convergence, delete→tombstone-not-presented,
  clock-tie refusal + rollback, no-op on identical row, failed-merge rollback,
  per-window frontier record, state-downgrade fallthrough). All green.
- `crates/smelt-runtime/tests/fixtures/succession/` (arrival-partitioned `customer_changes`
  + `customer_history`) and `.../technique_lowering/succession_patch_e2e.rs` — 2
  integration tests (`succession_model_runs_through_execute_project`,
  `late_event_in_a_later_arrival_window_splices`). Both green.
- Dispatch wired into `crates/smelt-runtime/src/execute/project.rs`, gated on
  `metadata.resolved_grain().is_none()`, immediately after the `plan_is_keyed` block and
  before `plan.incremental`; refuses by name (`--event-time-start`/`--event-time-end`)
  when no run window is given.

## Decisions

- **`smelt-core::metadata::validate_timeseries` gap not scoped in the 05b plan**: its hard
  `GrainRequiredForIncremental` refusal pre-dates the succession classifier and rejected
  every undeclared-grain `refresh: incremental` model, including real succession
  candidates, before the classifier ever ran. Fixed with a narrow syntactic pre-filter
  (`sql_may_be_succession_shaped`: a `LEAD(`/`LAG(` text scan) that only widens
  *acceptance* — the real classifier still fails closed
  (`Refusal::SuccessionNotRecognized`) for anything the prefilter admits but doesn't
  actually qualify. Documented in a doc comment per the fail-loud/heuristic convention.
  This is metadata-only pre-admission, not a composition-relevant property verdict, so
  it is outside `walk_coverage`'s scope, but the doc-comment discipline was applied anyway.
- Kept `project.rs`'s diff to the guard + one call block, per the plan — the file's
  hardening/large-file ratchets were already red going in.

## For the next planner

- **Not yet built**: no diagnostic surfacing exists for the ten `NotSuccession*`
  classifier codes in `file_check.rs` — only the `SuccessionPreFilterNegatesFlag` advisory
  is wired. A model that fails the real classifier after passing the new metadata
  pre-filter currently only surfaces as a maintenance-plan refusal, not an LSP/CLI
  diagnostic naming the specific reason. Worth a phase (3a follow-up or folded into 9).
  Serves Success criteria: user-facing clarity on why a model isn't recognized.
  - Update: 3a diagnostics (`SuccessionNotRecognized*` variants) may already cover part
    of this — check overlap before scheduling more work than needed.
- **5c (rebuild/repair, `--full-refresh`) is the next phase** and depends on this
  phase's `execute_succession_maintenance` shape — no blocking surprises found for it.
- Large-file ratchet grew on 9 files this phase touched (same pattern as 2b/3/3a/5a);
  left to the loop's dedicated shrink step, confirmed as the *only* failing test in
  `cargo test --workspace`.

## Gates

- `cargo test -p smelt-runtime --lib maintenance_driver::succession` — 9/9 pass
- `cargo test -p smelt-runtime --test technique_lowering succession` — 2/2 pass
- `cargo test -p smelt-runtime --test execute_parity` — 4/4 pass
- `cargo test -p smelt-runtime --test statement_parity` — 37/37 pass
- `cargo test -p smelt-logical --test walk_coverage` — 14/14 pass
- `cargo test --workspace --quiet` — only failure is `large_file_ratchet::gate_passes_on_committed_tree`
  (large-file regressions on 9 files; report-only per plan, loop's shrink step owns it)
- `bash .claude/scripts/verify-phase.sh` — fmt PASS, clippy PASS, workspace tests: same
  single large-file-ratchet failure, example_diagnostics PASS
