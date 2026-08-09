# Phase 7 summary — `AVG`/`STDDEV_*`/`VAR_*` decomposed folds at keyed grain

## Shipped

- `classify_cumulative`'s `OtherAggregate` arm widens onto the decomposed-fold
  family: `classify_decomposed_fold_column` (`crates/smelt-logical/src/rules/
  cumulative.rs`) attempts `decompose_to_state` before refusing, admits with
  the new `CrossPartitionCombiner::Recomputed` + derived state, and refuses
  `KeyedSnapshotSourceUnsupportedColumn` (family `"decomposed fold"`) under
  snapshot-reconcile.
- `CrossPartitionCombiner::Recomputed` — the presented column's value is
  `π(merged state)`, never a direct target/delta fold; `render` is
  unreachable by construction, guarded by `refuse()`.
- `WindowedKeyedRule::refuse`/`ledger_grade`
  (`crates/smelt-runtime/src/cumulative.rs`) are state-aware: they read
  `col.state`'s own state-column combiners first, falling back to the
  stateless per-combiner logic. `AVG`'s `(sum, count)` state grades
  `Grade::Additive` — the first additive hidden state this mechanism admits.
- `has_monoid_state_shape` (`crates/smelt-logical/src/analysis/
  decomposed_state.rs`) widens `faithful_fold`'s algebra condition — derives
  the family-level fact by calling `decompose_to_state` with placeholder
  text rather than duplicating a function list.
- `derive_fold_spec` (`crates/smelt-db/src/queries/maintenance.rs`) mirrors
  the widen with the same exact-arity/no-`DISTINCT` shape check, so
  `smelt explain`/LSP never reports a `KeyedFold` cell the runtime refuses.
- `expand_aggregator_column_folds`/`substitute_identifiers` moved from
  `smelt-runtime::cumulative` into `smelt-logical::maintenance::emit` (now
  `pub`) — single-owner statement rule. `smelt-runtime::diagnostics`'s
  `KeyedFold` preview now calls the same function (after applying
  `state_augmented_projection` pre-compile, matching the executed path's
  ordering) instead of hand-building stateless folds.
- Spec: `docs/specs/incremental_models.md` Known Divergences bullet for
  "Ladder rung 2 … partially wired" deleted (nothing residual). Docs-site:
  `AVG` removed from "Out of v1"; a decomposed-fold row + explanatory
  paragraph added to `docs-site/docs/reference/cumulative-aggregate.md`.
- 13 new tests across `keyed_families.rs` (6), `faithful_fold.rs` (2),
  `emit_statements.rs` (1), `smelt-runtime::cumulative` unit tests (3),
  `maintenance_fold_spec_companion.rs` (2, one more than planned — added a
  wrong-arity case alongside the DISTINCT case), `statement_parity.rs` (1).

## Decisions

- `has_monoid_state_shape` derives its answer by calling `decompose_to_state`
  with placeholder `"x"` argument/output text and checking every state
  column's combiner is `Sum`, rather than hand-listing the family — one
  source of truth, matches the plan's "family-level algebraic fact" framing.
- `refuse()`/`ledger_grade()` check `col.state` first for every column, not
  only `Recomputed` ones — generalizes cleanly since `MAX_BY`/once-write's
  existing state-column combiners (`OrderMonotone`, `Max`, `OnceWrite`,
  `BoolOr`) are already in the "recognised" allowlist, so behavior for
  already-admitted families is unchanged (verified by the regression test
  `max_by_and_once_write_state_stay_idempotent`).
- The `KeyedFold` preview test (`keyed_fold_preview_matches_executed_
  statement_for_state_bearing_model`) had to read `RecordingBackend::
  recorded_sql`, not `recorded_groups` — `AVG`'s new `Grade::Additive`
  routes execution through the reconciliation-ledger path
  (`Backend::execute_sql` directly), not `execute_statement_group`, unlike
  the `Idempotent`-graded `MIN`/`MAX` cells the neighboring test inspects.
  Documented inline so a future reader isn't surprised.

## For the next planner

- Row 8 (conformance-gate recipes) still needs **new** decomposed-fold
  recipes with a downstream `SELECT *` consumer — the existing 47
  `maintenance_conformance` recipes don't generate `AVG`/`STDDEV_*`/`VAR_*`
  models yet (they stayed 47/47 unchanged, as the plan required), so this
  phase adds no generative recipe coverage for the new family. Row 8 owns
  that.
- Row 9 (surface cleanup) is narrower than originally scoped: this phase
  already deleted the spec Known Divergence and the docs-site "Out of v1"
  entry (following the phase-5 precedent of not leaving a stale spec for
  multiple phases). Row 9's remaining scope is just `smelt explain` state
  rendering.
- Noted but not addressed (plan's own deferred item, decision 3 in the
  outcome log): `(cross_partition_combiner, state)` should probably collapse
  into a single `ColumnFold` enum — the `Recomputed` variant with an
  unreachable `render` is a real but accepted wart. ~40 construction sites
  make this its own future phase/outcome, not a row-7 fix.
- The `KeyedStateColumnCollision` detector, `state_augmented_projection`,
  and `presentation_projection` all reached a *third* state-bearing family
  for free (no code changes needed) — confirms the phase-3/4 mechanism
  design generalizes as intended.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings,
  full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-logical --test keyed_families --test emit_statements --test walk_coverage` — 41 + 32 + 4 passed.
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test keyed_reprocessed_window_refusal` — 19 + 27 + (bundled) passed.
- `cargo test -p smelt-db --test maintenance_fold_spec_companion` — 14 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 47/47 passed, unchanged shape.
