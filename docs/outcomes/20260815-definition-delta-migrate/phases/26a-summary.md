# Phase 26a summary — derived (not assumed) write-footprint mirror

**Shipped:**
- `ScanClamp::write_footprint: Option<(Seconds, Seconds)>` (`crates/smelt-logical/src/maintenance/mod.rs`) replaces the assumed `(after, before)` mirror; `footprint()` returns `Option<...>` and never re-derives from the clamp's own read margins.
- `ModelInputs::keyed_time_axis: Option<&'a str>` (`crates/smelt-logical/src/maintenance/derive.rs`) — a `Grain::Key` model's declared `timeseries.partition_column`, threaded from `crates/smelt-db/src/queries/maintenance.rs` (real `metadata.timeseries`), `smelt-runtime`'s test callers, and `smelt-maintenance-testkit` recipes.
- `project_source_link` (`derive.rs`) grows a `keyed_time_axis` parameter: with `Some(axis)` it poses the footprint question via `reflect_footprint` exactly like a partition-addressed output (`Bounded` ⇒ clamp with the derived footprint; `Unbounded`/`NotDerivable` ⇒ `Unlinked`); with `None` it keeps the pre-existing bare-keyed linking rule but stamps `write_footprint: None`.
- `derive_maintenance_plan_impl` computes `footprints` against `output_partition_col().or(keyed_time_axis)` so a keyed model's footprints map is populated when it has a declared axis.
- `Edge::footprint_days: Option<(i64, i64)>` (`crates/smelt-logical/src/maintenance/propagate.rs`), populated by `Edge::from_clamp` from `clamp.footprint()`; `Edge::reflect` consults it and returns `DayInterval::WHOLE` when absent. `Edge::require` (read direction) untouched.
- `smelt-runtime/src/propagation.rs`'s real per-workspace graph builder threads a parallel `footprint_days` map alongside `clamp_days`, widened the same way, downgrading to `None` the moment any contributing cell has no derived footprint.
- New tests: `crates/smelt-logical/tests/keyed_footprint.rs` (4 tests), 2 extensions to `maintenance_tracer_propagation.rs`, 1 extension to `crates/smelt-db/tests/maintenance_diagnostics.rs`.
- Spec: `model_properties.md` §"Footprint reflection" now states the keyed-axis case and drops the "keyed output poses no locality question" divergence; `incremental_models.md` drops the corresponding clause.
- Regenerated `docs-site/docs/examples/web-analytics/deduplication.md` — `silver.events_deduped`'s creation cell now shows `partition_local` with a real zero-margin clamp instead of the old "NOT partition_local" verdict (the declared axis now lets a zero-margin read link).

**Decisions:**
- Kept every non-target production fold site (self-edge margin, `PartitionLocal::No` fallback, the composed-node key→partition margin) mirroring its OLD read-margin behavior exactly via `footprint_days: Some((after_days, before_days))` (or symmetric `(0,0)`) — 26a's scope is the keyed/partition-addressed `ScanClamp` footprint, not these adjacent mechanisms; widening them to `WHOLE` would have been an uncontrolled scope expansion.
- Discovered (and fixed) that a locality-admitted composed node is a **clocked** node, not `Keyed`, once `model_grain` sees admission — so its inbound margin edge genuinely runs through `Edge::reflect`, unlike a bare keyed node. Initial code wrongly assumed this edge was always `Keyed` and skipped the footprint swap; the `since_upstream_composed_web_analytics` e2e test caught the resulting 3-day date shift.

**For the next planner:**
- The correct tuple order for a hand-rolled `Edge`/`ScanClamp` literal mimicking the old mirror is `footprint_days: Some((after_days, before_days))` — a real `ScanClamp::write_footprint` (via `reflect_footprint`) is already ordered correctly and needs no swap. This is easy to get backwards; a doc-comment reminder on `Edge::footprint_days` would help future edits.
- 26b (`INTERSECT`/`EXCEPT` per-arm classification) and 26c (hour-granularity) are next per the table; neither depends on this phase's internals.
- Did not attempt to widen `PartitionLocal::No`'s fallback edge from an exact zero-margin edge to a `WHOLE`-widening one — that fallback still predates any footprint proof and is out of scope here, but it is the same "no ScanClamp, no derived footprint" shape this phase's semantics describe; a future phase could legitimately fold it in.

**Gates:**
- `cargo test -p smelt-logical --test keyed_footprint --test locality_projection --test maintenance_tracer_propagation --test maintenance_tracer_evolution` — pass
- `cargo test -p smelt-runtime --test tracer_propagation --test tracer_evolution` — pass
- `cargo test -p smelt-cli --features duckdb --test e2e since_upstream_composed_web_analytics` (and the full `e2e` suite, 175 tests) — pass
- `cargo test -p smelt-cli --test maintenance_conformance` — pass (74 tests)
- `cargo test -p smelt-lsp --test example_workspaces` — pass (35 tests)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace test, example_diagnostics)
