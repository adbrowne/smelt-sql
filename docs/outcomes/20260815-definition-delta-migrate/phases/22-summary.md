# Phase 22 summary — Time-unrolled self-edges

## Shipped

- `docs/specs/incremental_models.md` §"The graph layer": new "Time-unrolled self-edges"
  paragraph replaces the flat self-referential refusal; §Known Divergences' "time-unrolled
  self-edges are designed but unbuilt" and the self-referential half of the
  `examples/web_analytics` parenthetical are dropped.
- `crates/smelt-logical/src/analysis/window_independence.rs`: new `pub fn self_edge_clamp` —
  shares a new private `self_edge_bound_days` with `window_independence` so the two verdicts
  cannot diverge.
- `crates/smelt-logical/src/maintenance/propagate.rs`: `DayInterval::is_open_ended`; new private
  `self_edges(edges)` validating every self-edge (day/month grain both sides, `after_days == 0`,
  `before_days > 0`) and returning its backward reach; `topo_order` skips self-edges (no longer a
  cycle); `propagate` widens a self-clamped node's own dirt to the frontier before classifying
  outbound edges, recording it under `per_edge[(n, n)]`; `required_inputs` applies the self-clamp
  once when pushing a node's requirement up its inbound edges.
- `crates/smelt-runtime/src/propagation.rs`: the `addr == table` self-reference site now calls
  `self_edge_clamp` (same inputs `windowing.rs` uses) instead of an unconditional `bail!`; `Ok`
  registers a `(table, table)` clamp entry, `Err` keeps the fail-loud refusal. `render_interval`
  renders `[<date>, →)` for an open-ended interval; the self-edge dirty-set line reads
  `<model> <-(self, unrolled) <model>`; the run scheduler emits `start: Some(_), end: None` for
  an open-ended dirty interval.
- `crates/smelt-cli/src/commands/run.rs`: the `[--since-upstream] running …` log now distinguishes
  `(Some, None)` (`[s, →)`) from `(None, None)` (`whole table`) instead of collapsing both to
  "whole table".
- `examples/web_analytics`'s `silver.sessions_chained` self-edge is now exercised in the
  **unfiltered** whole-workspace graph (the test-local filter and its explanatory comment are
  removed from `web_analytics_events_deduped_fully_suppressed_schedules_no_downstream_sessions`).

## Decisions

- A same-partition self-read (`before_days == 0`, no backward margin at all) is **not**
  admitted — only `before_days > 0` counts as "strictly time-backward". `self_edges()` refuses
  it fail-loud. This is stricter than `window_independence`'s pre-existing `Ordered` check
  (which only tests `after == 0`, not `before > 0`) — a pre-existing gap in that function,
  unrelated to the graph layer, left untouched (see below).
- Refusal for an inadmissible self-edge now happens in two places with different scope:
  `smelt_runtime::propagation::build_forward_graph` still refuses immediately when
  `self_edge_clamp` itself returns `Err` (no declared clock, forward reach, unbounded/undivable
  bound); a *same-partition* (`before_days == 0`) self-edge is `Ok` from `self_edge_clamp` (it
  has after==0) but is now caught downstream by `propagate`/`required_inputs`'s shared
  `self_edges()` gate instead. Both paths still surface `MaintenanceGraphUnsupportedNode` to the
  end user — `smelt run --since-upstream` always calls through to `propagate`.
- Widening/backward application happens exactly once per node visit (topo walk visits each node
  once) — no fixed point, matching the spec's "one application" wording.

## For the next planner

- **Pre-existing gap surfaced, not fixed here**: `window_independence`'s `Ordered` verdict does
  not check `before > 0` — a same-partition self-read (`before == 0, after == 0`) is classified
  `Ordered`, which would let the ordered-backfill batching path (`windowing.rs`) silently force
  single-partition execution on a model that can never converge, rather than refusing. The graph
  layer independently refuses this case (`self_edges()`), but the ordered-backfill execution path
  does not share that extra check. Worth a follow-up phase tightening
  `self_edge_bound_days`/`window_independence` itself (would need auditing
  `self_referential_ordered_backfill` and any other consumer for behavior change).
  Not in this phase's spec delta or test list, so left alone.
- **Real end-to-end run of an open-ended `PropagatedRun` is untested against `execute_project`**:
  `ExecuteRequest::start`/`end` still enforce "both or neither" in
  `crates/smelt-runtime/src/execute.rs::parse_run_window`. A `(Some(start), None)` propagated run
  (as `smelt run --since-upstream` would now schedule for a dirtied self-referential model) would
  fail that guard today. Phase 24 (the full `examples/web_analytics` `--since-upstream`
  end-to-end criterion) should wire this — either `parse_run_window` learns an open-ended form, or
  the CLI resolves `end: None` to "today"/the latest available partition before calling
  `execute_project`. This phase's own test list only exercised the propagation-layer scheduling
  (`plan_since_upstream`), not a live `execute_project` run over an open-ended window.
- Renamed the pre-existing `self_referential_model_refuses` test (smelt-runtime) to
  `same_partition_self_referential_model_refuses` and `self_edge_is_refused_as_a_cycle`
  (smelt-logical) to `backward_bounded_self_edge_is_admitted_as_a_time_unrolled_edge` +
  `same_partition_self_edge_is_still_refused`, since the blanket self-edge refusal they pinned no
  longer holds — the refusal point moved from `build_forward_graph` to
  `propagate`/`required_inputs`'s shared `self_edges()` gate.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_propagation_adjoint` — 29 passed.
- `cargo test -p smelt-logical --lib window_independence` — 8 passed.
- `cargo test -p smelt-logical --test maintenance_tracer_propagation` — 28 passed.
- `cargo test -p smelt-runtime --test since_upstream_propagation --test execute_parity` — 24 + 4
  passed.
- `cargo test -p smelt-cli --features duckdb --test since_upstream` — 13 passed.
- `cargo test -p smelt-runtime --test self_referential_ordered_backfill` — 1 passed (unchanged by
  the derivation sharing).
