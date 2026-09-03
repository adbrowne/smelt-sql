# Phase 26c summary — propagation intervals at the declared granularity

## Shipped

- `crates/smelt-logical/src/maintenance/propagate.rs`: `DayInterval` → `PartitionInterval`,
  bounds are now exact seconds since the civil epoch (`day_start`/`DAY_SECONDS` new pub helpers).
  `PartitionGrain` extended to `Hour | Day | Week { start_dow: chrono::Weekday } | Month |
  Quarter | Year | Unclocked | Keyed`, each with a real civil `align_outward`
  (short-circuiting the `WHOLE` sentinel before any arithmetic).
  `Edge::before_days/after_days/footprint_days` → `before_seconds/after_seconds/footprint_seconds`,
  carrying the clamp's exact margin (no more pre-ceiling in `Edge::from_clamp`/`clamp_days`,
  which is deleted). `locality_margin_days` → `locality_margin_seconds` (also exact, no
  ceiling). `project_observed_delta` migrated to exact-seconds arithmetic.
- `crates/smelt-runtime/src/propagation.rs`: `granularity_grain` is now total over every
  `Granularity` variant (was `Day`/`Month`-only, `bail!` otherwise); a new `chrono_weekday`
  conversion bridges `smelt_core::config::Weekday` (the declared `week_start` surface) to
  `chrono::Weekday` (`PartitionGrain::Week`'s own type). `parse_landed_range` and every
  `PropagatedRun`/dirty-set-report rendering site route through two new private helpers,
  `iso_floor`/`iso_ceil`, which align a seconds interval outward to whole days only at the
  CLI-facing rendering seam.
- Spec delta: `docs/specs/incremental_models.md` §"The graph layer" → Edges paragraph rewritten
  (exact seconds, every granularity has a graph axis, receiving-axis outward alignment, the
  runtime rendering-seam sentence); §Known Divergences' day-ordinal/declared-vs-propagated
  clause removed.
- 8 new unit tests in `propagate.rs` (`grain_alignment_tests` module) plus 2 new integration
  tests in `smelt-runtime/tests/since_upstream_propagation.rs`
  (`hour_granularity_model_is_scheduled_not_refused`,
  `sub_day_dirt_renders_a_day_aligned_run_window`) — all from the plan's test list.

## Decisions

- Kept the type/field renames literal (`DayInterval`→`PartitionInterval`,
  `before_days`→`before_seconds`, etc.) rather than a compatibility alias, per the plan's own
  "existing tests migrate mechanically" framing — a `pub type DayInterval = PartitionInterval`
  shim was tried and discarded (CLAUDE.md discourages back-compat shims where a clean rename
  is available).
- Every pre-existing test that constructed a `PartitionInterval`/`Edge` from small bare
  integers (day-ordinal fixtures under the old architecture) now scales by `DAY_SECONDS`
  (or `day_start(ordinal)`) at the fixture boundary — the test bodies and their assertions are
  otherwise untouched, since Day-grain `align_outward` is a no-op on day-aligned seconds.
- `self_edge_clamp` (in `smelt-logical::analysis::window_independence`) still returns whole
  days (unchanged, out of this phase's scope) — its one call site in
  `smelt-runtime::propagation::derive_clamp_and_locality_pass` now explicitly scales by
  `DAY_SECONDS` before folding into the seconds-based `clamp_seconds` map. This was the one
  real (not test-fixture) bug this phase's own migration introduced and caught via
  `since_upstream_propagation.rs`'s `web_analytics_self_referential_model_builds_a_self_edge`.

## For the next planner

- **26d (finer-than-partition column-group dirt)** is next in the table and unaffected by this
  phase's mechanics — it operates one layer up (which column groups are dirty), not on the
  interval representation itself.
- `self_edge_clamp`'s own day-ordinal return type is a residue worth a follow-up: every OTHER
  margin-producing function in this module now returns exact seconds, so a future caller could
  make the same before_seconds/after_seconds mix-up as this phase's own initial bug, silently.
  Not urgent (one call site, exercised by conformance tests), but worth converting to
  `Seconds`/exact-seconds return the next time that module is touched.
- `PartitionGrain::Week`'s `start_dow` is `chrono::Weekday`, not the plan's suggested `u8` — this
  matches `TimeseriesConfig.week_start: Option<smelt_core::config::Weekday>` exactly via a new
  one-shot conversion (`propagation.rs::chrono_weekday`), avoiding a second enum. No follow-up
  needed, just noting the deviation from the plan's literal type sketch.
- Did NOT touch `smelt-maintenance-testkit/src/probes.rs`/`schedule_gen.rs`'s own private
  `clamp_days` (day-ceiling helpers for SQL scan-window fixtures, unrelated to the propagation
  graph's `Edge`/`PartitionInterval` types) — verified these are a separate concern (maintenance
  SQL scan sizing, not cross-model propagation) and out of this phase's scope.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  `cargo test --workspace`, `example_diagnostics`).
- `cargo test -p smelt-logical --lib maintenance::propagate --test maintenance_propagation_adjoint --test maintenance_tracer_propagation` — 29 + 29 + 6 passed.
- `cargo test -p smelt-runtime --test since_upstream_propagation --test tracer_propagation --test typed_edge_graph` — 34 + 6 + 5 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed.
- `cargo test -p smelt-cli --features duckdb --test e2e since_upstream` — 1 passed (plus
  `cargo test -p smelt-cli --test since_upstream` — 19 passed).
- `cargo test --workspace` (via verify-phase.sh) — green; this DID surface real bugs outside
  the phase's own named test targets (the self-edge scaling bug, several testkit/CLI-test
  fixture-scaling gaps), consistent with phase 25's summary note that cross-cutting type
  changes need the full sweep.
