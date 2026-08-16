# Phase 8 summary — state-deletion conformance leg

## Shipped

- `crates/smelt-maintenance-testkit/src/schedule_gen.rs`: `ConformanceStep::DropStateDir`/
  `FreshClone` (excluded from `is_permutable`), plus a shared `StateResidencyOp` enum
  (`DropStateDir`/`FreshClone`) for the keyed pool's index-keyed injection.
- `crates/smelt-maintenance-testkit/src/link_c_harness.rs`: `LinkCProject` derives `Clone`;
  new `LinkCProject::fresh_clone(dest)` copies `models/` (recursively) + `smelt.yml`, never
  `.smelt/`, reusing the same warehouse `db_path`; a `copy_dir_recursive` helper backs it.
  Unit test `fresh_clone_copies_models_but_not_state`.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs`: `drive_and_assert` holds a local
  `let mut project = project.clone()` so `FreshClone`/`DropStateDir` steps can reassign/mutate
  it mid-loop; new `pub fn drop_state_dir`; `drive_keyed_and_assert_with_state_ops` (index-keyed
  `BTreeMap<usize, StateResidencyOp>`, applied before that window), with `drive_keyed_and_assert`
  delegating an empty map.
- `crates/smelt-cli/tests/maintenance_conformance_spark/gate_spark.rs`: residency steps
  `bail!` naming the ledger-less-backend downgrade reason (compile-checked, not run).
- New `crates/smelt-cli/tests/maintenance_conformance/state_deletion.rs`, registered in
  `main.rs`: all 6 planned tests plus a 7th (`keyed_schedule_with_residency_op_preserves_
  equivalence`) exercising the new keyed hook directly.

## Decisions

- No product-code changes were needed — every residency step exercised already-correct,
  already-engine-resident behaviour (phases 4/5/7). `git diff --stat -- crates/` touches only
  test-target files (`tests/`) and `smelt-maintenance-testkit/src/` (a dev-only, non-production
  harness crate never shipped), matching the plan's "no defect found" branch.
- The flagship test (`keyed_additive_fold_survives_state_dir_deletion`) asserts the redelivery
  still **refuses** (`KeyedReprocessedWindow`) after the `.smelt/` drop — matching the
  pre-existing `redelivered_window_refuses_for_additive_keyed` probe's contract. A silent
  post-drop success would have meant the ledger reset, which is exactly the bug class this leg
  exists to catch.

## For the next planner

- Row 9 (docs-site + Known Divergences sweep) is now unblocked — every criterion-5 claim is
  proven end to end.
- No new follow-up work surfaced; the ledger/frontier residency held under every injected
  scenario (redelivery, generative mid-schedule drop, generative fresh clone, region-recompute
  rerun).

## Gates

- `bash .claude/scripts/verify-phase.sh` (full): PASS (fmt, clippy zero-warnings, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test maintenance_conformance`: 76 passed.
- `cargo test -p smelt-maintenance-testkit`: 31 passed (incl. the new `fresh_clone` unit test).
- `cargo check -p smelt-cli --features smelt-cli/spark --tests`: clean (Spark twin compiles
  against the widened enum, including the residency `bail!` arms).
- `cargo test -p smelt-runtime --test frontier_residency --test state_posture`: 5 + 8 passed.
- Anti-vacuity: temporarily disabled `remove_dir_all` in `drop_state_dir` — confirmed
  `drop_state_dir_step_actually_removes_the_directory` fails; restored, confirmed green again.
