# Phase 21 plan — Keyed dirt cascades and is consumed

## Objective

Make the keyed dirt-set channel a live currency of the propagation graph rather than a
representation nothing reads. Today `smelt_logical::maintenance::propagate` builds
`KeyedDirt` into `Propagation::{keyed_dirty, per_edge_keys}`, but a node dirtied *only*
through that channel gets no `dirty` entry — so `propagate`'s own walk never visits its
outbound edges (one-hop dead end) and `smelt_runtime::propagation::plan_since_upstream`,
which reads `prop.dirty` alone, neither schedules nor reports it. This closes the first two
clauses of success criterion 15's graph half: a key-level dirt representation that the graph
layer actually propagates, and a bare `grain: key` model with readers getting past
`MaintenanceGraphUnsupportedNode` end to end.

## Spec delta

`docs/specs/incremental_models.md` §"The graph layer" → "Keyed dirt-sets and the narrowed
refusal": add one paragraph stating what a keyed dirt-set *does* once seeded — it cascades
(a node carrying only keyed dirt is still a dirty node whose own outbound edges are walked,
its outbound contribution derived from its own grain: keyed downstream ⇒ keyed dirt, clocked
or unclocked downstream ⇒ whole-table interval dirt, per widen-never-narrow), and it is
reported and scheduled by `--since-upstream` as a whole-table run on the keyed node with the
affected key set named in the dirty-set report. Do **not** restate the refusal rule; it is
unchanged. No CLI-flag surface change.

## Tests

Red-green, in this order:

1. `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs::keyed_dirt_cascades_past_one_hop`
   — source → keyed A → keyed B: `keyed_dirty` names B, not only A.
2. `…::keyed_only_node_widens_to_whole_table_for_a_clocked_reader`
   — source → keyed A → clocked C: C carries `DayInterval::WHOLE` even though A itself had
   no interval dirt (only keyed dirt) seeded.
3. `…::keyed_cascade_terminates_on_a_cycle_free_graph` — a diamond over keyed nodes visits
   each node once and produces deduplicated `keyed_dirty` entries (no exponential fan-out).
4. `crates/smelt-runtime/tests/since_upstream_propagation.rs::bare_keyed_model_with_readers_is_scheduled`
   — a bare `grain: key` model (no `timeseries:`, admitted `Addressing::Keyed` shape) with a
   downstream reader yields a `PropagatedRun` for both, in dependency order, and no
   `MaintenanceGraphUnsupportedNode` error.
5. `…::keyed_dirt_appears_in_the_dirty_set_report` — the rendered report names the keyed node
   with its key columns and upstream, distinguishably from an interval line.
6. `crates/smelt-cli/tests/since_upstream.rs::since_upstream_over_a_bare_keyed_chain_runs_end_to_end`
   — CLI leg over a staged fixture: exit 0, both models rebuilt, report printed.

## Tasks

1. Write tests 1–3 red against `propagate`.
2. In `propagate`, make the node walk driven by "node has interval dirt **or** keyed dirt",
   seeding keyed-only nodes so their outbound edges are classified and reflected exactly as
   today's `KeyedAdmission` match already does; dedupe `keyed_dirty`/`per_edge_keys` pushes.
3. Keep the existing `dirty.retain(non-empty)` prune but do not drop a keyed-only node from
   the propagation result — add the equivalent prune for `keyed_dirty`.
4. Write tests 4–5 red against `plan_since_upstream` / `plan_since_upstream_with_observed_deltas`.
5. Consume `prop.keyed_dirty` in the runtime plan builder: a keyed-dirty node becomes a
   `PropagatedRun` with `start`/`end` `None` (whole-table, the keyed channel has no interval
   axis), ordered by the same dependency order the interval path uses, deduplicated against a
   node that also carries interval dirt (one run, not two).
6. Extend `render`/`dirty_set_report` with a keyed line form naming key columns + upstream.
7. Make the spec edit named above.
8. Add the CLI fixture and test 6 (reuse the existing `since_upstream.rs` staging helpers;
   a synthetic bare-keyed workspace, not `examples/web_analytics` — that is phase 24).
9. Confirm `refuse_bare_keyed_origins` still refuses the `General`-shape case (existing tests
   must stay green unmodified; if one needs changing, say why in the summary).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_propagation_adjoint`
- `cargo test -p smelt-runtime --test since_upstream_propagation --test execute_parity`
- `cargo test -p smelt-cli --features duckdb --test since_upstream`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance`

## Commit message

`feat(propagation): cascade and consume keyed dirt-sets so a bare grain: key node propagates end to end`
