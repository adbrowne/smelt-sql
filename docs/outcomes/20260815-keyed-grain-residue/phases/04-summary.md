# Phase 4 summary — Derive and print execution postures

## Shipped

- `smelt_logical::rules::cumulative::{ExecutionPostures, PostureVerdict, execution_postures}` —
  pure derivation over `&[AggregatorColumn]`, single owner of the three verdicts (re-run
  tolerance, order-independence, reprocessing refusal), each with a reason naming the deciding
  column. `CumulativeClassification::execution_postures()` convenience wrapper. Re-exported from
  `smelt-logical/src/lib.rs`.
- `smelt-runtime/src/cumulative.rs`'s `WindowedKeyedRule::ledger_grade` now delegates to
  `smelt_logical::execution_postures(...).rerun_tolerant.holds` instead of re-deriving the
  additive-combiner scan inline.
- `MaintenancePlanResult` (`smelt-db/src/queries/maintenance.rs`) gained
  `execution_postures: Option<ExecutionPostures>` and `is_snapshot_reconcile: Option<bool>`,
  populated in `smelt-db/src/lib.rs`'s `maintenance_plan_report` from the same `classify_cumulative`
  call that already fills `state_columns` — no second classification.
- `smelt explain <model>` prints an `Execution postures:` block (run shape + three verdicts with
  reasons) immediately before `State columns:`, omitted entirely when `execution_postures` is
  `None`. `--json` carries the same as a top-level `execution_postures` object
  (`ExplainExecutionPosturesJson`), threaded through `build_maintenance_plan_json`.
- `docs-site/docs/reference/smelt-explain.md` — new "Execution postures" section.
- `docs/specs/incremental_shapes.md` — clarified the order-independence enumeration (additive
  fold and decomposed fold explicitly stated to qualify) and rewrote the Known Divergences bullet:
  postures are now derived and printed; only the sequential-application optimisation remains
  undone.

## Decisions

- Run shape (`window-forward`/`snapshot-reconcile`) is threaded as a second field
  (`is_snapshot_reconcile`) alongside `execution_postures` rather than folded into
  `ExecutionPostures` itself — it depends on the classification's `driving_source`, not on
  `aggregator_columns` alone, so it can't be derived by `execution_postures`'s pure column-slice
  signature (the plan's own stated reason for that signature).
- Decomposed fold (`AVG`/`STDDEV_*`/`VAR_*`) is **not** re-run tolerant — its hidden state folds
  via `Sum`, matching the admission matrix's "ledger-enforced, graded additive" and the pre-existing
  `ledger_grade` behavior exactly. An initial test draft assumed decomposed fold was re-run
  tolerant; caught by the red-green cycle and corrected before committing.

## For the next planner

- No new limitations discovered. `ledger_grade_agrees_with_shared_posture_derivation` pins the
  delegation directly against the shared derivation, so a future divergence between the runtime
  grading and `smelt explain`'s printed verdict fails loudly.
- Phase 5 (generative conformance pool nullable payload) and phase 6 (validate + close out) are
  unaffected by this phase's shape — proceed as planned.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test execution_postures --test keyed_families` — 46 passed.
- `cargo test -p smelt-runtime --lib cumulative` — 25 passed.
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test cli_docs_coverage` — all passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed (grading unchanged end-to-end).
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
- `python3 examples/web_analytics/generate_tutorial.py` re-run to refresh the one drifted
  tutorial page (`deduplication.md`), then `tutorial_freshness` re-verified green.
