# Phase 3g plan — gap 5: `Technique::KeyedFold` resolves its availability at plan time

## Objective

`required_state_structure` maps **every** `Technique::KeyedFold` cell to
`StateStructure::ReconciliationLedger`, and the windowed-keyed driver asks
`realises_reconciliation_ledger` at *execution* time and bails with
`BackendError::unsupported`. Both are wrong for the same reason: the
never-fold-twice ledger is what the **additive** grade needs, the **idempotent**
grade's ledger record is skippable re-run-tolerance bookkeeping the driver
already degrades gracefully, and the verdict belongs on the cell where
`smelt explain` can print it. This phase makes the grade a plan-time fact, makes
availability resolution grade-aware, and makes the additive cell's degraded route
the whole-target rebuild the spec already promises. Advances criteria 2 (the
whole-row `MERGE` upsert becomes reachable on a structure-less backend — 3h's
live leg), 3 (the additive fold's route is recorded on the cell and
explain-visible, not an execution-time backend error) and 8 (the downgraded cell
runs, so it can be asserted oracle-equal).

## Spec delta (first)

`docs/specs/state.md` §"The degradation contract" step 2 — state that the keyed
fold's required structure is **grade-dependent**, alongside the existing
cell-not-technique rules for `PerGroupRecompute`/`EnrichmentKeyed`/succession:

- An **idempotent** keyed fold (every cross-partition combiner outside the
  additive family) requires no correctness structure. Its merge-ledger record is
  re-run-tolerance bookkeeping, skipped where unrealisable; re-merging the same
  window converges. It is **not** downgraded on a structure-less backend — the
  key-addressed `MERGE` is its correct maintenance there.
- An **additive** keyed fold (`Sum`/`BitXor` anywhere in the cell) requires the
  `ReconciliationLedger`: without exact delta identities a re-merged window
  double-counts or cancels. On a backend with none it downgrades to the recompute
  family, and — carrying no `key_scope` and no `ScanClamp` — is executed by the run
  shape's own whole-target route every run, exactly as the paragraph already
  written for a downgrade-reached `PerGroupRecompute` cell specifies, and exactly
  as the succession grain's ledger-less full rebuild does.
- A cell whose grade cannot be determined requires the ledger (fail-closed).

No other spec file changes; §"Which dialects realise which structure" and
`multi_backend.md`'s matrix are unaffected (no realisation flips).

## Tests (red-green)

`smelt-logical` (`tests/maintenance_availability/resolution.rs`):
1. `idempotent_keyed_fold_is_not_downgraded_without_a_ledger` — a `KeyedFold` cell
   graded idempotent keeps its technique and records no `state_downgrade` under
   `StateAvailability::none()`.
2. `additive_keyed_fold_downgrades_to_per_group_recompute_without_a_ledger` —
   graded additive under `none()`: technique becomes `PerGroupRecompute`,
   `missing == ReconciliationLedger`, `key_scope` stays `None`, reason names the
   structure.
3. `keyed_fold_of_unknown_grade_still_requires_the_ledger` — fail-closed.

`smelt-logical` (derivation, `tests/` beside the existing plan-derivation suite):
4. `sum_fold_derives_additive_grade` / `max_fold_derives_idempotent_grade` — the
   grade reaches the cell from the fold spec's combiners.

`smelt-runtime`:
5. `plan_fold_grade_agrees_with_runtime_ledger_grade` — for a `SUM` and a `MAX`
   model, the cell's derived grade equals `WindowedKeyedRule::ledger_grade()`;
   one owner, not two.
6. `additive_keyed_fold_rebuilds_whole_target_under_warehouse_tables_none` — with
   `state.warehouse_tables: none` on DuckDB (a structure-less availability,
   offline), an additive keyed-fold model runs to completion over two windows and
   matches a `--full-refresh` oracle; the windowed-keyed driver's
   `BackendError::unsupported` is never reached.
7. `idempotent_keyed_fold_still_merges_under_warehouse_tables_none` — the same
   harness with a `MAX` fold still takes the keyed `MERGE` route (statements show
   a merge, not a rebuild) and matches the oracle.

`smelt-cli` (`tests/explain_maintenance/`):
8. `additive_keyed_fold_downgrade_is_explain_visible_on_a_structure_less_target` —
   `smelt explain --json` names the downgrade, its `original` technique and the
   missing reconciliation ledger for a Trino target, and names nothing for DuckDB.

## Tasks

1. Land the spec delta above.
2. Add the single owner of the additive-combiner predicate in
   `smelt-logical/src/rules/cumulative.rs` (`is_additive_combiner(CrossPartitionCombiner) -> bool`)
   and make `execution_postures`' two `matches!(… Sum | BitXor)` sites call it.
3. Add `FoldGrade { Idempotent, Additive }` and `PlanCell::fold_grade: Option<FoldGrade>`
   (`smelt-logical/src/maintenance/types.rs`), `None` for every non-`KeyedFold` cell.
4. Populate it in `maintenance/derive/new_data.rs`'s `Technique::KeyedFold` push,
   mapping `fold.add_columns`' `SqlFunction` combiners through the existing
   `rules::cumulative::combiner_for` and task 2's predicate.
5. Make `required_state_structure`'s `Technique::KeyedFold` arm match on
   `cell.fold_grade` per the spec delta (exhaustive; `None` ⇒ ledger).
6. Honour the downgrade at the dispatch site (`smelt-runtime/src/execute/project/mod.rs`,
   keyed arm): a resolved cell carrying a `state_downgrade` whose `original` is
   `Technique::KeyedFold` routes to the whole-target rebuild every run instead of the
   window-forward fold loop — the same shape `succession::resolve_live_succession_cell`
   already uses for its `state_downgraded` flag, reusing the existing rebuild path, not a
   new one.
7. Keep the driver's `Grade::Additive` ledger check as a fail-loud assertion, with its
   message rewritten to say the plan layer failed to downgrade first (it is now
   unreachable by construction, not the user-facing verdict).
8. Re-check the `state_guard_census` / `availability_seam` structural gates still name
   every guard correctly after 6-7.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage`
- `cargo test -p smelt-runtime --test statement_parity --test availability_seam --test state_guard_census --test execute_parity --test dry_run_statements`
- `cargo test -p smelt-cli --test maintenance_conformance --test explain_maintenance --test property_profile_parity`
- Live tier optional here (3h owns the live keyed-fold leg); if `scripts/trino-up.sh`
  is available, re-run `cargo test -p smelt-cli --test trino_incremental_families`
  for no-regression only.

## Commit message

`feat(maintenance): resolve the keyed fold's state requirement by grade at plan time`
