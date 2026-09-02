# Phase 22 plan — Time-unrolled self-edges

## Objective

Stop `build_forward_graph` refusing a whole workspace fail-loud the moment it discovers a
self-referential model. A model whose self-read is provably strictly time-backward (the
`WindowIndependence::Ordered` proof) becomes a **day-unrolled self-edge**: not a table-graph
cycle, but a time edge whose forward dirt runs to the frontier and whose backward requirement
reaches the model's own basis. Advances success criterion 15 ("time-unrolled self-edges are
built"), and unblocks phase 24's whole-workspace `examples/web_analytics` graph.

## Spec delta (comes first)

`docs/specs/incremental_models.md`:
- §"The graph layer" → **Refusals** paragraph: a self-referential model is no longer listed as
  a flat refusal. Replace with: a self-edge is admitted **iff** its self-clamp is strictly
  time-backward over the model's declared partition axis (`after == 0`, a derivable finite
  backward bound, a declared `timeseries` clock) — the same proof `Ordered` execution already
  requires. Its unrolling: dirt on `[a, b)` of the model widens **forward to the frontier**
  (`[a, →)`, since day `D`'s output feeds day `D+1`'s), and a backward requirement of `[s, e)`
  additionally requires the model's own `[s − before, s)` — one application, resolved against
  the stored basis/checkpoint, never a fixed point. A forward-reaching, unbounded, clockless or
  keyed-grain self-edge still refuses `MaintenanceGraphUnsupportedNode`, naming which.
- §Known Divergences "Graph-layer gaps": drop the "time-unrolled self-edges are designed but
  unbuilt" clause and the "a self-referential model … refuses the whole-workspace graph" half of
  the `examples/web_analytics` parenthetical (the bare-keyed-with-readers half stays for phase 24).

## Tests (red → green)

Pure layer, `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs`:
1. `self_edge_is_not_a_table_graph_cycle` — a graph with `n -> n` plus `src -> n` topo-orders
   instead of erroring; a genuine two-node cycle still errors.
2. `self_edge_widens_forward_dirt_to_the_frontier` — source dirt `[D, D+1)` on `n` yields
   `n`'s dirty interval open-ended forward (`is_open_ended`), start unchanged.
3. `self_edge_forward_dirt_reaches_downstreams_open_ended` — a reader of `n` inherits an
   open-ended interval through its own clamp (widen-never-narrow).
4. `forward_reaching_self_edge_is_refused` — `after_days > 0` on a self-edge is a fail-loud
   `Err` naming the model; likewise a `PartitionGrain::Keyed` self-edge.
5. `required_inputs_self_edge_reaches_the_basis_once` — `required[n]` for `[s, e)` widens to
   `[s − before, e)` and that widened window is what flows up `n`'s inbound source edge;
   terminates (no fixed point).

Pure clamp derivation, `crates/smelt-logical/src/analysis/window_independence.rs` unit tests:
6. `self_edge_clamp_returns_backward_reach_for_an_ordered_self_edge` — the new
   `self_edge_clamp` returns `Some(before_days)` exactly when `window_independence` returns
   `Ordered`, and `None` (with the same refusal reason available) otherwise.

Runtime, `crates/smelt-runtime/tests/since_upstream_propagation.rs`:
7. `web_analytics_self_referential_model_builds_a_self_edge` — `build_forward_graph` over the
   **unfiltered** `examples/web_analytics` model set succeeds and contains an edge
   `silver.sessions_chained -> silver.sessions_chained` with `after_days == 0` and
   `before_days >= 2`.
8. `self_referential_model_schedules_an_open_ended_run` — a delta on `sessions_chained`'s own
   upstream schedules a `PropagatedRun { start: Some(_), end: None }` for it, and the dirty-set
   report renders the self-edge line and the `[<date>, →)` form.

## Tasks

1. Write the spec delta above.
2. Add `pub fn self_edge_clamp(model_name, refs, self_partition_col, sql) -> Result<i64, String>`
   to `window_independence.rs` — additive, sharing `window_independence`'s own derivation so the
   two verdicts cannot diverge (`Ordered` ⇒ `Ok(before_days)`; every `Refused` arm ⇒ `Err(reason)`).
3. `propagate.rs`: add `DayInterval::is_open_ended()` (finite start, `end >= WHOLE.end`, not
   whole) beside `is_whole`, with the same both-bounds discipline and a unit test.
4. `propagate.rs`: add `fn self_edges(edges) -> Result<BTreeMap<&str, i64>, String>` — validates
   each `upstream == downstream` edge (day/month grain on both axes, `after_days == 0`,
   `before_days > 0`) and refuses fail-loud otherwise; used by both walkers.
5. `topo_order`: skip self-edges in in-degree accumulation and relaxation, so a self-loop is no
   longer counted as a cycle (a genuine multi-node cycle still is).
6. `propagate`: on visiting node `n` with a self-clamp, widen `result.dirty[n]`'s intervals to
   `end = WHOLE.end` **before** reading `node_dirty` / classifying outbound edges; record the
   widening under `per_edge[(n, n)]`. Self-edges are skipped by the generic outbound loop.
7. `required_inputs`: on visiting `n` in reverse topo order, apply the self-clamp once
   (`[s, e)` → `∪ [s − before, e)`) before pushing up its inbound edges; skip self-edges in the
   generic inbound loop.
8. `crates/smelt-runtime/src/propagation.rs`: replace the `bail!` at the `addr == table` site
   with `self_edge_clamp` over the same inputs `windowing.rs`'s call site uses (`model.refs`
   bare names, declared `timeseries.partition_column`, stripped SQL) — `Ok` pushes an
   `Edge { upstream: table, downstream: table, before_days, after_days: 0 }`; `Err` keeps
   today's fail-loud `MaintenanceGraphUnsupportedNode` carrying the derivation's own reason.
9. Reporting/scheduling: `render_interval` renders an open-ended interval as `[<date>, →)`; the
   run scheduler emits `start: Some(_), end: None` for it; the self-edge report line reads
   `<model> <-(self, unrolled) <model>`; fix `run.rs`'s `[--since-upstream] running …` log,
   which today prints "whole table" for any non-`(Some, Some)` pair.
10. Drop the `silver.sessions_chained` filter and its explanatory comment from
    `web_analytics_events_deduped_fully_suppressed_schedules_no_downstream_sessions`.

**Contingency (record, don't silently narrow):** if the unfiltered `web_analytics` graph then
refuses for a *different* reason, keep test 7 asserting the self-edge over a fixture narrowed by
that other model only, and note the surviving refusal in the phase summary for phase 24.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_propagation_adjoint`
- `cargo test -p smelt-logical --lib window_independence`
- `cargo test -p smelt-runtime --test since_upstream_propagation --test execute_parity`
- `cargo test -p smelt-cli --features duckdb --test since_upstream`
- `cargo test -p smelt-runtime --test self_referential_ordered_backfill` (the `Ordered`
  execution path must be unchanged by the new derivation sharing)

## Commit message

`feat(propagation): admit a backward-bounded self-reference as a day-unrolled self-edge`
