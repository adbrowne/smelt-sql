# Phase 9 summary — `smelt explain` renders decomposed state as internal state

## Shipped

- `smelt_logical::rules::cumulative::state_column_summary(&CumulativeClassification) ->
  Vec<StateColumnSummary>` (re-exported from the crate root) — a pure read of each
  `AggregatorColumn::state`, never a re-derivation of which spellings are state-bearing.
- `MaintenancePlanResult.state_columns: Vec<StateColumnSummary>`
  (`crates/smelt-db/src/queries/maintenance.rs`), populated only by
  `smelt_db::maintenance_plan_report` (`crates/smelt-db/src/lib.rs`) for a keyed model —
  it builds the `SourceTimeseriesMap` from the already-resolved `SourceInfo`s, calls
  `classify_cumulative`, and folds the result through `state_column_summary`. Every other
  `MaintenancePlanResult` construction site leaves it empty.
- `build_maintenance_plan_report` (`crates/smelt-cli/src/explain.rs`) prints a "State
  columns:" section after the cells block, naming each hidden state column and the
  presentation expression, and stating it is not part of the model's public schema —
  omitted entirely (no empty header) when `state_columns` is empty.
- `ExplainMaintenanceJson.state_columns` + `build_maintenance_plan_json`'s new parameter —
  the `--json` (with or without `--show-sql`) report now carries the same array.
- Spec edits: `incremental_models.md` §Surface "CLI" and §"Decomposed state (rung 2) in
  keyed models"; `cli.md` §"`smelt explain <model>` maintenance-plan report" (text + JSON
  shape); docs-site `smelt-explain.md` new "Internal state columns" section,
  `cumulative-aggregate.md` cross-links from the order-monotone/once-write/decomposed-fold
  paragraphs.
- 9 new tests: 3 in `smelt-logical` (`state_summary_reports_hidden_columns_for_avg`,
  `state_summary_is_empty_for_stateless_columns`,
  `state_summary_covers_order_monotone_and_once_write`), 2 in `smelt-db`
  (`crates/smelt-db/tests/maintenance_plan_state_columns.rs`, new file), 3 core +
  a json test in `smelt-cli/tests/explain_maintenance.rs` (all 7 tests in that file,
  including 4 pre-existing, pass).

## Decisions

- 2026-08-09 (implement 9): `state_column_summary` takes `&CumulativeClassification` (not
  `&[AggregatorColumn]`) per the plan's own signature — matches the test fixtures'
  `classify_cumulative(...)`-then-summarize shape and keeps one obvious call site.
- 2026-08-09 (implement 9): the CLI tests for the rendered section and the `--json` array
  build their own tempdir fixture (a keyed `AVG`/`SUM` model over a declared clocked
  source) rather than adding a new model to `examples/`, matching
  `explain_maintenance.rs::degenerate_plan_visibly_reported`'s existing pattern — avoids
  perturbing the example workspaces every other gate (`example_diagnostics`,
  `maintenance_conformance`) also exercises.
- 2026-08-09 (implement 9): `build_maintenance_plan_json` grew an 8th parameter (clippy's
  `too_many_arguments` threshold is 7) rather than bundling `state_columns` into an
  existing parameter or a new struct — the function already takes 7 loosely-related
  pieces of already-derived data; adding one more scalar Vec and silencing the lint with
  `#[allow(clippy::too_many_arguments)]` (already used elsewhere in this crate for the
  same reason) was the smallest change matching the existing style.

## For the next planner

- This was the outcome's last row (9 of 9). Nothing identified as follow-up work within
  scope; the outcome's success criteria are all now met (verified against the audit in
  plan 9's decision-log entry: criteria 1–6 all landed across phases 5–9).
- Not done, out of scope for this outcome per its own "Out of scope" section: ladder rungs
  3–4, approximate-sketch state, and the `smelt.latest`/`smelt.once`/`smelt.current`
  pattern functions.
- One deliberately-deferred refactor surfaced in phase 7's summary (unrelated to this
  phase): collapsing `(cross_partition_combiner, state)` into a single `ColumnFold` enum —
  noted there as "for a later outcome," still true.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — 4/4 pass (no new whole-text scans).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_show_sql` — 7/7 + 6/6
  pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — 53/53 pass, unchanged.
