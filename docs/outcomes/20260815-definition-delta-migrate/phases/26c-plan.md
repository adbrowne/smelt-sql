# Phase 26c — propagation intervals at the declared granularity

## Objective

Close the hour-granularity clause of success criterion 16: the graph layer's intervals stop
being day-ordinal and carry the full declared `timeseries.granularity` surface
(`hour`…`year`), so an `hour`-grain node propagates at its own axis instead of bailing with
`MaintenanceGraphUnsupportedNode`. The same change closes the "edge grains come from the
caller" residue: an `Edge` can no longer be built without both endpoints' declared grains.

## Spec delta (first)

- `docs/specs/incremental_models.md` §"The graph layer" → **Edges**: state that intervals are
  half-open instants on the node's declared axis, that clamp margins are exact and it is the
  receiving axis's outward alignment (not a day ceiling) that widens to whole partitions, and
  that every `Granularity` variant has a graph axis. Add one sentence recording the
  runtime-surface boundary: a propagated interval rendered into a run window still aligns
  outward to whole days, because `smelt run`'s window surface is date-valued.
- §Known Divergences, "Locality and diagnostic residues" bullet: delete the
  "hour granularity is declared surface but propagation is day-ordinal" clause and the
  "graph edges still take the declaration directly" clause. The latter is not a defect —
  §"Granularity is declared, not derived" is the normative posture, and after this phase the
  constructor enforces it rather than defaulting. Keep the write-footprint clause (26a) and the
  column-group-dirt clause (26d) until their phases land.
- Sweep `docs/specs/model_properties.md` §Known Divergences where it names the same
  day-ordinal residue.

## Tests (red-green)

`crates/smelt-logical/src/maintenance/propagate.rs` (inline module tests):
1. `hour_grain_aligns_outward_to_the_hour` — a sub-hour delta widens to exactly one hour, not
   one day.
2. `hour_edge_chain_does_not_coarsen_to_days` — a 2h clamp over an hour-grain chain dirties 
   hours, strictly narrower than today's whole-day result.
3. `week_grain_aligns_to_the_declared_week_start` — same delta, `start_dow` Monday vs Sunday,
   different aligned boundaries.
4. `quarter_and_year_align_to_civil_boundaries`.
5. `clamp_margins_are_exact_seconds_not_ceiled_days` — a 36h clamp on an hour axis widens by
   36h; on a day axis the same clamp still widens to 2 whole days via alignment.
6. `whole_axis_sentinel_survives_alignment_at_every_grain` — `WHOLE` in, `WHOLE` out, no
   civil-math overflow.
7. `mixed_grain_hop_widens_to_the_coarser_receiving_axis` — hour → month hop.
8. `day_grain_graphs_are_unchanged` — regression anchor: an existing day-only scenario's
   `Propagation` is identical to today's.

`crates/smelt-logical/tests/maintenance_propagation_adjoint.rs` (extend):
9. `adjointness_holds_at_hour_grain` — `forward(backward(P)) ⊇ P` on an hour-grain chain.

`crates/smelt-runtime/tests/since_upstream_propagation.rs` (extend):
10. `hour_granularity_model_is_scheduled_not_refused` — a workspace node declaring
    `granularity: hour` plans instead of erroring `MaintenanceGraphUnsupportedNode`.
11. `sub_day_dirt_renders_a_day_aligned_run_window` — the emitted run window is date-valued and
    covers the sub-day interval (widen, never narrow).

## Tasks

1. Write the spec delta above.
2. In `propagate.rs`, rename `DayInterval` → `PartitionInterval` and reinterpret `start`/`end`
   as **seconds since the civil epoch**; keep `day_ordinal`/`civil_from_ordinal` as the civil
   helpers and add `day_start(y,m,d) -> i64` plus `PartitionInterval::whole_day(ordinal)` so
   existing tests migrate mechanically. `WHOLE` stays the saturating sentinel.
3. Extend `PartitionGrain` to `Hour | Day | Week { start_dow: u8 } | Month | Quarter | Year |
   Unclocked | Keyed`; implement `align_outward` for each in civil math, short-circuiting the
   `WHOLE` sentinel before any arithmetic.
4. Replace `Edge::before_days`/`after_days` with exact `before_seconds`/`after_seconds`
   (from `ScanClamp`'s `Seconds` directly) and delete `clamp_days` — outward alignment at the
   receiving axis now performs the widening the day-ceiling used to. Same for
   `locality_margin_days` and `project_observed_delta`'s margins (rename accordingly).
5. Make `Edge::from_clamp(downstream, clamp, upstream_grain, downstream_grain)` require both
   declared grains; delete the "override the grain fields" affordance and the doc sentence that
   documents caller-declared grains as a boundary.
6. `crates/smelt-runtime/src/propagation.rs`: make `granularity_grain` total over
   `Granularity` (no `bail!`), sourcing `Week`'s `start_dow` from `TimeseriesConfig.week_start`
   (default Monday); pass exact seconds and both grains at every `Edge` construction; keep ISO
   rendering date-valued by aligning outward to whole days at the rendering seam only
   (`ordinal_to_iso` call sites), with a doc comment naming that seam.
7. Migrate the remaining `DayInterval` users (`smelt-maintenance-testkit`, the
   `maintenance_conformance` dags, the runtime/logical propagation tests) mechanically via the
   `whole_day` helper.
8. Update the module doc's "Known boundaries" block: the day-ordinal boundary is gone; the
   whole-partition-dirt boundary stays (26d).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance::propagate --test maintenance_propagation_adjoint --test maintenance_tracer_propagation`
- `cargo test -p smelt-runtime --test since_upstream_propagation --test tracer_propagation --test typed_edge_graph`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-cli --features duckdb --test e2e since_upstream`
- `cargo test --workspace` (phase 25's summary: a cross-cutting type change breaks tests outside
  the phase's own file list — sweep before declaring green)

## Commit message

`feat(maintenance): propagate dirt at each node's declared granularity`
