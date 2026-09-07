# Phase 3 plan — the succession plan model and its derivation

## Objective

Give the recognised keyed-succession shape a maintenance plan: the `Grain::Succession` /
`Technique::SuccessionPatch` / `StateStructure::TombstoneLedger` triple, a pure deriver in
`smelt-logical` that turns phase 2's verdict into exactly one cell (or one refusal), and the
`smelt-db` branch that builds the classifier's world-facts and calls it. Advances criterion 3
(plan purity, availability downgrade, contract-lattice posture) and unblocks criteria 2 and 4,
which have no producer until the refusal and the cell exist.

## Spec delta

None. The plan-model surface this phase adds is already normative — `incremental_shapes.md`
§"The succession grain", §"Succession-grain admission (no declaration)", `state.md`'s tombstone
ledger row and §"The degradation contract", and the 2026-09-06 contract decision in the outcome's
decision log. No user-visible behaviour is invented here.

## Tests

`crates/smelt-logical` (unit + `tests/maintenance_availability.rs`):

- `succession_patch_requires_the_tombstone_ledger` — `required_state_structure(SuccessionPatch)`
  is `Some(TombstoneLedger)`; the match stays exhaustive over `Technique`.
- `tombstone_ledger_is_realisable_only_on_duckdb` — `realisable_state_structures` lists it for
  DuckDB and not for Spark or BigQuery; `StateAvailability::all()` contains it.
- `succession_cell_downgrades_to_full_refresh_without_a_ledger` — with `TombstoneLedger`
  unavailable the cell's technique becomes `DeleteInsert` (never `PerGroupRecompute`, never a
  ledger-less patch), with `StateDowngrade { original: SuccessionPatch, missing: TombstoneLedger }`.
- `succession_downgrade_fires_for_spark_bigquery_and_warehouse_tables_none` — one case per
  criterion-3 route, all reaching the same downgrade.
- `derive_succession_plan_yields_one_patch_cell` — a `Recognized` verdict derives exactly one
  cell: `Trigger::NewData { source }`, `Corner::FoldDelta`, `Technique::SuccessionPatch`,
  `PartitionLocal::Yes` on the source's run axis, `state_downgrade: None` out of ideal derivation.
- `succession_plan_skeleton_is_key_plus_clock` — `OutputSpec::skeleton_columns == k ∪ {t}`
  derived from the verdict, and `Grain::Succession { key_cols, clock_col }` matches it.
- `succession_refusal_plan_carries_the_classifier_reason` — a `NotSuccession` verdict derives no
  cells and exactly one `Refusal::SuccessionNotRecognized` carrying the reason verbatim.
- `succession_recognition_records_the_advisory` — a `Recognized`-with-advisory verdict still
  derives the same cell (the advisory changes nothing about admission).

`crates/smelt-db` (`tests/maintenance_*`, or the module's own unit tests):

- `undeclared_grain_incremental_model_derives_the_succession_plan` — the running example
  (`customer_history` over an `append_only`, clocked `customer_changes`) yields the succession
  cell, where today `resolved_grain()` returns `None` and derivation bails.
- `undeclared_grain_unrecognised_shape_derives_the_succession_refusal` — a `GROUP BY` body under
  the same declarations yields `Refusal::SuccessionNotRecognized`, not `None`.
- `succession_context_is_built_from_the_source_declarations` — `event_time_column` and the
  `NOT NULL` column set reach the classifier from `SourceInfo`; an undeclared profile fails closed.
- `declared_grain_models_are_unchanged` — a partition-grain and a key-grain model derive
  byte-identical plans to before (regression guard on the new branch).

`crates/smelt-logical` contract tests:

- `frozen_horizon_refused_on_a_succession_model` / `retain_departed_refused_on_a_succession_model`
  — each refuses with a message naming the succession grain (not the `Key` fallback today's
  `metadata.grain.unwrap_or(Grain::Key)` produces).
- `deferral_admitted_on_a_succession_model` — no refusal; frontier-lag semantics untouched.

## Tasks

1. Add `Grain::Succession { key_cols: Vec<String>, clock_col: String }` and
   `Technique::SuccessionPatch` to `crates/smelt-logical/src/maintenance/mod.rs`; give every
   exhaustive match over both a real succession arm — no `_` catch-all anywhere.
2. Add `StateStructure::TombstoneLedger` (+ `as_str` spelling from `state.md`), list it in
   `StateAvailability::all()` and in `realisable_state_structures`'s DuckDB arm only, and map
   `SuccessionPatch → TombstoneLedger` in `required_state_structure`.
3. Branch `recompute_equivalent` on `Technique::SuccessionPatch` → `DeleteInsert` ahead of the
   `key_scope` test, so a succession cell degrades to full refresh.
4. Add `Refusal::SuccessionNotRecognized { reason: NotSuccessionReason }` and a
   `succession_refused_plan(reason)` constructor beside `unsupported_grain_plan`.
5. Add the pure deriver (`maintenance/derive.rs` or a new `maintenance/succession.rs` if it would
   push `derive.rs` past the large-file baseline): verdict + table + source facts → the one-cell
   plan or the refusal plan. Skeleton columns come from the verdict (`k ∪ {t}`), never supplied.
6. In `crates/smelt-db/src/queries/maintenance.rs`, add `build_succession_context` over the same
   `(ref, SourceInfo)` pairs `build_key_recurrences` walks — a side channel, not two new
   `SourceFacts` fields (153 literal construction sites).
7. In `derive_model_maintenance_plan`, replace the bare `metadata.resolved_grain()?` bail with:
   `None` + `refresh: incremental` + no `unique_key`/`timeseries:` → build the context, call
   `walk::model_keyed_succession`, and return the derived succession or refusal plan; any other
   `None` keeps returning `None`. Thread the same call through
   `derive_model_maintenance_plan_with_edges` and the `smelt-runtime` availability seam.
8. Introduce the grain label the contract validators name (a small `smelt-logical` enum covering
   partition/key/key_per_partition/succession), switch `validate_frozen_horizon` and the
   `retain_departed` posture check to it, and pass the real label from
   `crates/smelt-db/src/file_check.rs` instead of `metadata.grain.unwrap_or(Grain::Key)`.
   `smelt_core::config::Grain` is untouched — succession is never a declarable value.
9. Run the gates; record any residual finding in the summary rather than papering over it.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, workspace tests,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage`
- `cargo test -p smelt-db --test maintenance_diagnostics` (unchanged codes: this phase adds no
  diagnostic, so the catalogue must be untouched).
- `cargo test -p smelt-runtime --test availability_seam --test execute_parity`
- `bash .claude/scripts/hardening-budget.sh` and `.claude/scripts/large-file-check.sh` — both
  must be at baseline with the baseline files unedited.

## Commit message

`feat(smelt-logical): derive the succession-grain maintenance plan and its tombstone-ledger availability`
