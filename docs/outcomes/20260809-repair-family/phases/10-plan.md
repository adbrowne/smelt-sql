# Phase 10 plan — repair over a decomposed combiner

## Objective

A live repair over a decomposed combiner (`MAX_BY`, the `(value, written)` once-write
spellings, the decomposed folds) currently crashes: `repair_candidate_select` wraps the
model's plain PRESENTED projection, while the physical table the fold's own create path
built carries the extra `__`-marked hidden state columns, so the repair's `INSERT` supplies
fewer columns than the table has. This phase makes the repair candidate the *state-augmented*
projection — the same widening `execute_windowed_keyed`/`execute_snapshot_reconcile` already
apply — and makes the diff-patch suppression compare state columns too. Advances success
criteria 1, 4 and 6 (equivalence coverage today reaches only `Idempotent`-shaped combiners).

## Spec delta

`docs/specs/incremental_models.md`:

- §"The repair family" — add a short subsection "Repair over a decomposed combiner": the
  candidate relation a repair stages is the model's **state-augmented** projection
  (§"Decomposed state (rung 2) in keyed models"), identical to the projection the fold's own
  create/merge path materializes, so a repaired group's row carries its hidden state as well
  as its presented columns; and the `diff_patch` change-suppression predicate compares the
  hidden state columns alongside the presented compared columns, so a group whose presented
  value is unchanged but whose state moved is still rewritten (suppressing it would leave
  stale state behind a correct-looking value).
- §Known Divergences — delete the entry "**The repair family's affected-key recompute ignores
  a decomposed combiner's hidden state.**" (closed by this phase).

## Tests

1. `smelt-logical` (`rules/cumulative.rs` unit) —
   `cumulative_classification_state_columns_collects_every_state_bearing_column`: the new
   `CumulativeClassification::state_columns()` returns every state-bearing aggregator column's
   `StateColumn`s in column order, empty for a stateless classification.
2. `smelt-runtime` (`maintenance_driver.rs` unit) —
   `repair_augmented_model_sql_appends_state_columns`: the new helper returns the model SQL
   widened with one `, <per_partition_expr> AS <name>` per state column, and returns the SQL
   unchanged for an empty state-column list; an unparseable body errors by name.
3. `smelt-runtime` (`tests/repair_lowering.rs`) —
   `repair_candidate_select_carries_hidden_state_columns`: a candidate select built over the
   augmented SQL projects the state column aliases (a repair `INSERT` therefore matches the
   fold-created table's column list).
4. `smelt-runtime` (`tests/repair_lowering.rs`) —
   `diff_patch_compared_columns_include_hidden_state`: the compared-column set handed to
   `emit_diff_patch` for a state-bearing cell contains the state column names, so the emitted
   suppression predicate mentions them.
5. `smelt-cli` (`tests/maintenance_conformance/repair.rs`) —
   `repair_pool_upholds_equivalence_under_retraction`: `KeyedCombiner::OrderMonotone` joins
   `Idempotent` in the mutation loop (insert → update-in-place → delete), equivalence asserted
   against the full-refresh oracle after every step. Red today (INSERT column-count mismatch).
6. `smelt-cli` (`tests/maintenance_conformance/repair.rs`) —
   `diff_patch_repair_over_decomposed_state_upholds_equivalence`: a
   `RepairRecipe::new(OrderMonotone, RepairWriteMode::DiffPatch)` case, equivalence after a
   retraction plus the assertion that diff-patch statements were actually executed.
7. `smelt-cli` (`tests/maintenance_conformance/registry.rs`) —
   `divergence_registry_staleness_report` still passes with the
   `known_bug_repair_candidate_select_ignores_decomposed_state` entry and its
   `known_bug_still_reproduces` arm deleted.

## Tasks

1. Spec delta above (spec-first).
2. Add `CumulativeClassification::state_columns()` in `crates/smelt-logical/src/rules/
   cumulative.rs`; replace the three existing hand-rolled `aggregator_columns.iter()
   .filter_map(|c| c.state.as_ref()).flat_map(...)` copies (`smelt-runtime/src/cumulative.rs`
   ×2, `smelt-runtime/src/diagnostics.rs`) with calls to it — one derivation, reused.
3. Add `repair_augmented_model_sql(clean_sql, &[StateColumn]) -> Result<String>` in
   `crates/smelt-runtime/src/maintenance_driver.rs` — a named wrapper over
   `emit::state_augmented_projection` with the repair path's own error text, so the widening
   is unit-testable rather than inline in `execute.rs`.
4. `crates/smelt-runtime/src/execute.rs` repair leg: augment `clean_sql_for_merge` with
   `classification.state_columns()` **before** `compile_with_sql_and_ephemerals` (raw,
   pre-compile SQL — same ordering rationale as `execute_snapshot_reconcile`), then build
   `repair_candidate_select` over the compiled augmented SQL.
5. Same leg, `RepairWrite::DiffPatch`: append the state column names to `compared_columns`
   before calling `execute_diff_patch` (strictly less suppression — sound by construction).
6. `crates/smelt-runtime/src/diagnostics.rs` `Technique::PerGroupRecompute` preview: apply the
   identical augmentation before compiling, so the preview stays byte-identical to what a run
   executes (§"Statement emission (single owner)").
7. Delete the `known_bug_repair_candidate_select_ignores_decomposed_state` registry entry, its
   `known_bug_still_reproduces` arm, and the now-stale doc comment on
   `repair_pool_upholds_equivalence_under_retraction`; extend that test's combiner list and add
   the diff-patch decomposed case (tests 5, 6).
8. Write `phases/10-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering --test diagnostics`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-logical --test walk_coverage`

## Commit message

`feat(incremental): repair a decomposed combiner's hidden state columns`
