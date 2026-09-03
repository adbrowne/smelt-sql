# Phase 26d summary — column-group-scoped dirt

**Shipped:**
- `grouping.rs`: `GroupingResult::value_only_sources` — sources whose enrichment `ON` read was
  proven closure-pruned (`closure_pruned_source`), surfaced (previously computed but only used
  to skip a membership-sensitivity attachment). New pure `dirt_scope(upstream, &GroupingResult)
  -> Option<Vec<String>>` implementing the spec's three-condition admission.
- `propagate.rs`: `Edge::dirtied_groups: Option<Vec<String>>` (+ `with_dirtied_groups`);
  `Propagation::{dirty_groups, per_edge_groups}`. Forward walk gates each outbound typed edge
  against the node's own merged scope (skip when non-empty components name only groups outside
  the scope) and merges/contaminates the working accumulators (`merge_edge_groups`) — an
  unscoped inbound edge permanently widens the node to whole-model, never revocable by a later
  scoped one.
- `smelt-runtime/src/propagation.rs`: `consumer_grouping_result` caches each downstream's raw
  `GroupingResult`; `build_forward_graph` sets `Edge.dirtied_groups` via `dirt_scope`. The
  `--since-upstream` dirty-set report renders `[groups: {g}, …]` on a narrowed per-edge line.
- `docs/specs/incremental_models.md` §"The graph layer": new "Column-group-scoped dirt"
  subsection. Known Divergences: removed the "column-group-scoped dirt coarsens to
  whole-partition" bullet entirely (both its clauses were closed — the sibling grain-alignment
  clause was posture per 26c's own call, not a live gap).

**Decisions:**
- Fixed a real plumbing gap this phase's own end-to-end test exposed: `output_delta::SourceFacts`
  never carried a source's declared `unique_key` (its `to_maintenance_source_facts` conversion
  read `delta_identity` instead — a change-feed-only fact, always empty for a `mutable_snapshot`
  enrichment source). This made closure-pruning structurally unreachable from the real
  per-workspace graph (`build_forward_graph`) even though the underlying `grouping.rs` proof and
  `smelt-db`'s own plan-layer `source_facts` adapter were both already correct. Added
  `SourceFacts::unique_key`, populated from `SourceInfo::unique_key` in `from_source_info`, and
  fixed the conversion. This is a strict widening (more real closures provable, never fewer) —
  full regression sweep (`cargo test --workspace`) is green.
- `dirt_scope`'s "every group sensitive" case is unreachable through real SQL for a genuinely
  closure-pruned source (the closure proof's own conjunct 2 forbids a column blending the
  pruned source with another source's provenance in the same group, and non-blended columns
  reading only that source share one sensitivity set and merge into one group) — that test is
  built directly against a synthetic `GroupingResult` rather than through `derive_column_groups`.

**For the next planner:**
- The `output_delta::SourceFacts::unique_key` fix is scoped exactly to what 26d's own dirt_scope
  wiring needed; no other consumer of `output_delta::SourceFacts` was audited beyond the full
  test sweep passing. Worth a note if a future phase finds another silent gap in that struct.
- Column-group scoping is applied uniformly to both the interval (`NotKeyed`) and `Admitted`
  (keyed) branches of `propagate`'s forward walk, though no test in this phase's list exercised
  the keyed+group interaction directly — the existing keyed-channel tests all pass unchanged.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  `cargo test` workspace, example_diagnostics).
- `cargo test -p smelt-logical --lib maintenance::grouping --lib maintenance::propagate --test maintenance_propagation_adjoint --test maintenance_tracer_propagation` — pass.
- `cargo test -p smelt-runtime --test since_upstream_propagation --test typed_edge_graph --test tracer_propagation` — pass (38+6+5).
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed.
- `cargo test -p smelt-cli --features duckdb --test e2e since_upstream` — pass.
- `cargo test --workspace` — full sweep green (exit 0, no FAILED entries).
