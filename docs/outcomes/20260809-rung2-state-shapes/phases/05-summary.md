# Phase 5 summary — admission: `MAX_BY`/`MIN_BY` without the companion projection

## Shipped

- `classify_order_monotone_column` (`crates/smelt-logical/src/rules/cumulative.rs`) now calls
  `decomposed_state::decompose_to_state` and admits with `state: Some(...)` — no companion
  `MAX(<ordering>)`/`MIN(<ordering>)` projection required. `order_monotone_companion` and its
  doc comment are deleted; `derive_fold_spec` (`crates/smelt-db/src/queries/maintenance.rs`)
  drops the same call but keeps the exact-2-argument check.
- `CrossPartitionCombiner::OrderMonotone`'s `ordering_column` now names the hidden `<alias>__o`
  state column, not a user-visible companion.
- Fixed a real correctness bug in `expand_aggregator_column_folds`/`substitute_identifier`
  (`crates/smelt-runtime/src/cumulative.rs`): sequential per-name substitution let one state
  column's merged fold text (e.g. `MAX_BY`'s `v` column embedding `target.status__o`) get
  re-substituted by a later column's pass, corrupting the recomputed presented column.
  Replaced with `substitute_identifiers`, one simultaneous pass over the original text.
- Fixed a real wiring bug: `state_augmented_projection` was being applied to the compiled,
  type-cast-wrapped SQL (`_smelt_typed` wrapper), which only exposes the model's own presented
  columns — a state expression like `ARG_MAX(val, d)` needs the model's raw source columns,
  only in scope before the cast wrap. Moved state augmentation to run on the raw pre-compile SQL
  in both `execute_windowed_keyed` and `execute_snapshot_reconcile`.
- `maintenance_fold_spec_companion.rs`'s two `*_is_not_admitted` tests flip to
  `*_is_admitted`; `keyed_families.rs` gets a companion-less admit test, a self-companion test,
  a redundant-companion-still-admits test, a wrong-arity refusal test, and the first real-SQL
  exercise of `KeyedStateColumnCollision`; `emit_statements.rs` gets a real-SQL-driven fold test.
- Spec: deleted the now-false "no decomposed-state storage wired in" Known Divergences bullet;
  reworded the `MAX_BY(x, x)` degenerate case (no one-column special case); updated the rung-2
  wiring status bullet to reflect what's landed vs. what's still row 6. docs-site's
  `cumulative-aggregate.md` MAX_BY/MIN_BY paragraph rewritten to drop the companion requirement.

## Decisions

- Single admission path, no stateless fast path — matches the plan's decision; a model that
  already projects `MAX(ord)` keeps it as an ordinary extremal-fold output column, unrelated to
  the `MAX_BY` column's own proof.
- `maintenance_conformance`'s equivalence assertion for `KeyedRecipe` (`gate.rs`) needed a new
  `presented_columns_select` helper: comparing `SELECT * FROM main.<model>` against the
  full-refresh oracle broke once the physical table carried hidden state columns the oracle
  doesn't produce. Filters columns via `information_schema.columns ... NOT LIKE '%__%'`,
  mirroring the reserved-suffix convention `KeyedStateColumnCollision` already polices.

## For the next planner

- The two bugs above (fold-substitution corruption, pre-cast state augmentation) were latent
  since phase 3/4 but only became reachable through a real end-to-end execution path once an
  admitted family's state columns cross-reference each other by name (`v` embedding `o`) or
  the state expression needs raw source columns not visible post-cast. Row 6 (AVG/once-write
  keyed-grain folding) should re-verify both paths hold for those shapes too — AVG's state
  columns don't cross-reference each other, so the substitution bug is dormant there, but the
  pre-cast wiring fix generalizes and is already in place.
- `keyed_enriched_pool`/`composed_keyed_pool` equivalence assertions in `gate.rs` still do bare
  `SELECT * FROM main.<model>` — currently safe since neither recipe generator produces a
  state-bearing combiner, but row 6/7 recipe generators should route through
  `presented_columns_select` (or a shared variant) rather than re-discovering this bug.
- Out of scope, not done: once-write fallback/multi-candidate admission and AVG/stddev keyed
  folding (row 6); new decomposed-state conformance recipes with downstream `SELECT *`
  consumers (row 7).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, example_diagnostics).
- `cargo test -p smelt-logical --test keyed_families --test emit_statements --test walk_coverage` — pass.
- `cargo test -p smelt-db --test maintenance_fold_spec_companion` — pass.
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test execute_parity` — pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — 47/47 pass (the phase's strongest
  gate; caught both bugs above before this summary was written).
