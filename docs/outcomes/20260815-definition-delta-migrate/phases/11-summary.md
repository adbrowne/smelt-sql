# Phase 11 summary — dispatch the key-addressed model-edge cell on the partition-grain branch

## Shipped

- `crates/smelt-runtime/src/execute.rs`: new shared helper
  `resolve_and_dispatch_key_addressed_edge_cell` (+ `KeyAddressedEdgeDispatch`) factors the
  resolve-then-execute body for a live key-addressed model-edge repair cell out of the keyed
  run branch, and wires the SAME call into the non-keyed (window-forward) incremental branch's
  `Some(inc_plan)` arm — resolved and, if live, dispatched before that branch's self-ref
  bootstrap or per-batch DELETE+INSERT loop (both wrapped in a `'run_dispatch_or_batches:`
  labeled block so the dispatch case can `break` past them without touching their
  indentation). `table_exists_before_run` is captured before either.
- A fail-loud mutual-exclusion check: if a key-addressed cell and a `column_scoped_cell` ever
  resolved for the same trigger name, the non-keyed branch now bails by name instead of
  silently preferring one.
- Tests (`crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs`): unit leg
  `partition_grain_downstream_resolves_the_key_addressed_cell` (resolution stays reachable for
  a `grain: partition` downstream), and two real-DuckDB chain legs —
  `partition_grain_chain_maintains_only_the_changed_keys_end_to_end` (RED before the fix: the
  run recorded `deleteinsert`, not `per_group_recompute`) and
  `partition_grain_creation_run_does_not_take_the_key_addressed_route`.
- `docs/specs/incremental_models.md`: §"Upstream model edges" states the cell dispatches
  irrespective of the downstream's own grain; the Known Divergences "scheduler does not
  consume delta signatures end to end" bullet no longer names the clockless-`keyed upsert`→
  `grain: partition` gap (the other two clauses — key-value-level dirt-sets, cross-model
  watermarks — are untouched and still open).

## Decisions

- The `grain: partition` downstream test fixtures declare NO top-level `unique_key:` —
  declaring one alongside `timeseries:` (with the partition column outside the key) derives
  `grain: key` from the shape facts and fails the `grain: partition` assertion before reaching
  the maintenance layer. `admit_key_addressed_recompute`'s row-identity proof falls back to the
  SQL's own `GROUP BY user_id` when `declared_unique_key` is empty, so the cell still resolves.
- The two new chain-project tests run `agg` (unwindowed, `grain: key`) and `downstream`
  (windowed, `grain: partition`) as SEPARATE `execute_project` calls per run: a single
  `ExecuteRequest`'s `start`/`end` applies uniformly to every selected model, and giving `agg`
  a window registers a reconciliation-ledger entry for that window that the SAME window on run
  2 then refuses re-folding (`KeyedReprocessedWindow`, never-fold-twice).

## For the next planner

- Phase 12 (per-cell frontier addressing / `diff_patch` over the region `DeleteInsert`
  default) and phase 13 (write-pin equivalence) are the natural next steps in this same
  run-loop area — nothing new surfaced here that widens their scope.
- Not investigated: whether the same narrowing applies to any OTHER cell family gated on
  `plan_is_keyed` alone (only the key-addressed model-edge cell was in scope for this phase).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature
  sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering` — 9 passed.
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test typed_edge_graph` — 4 + 23 + 5 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 74 passed.
- `cargo test -p smelt-cli --test e2e --features duckdb` — 175 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
