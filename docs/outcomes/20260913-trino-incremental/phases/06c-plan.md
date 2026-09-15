# Phase 6c — the repair family's own sidecar requirement

## Objective

A repair-admitted `PerGroupRecompute` cell needs `StateStructure::FingerprintSidecar`
unconditionally, but `required_state_structure` only asks for it on the key-addressed route, so on
a structure-less backend the cell is never downgraded and execution hard-refuses by name instead.
Fix the requirement in `smelt-logical`'s single-owner availability module, route the downgraded
cell to the run shape's own whole-target rebuild (3g's precedent), and re-enable phase 6's parked
`per_group_recompute_matches_full_refresh_on_trino`. Advances criteria 2 (the per-group-recompute
family executes), 3 (the unreachable route takes a named, recorded, explain-visible downgrade) and
8 (a downgraded cell is asserted oracle-equal, not exempted).

## Spec delta (first, before code)

1. `docs/specs/state.md` §"The degradation contract" step 2 — the sentence "a plain,
   clamp-bounded `PerGroupRecompute` cell requires nothing" is wrong and becomes: a
   **repair-admitted** clamp-bounded cell requires the fingerprint sidecar too, because repair
   narrowing only ever fires for a non-append-only source and `ChangeFeed` is refused upstream,
   leaving `MutableSnapshot`, whose affected-key discovery is unconditionally the group-grain
   sidecar diff. Only a **downgrade-reached** keyless, clamp-less cell requires nothing. Add: a
   repair-admitted cell downgrades to `DeleteInsert`, and the replacement **clears the cell's now
   meaningless `ScanClamp`**, so the paragraph's own "recorded downgrade, no `key_scope`, no
   derived `ScanClamp`" discriminator stays exact and the run takes the whole-target route.
2. `docs/specs/multi_backend.md` §"Incremental & schema evolution per backend", the `trino` table
   — split `| PerGroupRecompute, no key_scope | reachable | — |` into two rows: repair-admitted
   (downgraded to `DeleteInsert`, `MaintenanceStateDowngraded`) and downgrade-reached (reachable,
   executed by the whole-target route). No new diagnostic code.

## Tests (red-green, in this order)

`crates/smelt-logical/tests/maintenance_availability/`:
1. `repair_admitted_cell_requires_the_fingerprint_sidecar` — `required_state_structure` over
   `derive_repair_cell`'s output is `Some(FingerprintSidecar)` (returns `None` today: the red).
2. `repair_admission_is_only_ever_over_a_mutable_snapshot_source` — pure proof of the
   unconditionality: `faithful_fold`'s `partitioned_input` holds for `AppendOnly` (so the repair
   branch cannot fire), `discovery_posture(MutableSnapshot) == SidecarDiff`, `ChangeFeed` has no
   posture and is refused upstream.
3. `trino_invariants.rs::repair_cell_downgrades_to_delete_insert_on_trino` — after
   `resolve_availability` under Trino's availability: `technique == DeleteInsert`,
   `state_downgrade == {original: PerGroupRecompute, missing: FingerprintSidecar}`, `scans` empty,
   `has_repair_family_lowering(cell) == false`.
4. `repair_cell_is_not_downgraded_when_the_sidecar_is_available` — under `StateAvailability::all()`
   the same cell keeps `PerGroupRecompute` and its `ScanClamp` (no over-firing).
5. `a_column_scoped_merge_downgrade_keeps_its_scans` — the clamp-clearing is scoped to an
   `original == PerGroupRecompute` downgrade only, so phase 3c's route is untouched.

`crates/smelt-runtime/tests/`:
6. `availability_seam::repair_downgrade_routes_to_whole_target_rebuild` — for a structure-less
   project, `resolve_repair_state_downgrade` reports the downgrade and
   `resolve_live_per_group_recompute_cell` returns `None` for that cell (no double dispatch).
7. `repair_lowering.rs::repair_downgrade_matches_full_refresh_offline` — DuckDB with
   `state.warehouse_tables: none`: run, mutate the mutable-snapshot source in place, re-run, assert
   row-identity to a full-refresh oracle. The equivalence invariant under the new downgrade, proved
   with no live tier.

`crates/smelt-cli/tests/`:
8. `explain_maintenance/repair.rs::explain_shows_the_repair_sidecar_downgrade` —
   `smelt explain --json` renders `MaintenanceStateDowngraded` naming the fingerprint sidecar and
   the replacement technique (criterion 3's explain-visibility).
9. `trino_incremental_families/degraded_routes.rs::per_group_recompute_matches_full_refresh_on_trino`
   — re-enabled as a real live-gated `#[test]`, its `#[allow(dead_code)]` and gap doc comment
   replaced by a comment stating the downgrade it now proves.
10. `trino_incremental_spec_freshness.rs` — updated to pin the two new `multi_backend.md` rows and
    the corrected `state.md` requirement sentence.

## Tasks

1. Land the two spec edits above (spec-first).
2. `required_state_structure`'s `PerGroupRecompute` arm: require `FingerprintSidecar` when the cell
   is repair-admitted (`key_scope: None` **and** non-empty `scans` **and** no recorded downgrade),
   keeping the existing `KeyDiscovery` arms for a `key_scope`-carrying cell; document the
   discriminator against `has_repair_family_lowering`'s.
3. Express the discriminator as one named pure predicate in
   `smelt-logical::maintenance::repair` (single owner) and call it from `required_state_structure`.
4. `resolve_availability`: when the recorded `original` is `PerGroupRecompute`, clear `scans` on the
   replacement cell; leave every other downgrade's `scans` untouched.
5. Add `resolve_repair_state_downgrade` beside `resolve_keyed_fold_state_downgrade`
   (`maintenance_driver/resolve/live_cells.rs`), matching `downgrade.original == PerGroupRecompute`.
6. In `execute/project/mod.rs`, fold it into the existing whole-target-rebuild arm (the
   `keyed_fold_state_downgrade` binding, ~L1637/L1728) with `.or(...)` rather than a second arm, so
   one route serves both downgrades; widen the arm's `tracing::debug!` and comment accordingly.
7. Run tests 1-8 red → green; then re-enable test 9 and update test 10.
8. Check the fixture/diagnostic fallout on the other structure-less dialects (Spark, BigQuery) —
   expect new `MaintenanceStateDowngraded` entries; update fixtures, and record the BigQuery
   behaviour change in the decision log (escalation only, no BigQuery fix here).
9. Write `phases/06c-summary.md`, flip the row to `done`.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt + clippy both feature sets + shellcheck + full
  `cargo test` + `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage`
- `cargo test -p smelt-runtime --test availability_seam --test repair_lowering --test execute_parity --test statement_parity`
- `cargo test -p smelt-cli --test explain_maintenance --test trino_incremental_spec_freshness`
- Live tier (`bash scripts/trino-up.sh` / `source scripts/trino-env.sh`):
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1`, then
  `bash scripts/trino-down.sh`. If the coordinator is unreachable, emit `<<PHASE_BLOCKED>>` —
  never report a skipped live leg as green.

## Commit message

`feat(trino): require the fingerprint sidecar for every repair-admitted cell, so a structure-less backend downgrades instead of refusing`
