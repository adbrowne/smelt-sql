# Phase 3c — a downgrade-derived `PerGroupRecompute` cell needs no `ScanClamp`

## Objective

A `Technique::ColumnScopedMerge` cell on a fully-degraded dialect downgrades to
`PerGroupRecompute` (`resolve_availability`), and `resolve_live_per_group_recompute_cell` then
refuses the whole run with `MaintenanceRepairSliceMissing` because the cell carries no derived
`ScanClamp` — which it never could, its trigger being `UpstreamMutation` rather than a clock. This
phase makes that cell decline the repair-family lowering instead, leaving the run shape's own
whole-target route to perform the full-scan recompute `required_state_structure`'s `key_scope:
None ⇒ None` row already promises. It is gap 3 of phase 3's block report, and it blocks criterion
2 (the reachable families execute end-to-end) and criterion 7 (the generative gate drives the real
pipeline) for every snapshot-reconcile-shaped keyed model on Trino — and, per the block report, on
Spark for the same reason.

## Spec delta

`docs/specs/state.md` §"The degradation contract" (step 2, after the existing
`EnrichmentKeyed`/`SuccessionPatch` fallback paragraphs). Add: a `PerGroupRecompute` cell reached
**by downgrade** never passed the repair family's own admission obligations, so it carries neither
a derived bounded slice nor a `key_scope`, and has **no repair-family lowering**; its recompute is
performed by the run shape's own whole-target route, and a run must not refuse it for the absent
slice. State the discriminator explicitly (a recorded `state_downgrade`, no `key_scope`, no derived
`ScanClamp`) and state that a clamp-less cell the repair family *did* admit remains a fail-loud
internal inconsistency, so the relaxation cannot widen into a silent unbounded scan. Cross-reference
`incremental_models.md` §"The repair family" from the new paragraph; no edit is needed there.

## Tests

Red-green, in this order:

1. `crates/smelt-logical/src/maintenance/repair.rs` (unit, `--lib`)::
   `downgraded_clampless_cell_has_no_repair_family_lowering` — the new pure predicate returns
   `false` for a `PlanCell { technique: PerGroupRecompute, state_downgrade: Some(..),
   key_scope: None, scans: vec![] }`. Red: the predicate does not exist.
2. `…::repair_admitted_cell_has_repair_family_lowering` — `true` for a clamp-bearing,
   non-downgraded `PerGroupRecompute` cell, and `true` for a `key_scope`-carrying key-addressed
   cell, so the predicate narrows nothing that works today.
3. `crates/smelt-runtime/tests/repair_lowering.rs::
   resolve_live_per_group_recompute_cell_declines_a_downgraded_clampless_cell` — drive a keyed
   model over a `mutable_snapshot` source with `allow_full_scan` through the resolver under an
   **empty** `StateAvailability`; assert `Ok(None)`, not the `MaintenanceRepairSliceMissing` error.
   Red today.
4. `…::resolve_live_per_group_recompute_cell_still_fails_loud_on_a_missing_clamp` — an admitted
   (non-downgraded) `PerGroupRecompute` cell whose `scans` are empty still `bail!`s with
   `MaintenanceRepairSliceMissing`. Green before and after; the fence that keeps the relaxation
   narrow.
5. `crates/smelt-runtime/tests/repair_lowering.rs` (or the nearest DuckDB-backed harness)::
   `downgraded_keyed_model_recomputes_full_scan_and_matches_a_full_refresh` — run the same model
   through `execute_project` against DuckDB with every state structure unavailable, twice (create
   then a mutated re-run), and assert the target's rows equal a full-refresh rebuild of the same
   SQL. This is criterion 8 in miniature: the downgraded cell is compared to the oracle, not
   exempted. Runs per-PR, no live tier.
6. `crates/smelt-cli/tests/trino_incremental_families.rs::
   snapshot_reconcile_keyed_model_runs_on_trino` — replaces the `gap_3_…` documentation anchor
   with the live leg: the same model shape run twice through `smelt run --target trino`, rows
   asserted via `fetch_trino_rows`. Follows the file's existing `trino_env()` skip-with-message
   shape; the phase emits `<<PHASE_BLOCKED>>` rather than reporting green if the coordinator is
   unreachable for the whole run.

## Tasks

1. Add the pure predicate to `smelt-logical`'s maintenance layer — `maintenance::repair::
   has_repair_family_lowering(cell: &PlanCell) -> bool` (name at implementer's discretion) — with a
   doc comment carrying the spec cross-reference. It is plan-shaped data derived in
   `smelt-logical`, not a runtime judgement, per maintenance-plan purity.
2. Land tests 1-2 against it.
3. In `crates/smelt-runtime/src/maintenance_driver/repair/resolve_cell.rs`, consult the predicate in
   the per-cell loop **before** the `repair_cell_key` / clamp / discovery-posture ladder, and
   `continue` to the next cell when it is `false` — a `continue`, not a `return Ok(None)`, so a
   sibling genuinely-admitted cell on another source still resolves.
4. Land tests 3-4; confirm the `MaintenanceRepairSliceMissing` bail text and its doc comment now
   describe only the admitted case.
5. Land test 5; if the whole-target route does not in fact dispatch for this shape once the
   resolver stops refusing, wire it and say so in the summary — do not narrow the test.
6. Make the §"The degradation contract" spec edit.
7. Replace the `gap_3_…` anchor in `trino_incremental_families.rs` with test 6, and update that
   file's header doc comment (gaps 2 and 3 landed; gap 1 still open, now owned by row 3b2).
8. Bring the tier up (`bash scripts/trino-up.sh`, `source scripts/trino-env.sh`), run test 6, tear
   down (`bash scripts/trino-down.sh`).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance`
- `cargo test -p smelt-runtime --test repair_lowering`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — the relaxation
  must not change which statements any already-working family emits.
- `cargo test -p smelt-cli --test maintenance_conformance` — the equivalence gate on DuckDB, over
  a change to the downgrade route it exercises.
- `cargo test -p smelt-cli --test trino_incremental_families` with `SMELT_TRINO_URL` set.

## Commit message

`fix(maintenance): a downgrade-derived PerGroupRecompute cell declines the repair lowering instead of demanding an impossible ScanClamp`
