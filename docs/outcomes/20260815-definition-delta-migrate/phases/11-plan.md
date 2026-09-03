# Phase 11 — Dispatch the key-addressed model-edge cell for a `grain: partition` downstream

## Objective

A clockless `KeyedUpsert` upstream feeding a `grain: partition` downstream already derives a
key-addressed `Technique::PerGroupRecompute` cell (`append_model_edge_cells` skips the clock
route when `edge.clock_col` is `None`, regardless of the downstream's `output_partition_col`),
but `smelt-runtime`'s run loop resolves and dispatches that cell only inside the
`plan_is_keyed` branch of `execute.rs`, so a partition-grain downstream silently falls back to
its ordinary window-forward batch loop. Wire dispatch on the non-keyed incremental branch too,
and narrow the spec bullet that records the gap. Advances success criterion 10 (and
criterion 9's gate sweep).

## Spec delta

`docs/specs/incremental_models.md`:
1. §"Upstream model edges" (~L1350, the "key-addressed edge" paragraph): add one sentence
   stating the key-addressed route is dispatched irrespective of the downstream's own grain —
   a `grain: partition` downstream takes it in place of its window-forward batch loop for that
   run, because the cell's bounded read is the affected key set and has no partition-interval
   axis to compose with a run window.
2. §Known Divergences "The scheduler does not yet consume delta signatures end to end"
   (L1813–1823): delete the clockless-`keyed upsert`→`grain: partition` clause (the run loop
   now dispatches it). Keep the two remaining clauses verbatim — keyed dirt-sets carry key
   columns/provenance not values, and cross-model runs need the operator to state the landed
   upstream window. Reword the bullet's lead so it still reads coherently with that clause gone.

## Tests

Red-green, all in `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs` (extend the
existing `chain` module rather than a new file — it already stages a real DuckDB two-model
project through `execute_project`):

1. `partition_grain_downstream_resolves_the_key_addressed_cell` — unit leg: a `grain: partition`
   + `timeseries:` downstream reading a clockless `KeyedUpsert` upstream resolves `Some(..)`
   from `resolve_live_key_addressed_model_edge_cell` (guards that derivation, which already
   works, stays reachable from the non-keyed inputs).
2. `partition_grain_chain_maintains_only_the_changed_keys_end_to_end` — real-DuckDB leg mirroring
   `keyed_chain_maintains_only_the_changed_keys_end_to_end`, with `downstream` declared
   `grain: partition` (+ its own `timeseries:`): after mutating one upstream key's contribution,
   run 2's `ModelRecord::strategy` is `per_group_recompute` (RED today: it is the ordinary
   window-forward/delete-insert strategy), the mutated key's value is repaired, the untouched
   key's row is bit-identical, and the repaired value equals a full-refresh oracle over current
   source state.
3. `partition_grain_creation_run_does_not_take_the_key_addressed_route` — the first (table-absent)
   run of the same project still materializes via the ordinary fold path, proving the
   `table_exists_before_run` guard is honoured on this branch as it is on the keyed one.

## Tasks

1. Write test 2 first and confirm it fails on the strategy assertion (RED).
2. In `execute.rs`'s non-keyed incremental branch (the `Some(inc_plan)` arm at ~L2718), reuse the
   already-built `sql_for_bounds`, `maint_source_facts`, `explicitly_mutable` and `model_edges`
   to call `resolve_live_key_addressed_model_edge_cell` (same argument shape as the keyed branch
   at ~L1977), capturing `table_exists_before_run` **before** any write this run performs.
3. When the cell resolves live and the table already exists, short-circuit the batch loop and
   dispatch `maintenance_driver::execute_key_addressed_model_edge_cell` with the same upstream
   target/table/source-address resolution the keyed branch performs; record the run's strategy as
   `per_group_recompute` (or `diff_patch`, per the cell's `RepairWrite`) through the same
   `used_per_group_recompute`/`used_diff_patch` plumbing the keyed branch uses.
4. Factor the duplicated resolve+dispatch body out of both branches into one helper in
   `execute.rs` (or `maintenance_driver.rs`) rather than copying it — a second hand-written copy
   is exactly how the two branches diverged in the first place.
5. Confirm mutual exclusion with the branch's existing `column_scoped_cell` /
   `delta_restriction_facts` dispatch: a key-addressed cell is keyed on the upstream *model*'s
   bare name, the others on declared sources, so they cannot contend — assert this in a comment
   with the same reasoning the keyed branch already records, and bail loudly if both resolve for
   the same trigger name rather than silently preferring one.
6. Apply the spec delta above.
7. Run the verification gates.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test typed_edge_graph`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb`
- `cargo test -p smelt-cli --test e2e --features duckdb`
- `cargo test -p smelt-logical --test walk_coverage`

## Commit message

`feat(runtime): dispatch the key-addressed model-edge repair cell on the partition-grain run branch`
