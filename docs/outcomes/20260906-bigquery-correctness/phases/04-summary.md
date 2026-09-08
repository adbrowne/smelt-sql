# Phase 4 summary — the enrichment-keyed model-edge route

**Shipped:**
- A third discovery route in `append_model_edge_cells`
  (`crates/smelt-logical/src/maintenance/derive/model_edge.rs`,
  `admit_enrichment_keyed_merge` + `enrichment_join_key_scope`): a clockless
  `KeyedUpsert` model edge read in value-enrichment position (an ON/USING
  equality matching the edge's own declared `unique_key`, no row-admission
  reach) by a partition-addressed downstream now derives an
  `UpstreamMutation`/`Technique::ColumnScopedMerge` cell, addressed by the
  join key the downstream itself projects. Attempted only after both
  key-addressed routes decline with `KeysNotDiscoverable`; guarded to real
  enrichment JOINs only (a plain `FROM <edge>` driving relation is excluded —
  caught a real regression, see Decisions).
- `KeyDiscovery::EnrichmentKeyed` (new variant, exhaustively matched
  everywhere `KeyDiscovery` was already matched: `smelt-runtime`'s
  key-addressed driver, `smelt-cli`'s `explain` text renderer).
- `Refusal::RepairKeysNotDiscoverable` now carries a real `DiagnosticCode`
  (`MaintenanceRepairKeysNotDiscoverable`) end to end: `smelt-logical`'s
  `refusal_code`, `smelt-db`'s `MaintenanceRefusal`/`diagnostic_for_refusal`,
  the LSP's kebab-case code map. The catalogue row in
  `docs/specs/diagnostics.md` already existed (issue #179 anticipated it);
  moved out of "no variant yet".
- `ModelEdge::allow_full_scan: bool` (new field, `false`-defaulted at every
  one of ~20 existing construction sites); populated for real in
  `smelt-db`'s `derive_model_maintenance_plan_with_edges` from the
  downstream's own `maintenance.scan_bounds.per_source.<edge>`.
- `join_shape::local_equality_columns_against` and
  `skeleton_closure::enrichment_join_clause` — small new leaf helpers the
  route's key-resolution needs (the local, downstream-side column of an
  enrichment join's equality, and the join's own `JoinClause`).
- Spec: `docs/specs/incremental_models.md` §"Upstream model edges" — the new
  route, its scan-bound obligation, and the three-way refusal ordering.
- Tests: `crates/smelt-logical/tests/model_edge_enrichment_mutation.rs` (7
  cases per the plan), `crates/smelt-db/tests/maintenance_diagnostics/
  model_edge_enrichment.rs` (2 cases), `refusal_codes` fixtures extended,
  `github_activity_replay.rs`'s `events_enriched_dimension_mutation_cell_
  technique` flipped from "no cell" to asserting `ColumnScopedMerge` — on
  the REAL `examples/github_activity/models/gold/events_enriched.sql`
  fixture, confirming the derivation fires end to end, not just in
  synthetic tests.
- `examples/github_activity/models/gold/{repo_dim,events_enriched}.sql`
  header comments updated to describe the fixed behaviour.

**Decisions:**
- The helper's `derive_column_groups` call passes only the ONE synthesized
  edge as `sources` (not the model's other declared sources), matching the
  plan's literal wording — a v0 scoping choice, not an oversight: it's
  correct for every real shape this outcome's spine surfaced (single
  enrichment dimension), and a model with other unrelated sources referenced
  elsewhere would see those columns' provenance collapse fail-closed to
  "unknown", which never affects THIS edge's own eligibility filter.
- Added a guard `enrichment_join_clause(sql, &edge.name).is_none() =>
  Ok(None)` that the plan didn't spell out explicitly: without it, a plain
  `FROM smelt.<edge>` driving relation (no JOIN at all) was wrongly treated
  as eligible for a merge, breaking an existing regression test
  (`keyed_model_edge.rs::consumer_not_carrying_upstream_keys_is_refused`).
  Caught by running the full gate before considering the phase done — not
  by the plan's own listed tests, none of which covered a driving-relation
  (non-JOIN) clockless KeyedUpsert edge.
- Did NOT wire model edges into `smelt-db`'s LSP-facing `maintenance_plan`
  Salsa query (`maintenance_plan_diagnostics`, which `file_diagnostics()`
  calls) — discovered it calls `derive_model_maintenance_plan` (source-only),
  never `..._with_edges`, so NO model-edge refusal (not just mine —
  `ReachNotDerivable` has the identical, pre-existing gap, already documented
  in its own doc comment) ever reaches `file_diagnostics()`/LSP diagnostics
  today; only `smelt explain`/`maintenance_plan_report` sees them. Test 8
  ("...raises a diagnostic from file_diagnostics()") was rewritten to use
  `plan_for` (the `smelt explain` query) instead, with the gap documented
  inline. Wiring that divergence closed is real, separate work — flagged
  below for the next planner, not attempted here (would have expanded this
  phase well beyond its stated scope).

**For the next planner:**
- **LSP-diagnostics/`smelt explain` divergence for model edges** (found, not
  fixed): `smelt-db`'s `maintenance_plan` Salsa query never threads model
  edges at all, so EVERY model-edge refusal (`ReachNotDerivable`, and now
  `RepairKeysNotDiscoverable`) is invisible to `file_diagnostics()` / the LSP
  — only `smelt explain` (`maintenance_plan_report`) sees them. A user editing
  `events_enriched.sql` in an editor gets no diagnostic for a
  `RepairKeysNotDiscoverable` refusal at all; only running `smelt explain`
  surfaces it. Worth its own outcome/phase: thread `model_edges_for` into
  `maintenance_refs/plan.rs`'s `maintenance_plan()` the same way
  `maintenance_plan_report()` already does, and switch `maintenance_plan_
  diagnostics` to `derive_model_maintenance_plan_with_edges`.
- Phase 5 (this outcome's next row) makes the derived cell live on the run
  path: `resolve_live_column_scoped_cell`/`maintenance_availability::
  derive_resolved` currently call the source-only `derive_model_maintenance_
  plan`, never `..._with_edges`, so the mutation gate and unique-key lookup
  can't see this new cell at all yet. `github_activity`'s `current_repo_name`
  will not actually heal on a rename until that phase lands.
- The `KeyDiscovery::EnrichmentKeyed` arms added to `smelt-runtime`'s
  key-addressed driver are unreachable dead-code guards today (that driver
  only processes `Technique::PerGroupRecompute` cells; this route only ever
  produces `ColumnScopedMerge`) — worth a note if phase 5's wiring ever
  routes a cell through that driver by mistake.
- `dags::diamond_propagation_suffices` (one of the two "known live
  conformance failures" success criterion 6 names) passed clean in this
  phase's full gate run — worth re-checking at phase 9 whether it's actually
  fixed already or just flaky.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test model_edge_enrichment_mutation --test
  maintenance_referential_integrity --test model_edge_delta_restriction
  --test keyed_model_edge` — all pass (the last one caught the
  driving-relation regression, fixed before this summary).
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration`
  — all pass (40 + 2 tests).
- `cargo test -p smelt-runtime --test statement_parity --test
  execute_parity` — all pass (41 + 4 tests).
- `cargo test -p smelt-cli --test maintenance_conformance --test
  github_activity_replay --features duckdb` — all pass (101 + 16 tests).
- `cargo test -p smelt-cli --test example_diagnostics` — all pass (125
  passed, 1 ignored, unrelated).
- `.claude/large-file-baseline.txt` bumped (+1-4 lines each) for 5 files
  touched by exhaustive-match arms and doc comments — sign-off note in the
  baseline file and this summary.
