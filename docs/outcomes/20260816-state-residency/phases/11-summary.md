# Phase 11 summary — close-out: full standing-gate sweep, live Spark leg, criteria judgment

HEAD at start: `f4a7754daa2606127dc4a37d0e4861468f9f23c6` (outcome(20260816-state-residency): plan phase 11), branch `delta-signature-closure`, clean tree.

## Shipped

- No feature code changed. This phase is pure verification/close-out per its own plan.
- Bootstrapped the Spark client venv (`.smelt-spark-venv/`, gitignored) in this worktree —
  absent here because this worktree had never run the live-Spark leg before.
- Rebound the `smelt-spark` Docker container to this worktree (it was bind-mounted to the
  now-idle `worktree-production` checkout; confirmed no tmux session or process was actively
  driving that worktree before running `spark-down.sh && spark-up.sh` here).

## Gates (all read directly, none assumed)

- `bash .claude/scripts/verify-phase.sh` (env: `DUCKDB_LIB_DIR=~/.local/lib/duckdb`,
  `LD_LIBRARY_PATH` likewise, `CARGO_INCREMENTAL=0`) — **ALL GREEN**: fmt-check, clippy
  zero-warnings, full `cargo test` (workspace), `example_diagnostics`. The
  `prop_smelt_valid_implies_spark_valid` flake phase 10 flagged did **not** reappear on this
  run — no `.proptest-regressions` file was produced or committed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 76 passed, 0 failed.
- `cargo test -p smelt-runtime --test statement_parity` — 23 passed, 0 failed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed, 0 failed.
- `cargo test -p smelt-runtime --test execute_parity` — 4 passed, 0 failed.
- `cargo test -p smelt-cli --test state_docs` — 3 passed, 0 failed.
- `cargo test -p smelt-runtime --test frontier_residency` — 5 passed, 0 failed (this outcome's
  "state_deletion" leg lives inside `maintenance_conformance` above, already counted there).
- `cargo test -p smelt-runtime --test state_posture` — 8 passed, 0 failed.
- **Live Spark leg**: `bash scripts/spark-up.sh` (from this worktree) →
  `source scripts/spark-env.sh` →
  `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark -- --test-threads=1`
  — **19 passed, 0 failed**, 182.8s. First attempt without `--test-threads=1` produced 18
  failures, all `DELTA_CREATE_TABLE_WITH_NON_EMPTY_LOCATION` on the shared `smelt_conf_gen`
  Delta warehouse schema — this is the exact hazard the test binary's own header comment
  documents ("Run with `-- --test-threads=1`... two tests racing on the same physical Delta
  table... is a real hazard this binary has not yet removed"), not a real regression. Rerun
  serialized was clean. Container torn down after (`spark-down.sh`).
- `/smelt:validate state` — ran the real slash-command flow via a subagent. **Verdict: PASS,
  0 drift items** (one non-blocking scope note under Invariant drift — posture×backend
  combinatorics were spot-checked rather than exhaustively re-verified, not counted as drift).
  Confirmed no `Phase [A-Z0-9]+` leakage in `state.md`/`run_state.md`/`incremental_models.md`
  or the docs-site pages the spec references; confirmed both diagnostic codes
  (`MaintenanceStateDowngraded`, `DeclaredContractRequiresState`) and the advisory-string
  `ProbeBaselineUnavailable` exist as specified; confirmed the reconciliation/frontier ledgers
  are engine-resident and DuckDB-only exactly as the spec's own Known Divergences bullet says.
- Mechanical Known-Divergences check: `rg "ignores.*state.mode|state.mode.*ignor"` and
  `rg "runtime ignores"` over `docs/specs/*.md` — zero hits. The bullet phase 1/10 claimed
  removed is actually gone.

## Criteria judgment

1. **`state.mode` is consulted** — met. `phases/02-summary.md` threads `StateMode` through
   `execute_project`/`FileStore`; `state_posture.rs` (8/8 passing) proves each posture writes
   exactly its assigned families and `stateless` creates no `.smelt/` dir at all.
2. **Reconciliation ledger is engine-resident** — met. `phases/04-summary.md` (engine-resident
   table + migration) and `phases/07-summary.md` (frontier reset fused into the fold's own
   write transaction, closing the flagged gap) together satisfy the "transactional with the
   fold" wording; `frontier_residency.rs` (5/5) and the mechanical grep above confirm the
   `.smelt/reconciliation.json` bullet is gone from all three owning specs.
3. **Availability resolution in derivation** — met. `phases/05-summary.md` (two-step
   ideal-then-availability pass, `MaintenanceStateDowngraded`) and `phases/06-summary.md`
   (`DeclaredContractRequiresState`) both landed with dedicated e2e tests; `explain.rs`
   visibility completed in `phases/09-summary.md` (target-dialect-aware `StateAvailability`
   instead of `all()`).
4. **Absent-state behaviour specified + implemented everywhere the optionality rule requires**
   — met. The spec half landed in `phases/01-summary.md` even though row 1 in the table above
   still reads `blocked` — per the 2026-08-16 phase-10 Decision log entry, the spec-delta
   *content* of phase 1 shipped; only its own verification gate (the pre-existing
   `contract_lattice_spec` heading-lookup regression) was blocked and repaired instead in
   phase 2. The implementation half (`ProbeBaselineUnavailable` emission,
   absent-schema-snapshot degradation) is `phases/03-summary.md`.
5. **State-deletion conformance leg** — met. `phases/08-summary.md` added `DropStateDir`/
   `FreshClone` steps to the generative pool; both are exercised for keyed additive folds and
   idempotent-graded region-recompute models, re-confirmed green in this phase's
   `maintenance_conformance` run (76/76, including the two `state_deletion::*` tests) and in
   the live-Spark run (19/19).
6. **All standing gates green + `/smelt:validate state` no drift + Known Divergences bullets
   actually removed** — met, per the gate list above.

All six success criteria are met with direct evidence from this phase's own gate runs.

## Decisions

- 2026-08-16: rebound the `smelt-spark` Docker container from the stale `worktree-production`
  bind mount to this worktree after confirming (via `ps aux` + `tmux capture-pane`) no active
  session was using that worktree — safe per
  `project_spark_container_worktree_binding` precedent.
- 2026-08-16: the 18 Spark-leg failures on the first (unserialized) run are the test binary's
  own documented `--test-threads=1` hazard, not a regression; recorded here rather than
  investigated as a bug, per the plan's instruction not to paper over red gates but also not
  to chase a documented, already-tracked hazard.
- 2026-08-16: outcome status flips to `done` — all six criteria met with gate evidence; no
  criterion left unmet, so no `## Blocked` entry is needed.

## Out of scope (unchanged from prior phases)

- The `prop_smelt_valid_implies_spark_valid` flake did not reproduce this run; still tracked as
  out of scope (unrelated `smelt-parser-compat` crate, SQL-dialect divergence) per phase 10.
- Per-test schema/table namespacing for the Spark conformance binary (would remove the
  `--test-threads=1` requirement) — the binary's own header comment already tracks this as
  follow-up, not blocking.
