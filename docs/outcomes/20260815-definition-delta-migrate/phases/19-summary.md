# Phase 19 summary — Mutation-cell reachability

**Shipped:**
- `docs/specs/incremental_models.md` §"Per-cell admission": new "Which changed inputs get a
  mutation cell" paragraph stating the derivation rule as timeless behaviour; narrowed the
  "Plan-consumer gaps" Known Divergences bullet to only the remaining "mutation genuinely
  happened" clause (retargeted to phase 19b).
- `smelt_logical::maintenance::derive::derive_triggers` (`crates/smelt-logical/src/maintenance/derive.rs`):
  new pure function deriving a model's full `Trigger` set. `UpstreamMutation` now admits (a) any
  explicitly-declared `mutable_snapshot` source regardless of clock, and (b) an `AppendOnly`
  source named in some column group's `mutation_sensitivity`. 5 new tests in
  `crates/smelt-logical/tests/maintenance_plan_admission.rs`.
- `crates/smelt-db/src/queries/maintenance.rs::derive_model_maintenance_plan` now calls
  `derive_triggers` instead of an inline loop (maintenance-plan purity). 2 new tests in
  `crates/smelt-db/tests/maintenance_diagnostics.rs` proving the `{status}`
  `UpstreamMutation{raw.user_status}` cell derives through the production wrapper
  (`DeleteInsert`/`RecomputeRegion`/`PartitionLocal::Yes`, membership-sensitive via the join's
  `ON` predicate) and that removing the window predicate correctly refuses `ScanUnbounded`.
- `crates/smelt-runtime/tests/technique_lowering.rs`: the old hand-built-trigger-list test
  (`real_fixture_daily_events_status_would_admit_partition_local_yes_cell`) rewritten onto the
  production wrapper (`real_fixture_daily_events_status_admits_partition_local_yes_cell`),
  deleting the "Known production gap" comment block — the `PartitionLocal::Yes` corner is now
  reachable from `examples/timeseries` through `smelt_db::queries::maintenance::derive_model_maintenance_plan`.
- `derive_mutation` (derive.rs) gained a narrowing: a group already covered by a
  `Trigger::NewData`/`Technique::PerGroupRecompute` repair cell for the same source is not also
  given a redundant `UpstreamMutation` cell (the repair cell already recomputes it).
- `crates/smelt-runtime/src/propagation.rs`: a `grain: key` model's `UpstreamMutation` cell over
  an `AppendOnly` source no longer registers a forward-propagation graph edge (it's key-addressed
  maintenance, dispatched by the live-cell resolvers, not a propagation concern) — scoped to
  `AppendOnly` only, so a `MutableSnapshot` enrichment source's pre-existing edge is unaffected.
- `crates/smelt-db/src/lib.rs::ref_model_source_facts` now threads the downstream model's
  `scan_bounds` config so a composed-upstream-model source (previously hard-coded
  `allow_full_scan: false` with no way to declare otherwise) can accept the escape hatch too.
- `docs-site/docs/reference/sources-yml.md` §"Mutation profile": two sentences on the widened
  admission rule and the `allow_full_scan` escape hatch.

**Decisions:**
- The widened rule surfaced ~15 real, pre-existing gaps across examples and test fixtures where
  an `AppendOnly` fold-driving source now derives a genuine (previously silently unmodeled)
  `UpstreamMutation` cell with no statically derivable scan bound. Every one was declared
  `allow_full_scan: true` rather than worked around — the honest escape hatch the guardrail
  already provides everywhere else (`examples/timeseries/models/{user_daily_spend,
  daily_cube_metrics,user_spend_running_total}.sql`, `examples/web_analytics/models/silver/
  {events_deduped,device_user_edges}.sql`, `examples/broken/models/maintenance_granularity_mismatch.sql`,
  plus ~10 test fixtures in smelt-runtime/smelt-cli/smelt-state).
- `docs-site` tutorial pages regenerated (`python3 examples/web_analytics/generate_tutorial.py`)
  after `events_deduped.sql`'s frontmatter changed; only `deduplication.md` drifted.
- The repair-family collision (a source admitted for BOTH the repair family's
  `PerGroupRecompute` and the new `UpstreamMutation` cell, double-writing the same group) is
  closed by the `derive_mutation` narrowing above, not by excluding the source from the wider
  rule — the repair cell already fully covers what the mutation cell would.

**For the next planner:**
- Phase 19b (mutation-happened discrimination) is next in the table and was NOT touched here —
  every `UpstreamMutation` cell dispatches/re-checks on every run regardless of whether the
  source actually changed.
- `crates/smelt-runtime/src/execute.rs`'s comment (~L2058) claiming "`derive_model_maintenance_plan`
  derives an `UpstreamMutation` trigger only for an UNCLOCKED [source]" is now stale — worth a
  follow-up doc sweep if a future phase touches that file, but out of this phase's scope (I did
  not find an actual behavioral collision there beyond the repair-family one already fixed).
- The widened rule is broad by design (any `AppendOnly` source in a value-sensitive group) — a
  future phase implementing 19b's "mutation genuinely happened" discrimination should double-check
  it doesn't need a narrower admission rule first (e.g. excluding a source that's *also* the
  model's own sole `NewData`-fold-driving source, which this phase found is NOT redundant for a
  `grain: partition` model — see `daily_cube_metrics.sql` — but IS fully redundant for a
  `grain: key` fold, handled here only via the repair-cell narrowing, not a general exclusion).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test --workspace --quiet` — 0 failures (ran to completion)
- `cargo test -p smelt-logical --test maintenance_plan_admission --test maintenance_coverage_matrix --test locality_projection` — 29 passed
- `cargo test -p smelt-db --test maintenance_diagnostics --test maintenance_model_upstream` — 26 passed
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity --test execute_parity` — 59 passed
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance --test example_diagnostics` — 193 passed, 1 ignored
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed
