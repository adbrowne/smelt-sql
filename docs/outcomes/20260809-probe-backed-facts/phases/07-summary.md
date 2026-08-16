# Phase 7 summary — conformance recipes: violated-fact scenarios caught by probes

**Shipped:**
- `crates/smelt-cli/tests/maintenance_conformance/fact_violations.rs` (new): a six-recipe pool,
  one per `built` §"Probe obligation" row, driven in-process (`LinkCProject`/`execute_project`
  for the three lifted full-refresh recipes; `smelt_runtime::model_probes` directly for
  `assert_monotonic`; `execute_delete_insert_with_delta_restriction` /
  `run_windowed_keyed_maintenance` directly for `referential_integrity` / `key_recurrence`, the
  route's only live dispatch sites). Four standing tests: a spec-registry coverage parse
  (`every_built_registry_row_has_a_violation_recipe`), a conforming-feed-matches-oracle leg, a
  violated-feed-fails-before-any-write leg, and a `probes: {cadence: off}` observability leg.
  Registered in `main.rs` (`mod fact_violations;`).
- `docs/specs/model_properties.md` §References → Tests: one added sentence naming the registry
  gate and the new pool (docs-only, no behaviour change).

**Decisions:**
- Four of six recipes (`functional_dependencies`, `bounded_domain`,
  `mutation_profile.kind: append_only`, `assert_monotonic`) are **not** end-state observable in
  their staged shape: each is a full-refresh (or, for monotonicity, unconditional
  `CREATE TABLE AS`) passthrough that consumes no technique the declaration licenses
  differently, so the write is bit-identical whether the declaration holds or not — only the
  probe distinguishes conforming from violating data. This is a real, checked property of each
  staged recipe, not a placeholder; each carries its own reason in the module doc comment and is
  printed as an explicit skip (never silently omitted) by the observability test. Two recipes
  (`referential_integrity`, `key_recurrence`) are genuinely observable: with the probe off, a
  narrowed technique (a delta-restricted recompute; a recurrence-slice-restricted merge)
  silently leaves stale/incomplete state a full recompute would not.
- `referential_integrity`'s recipe drives `execute_delete_insert_with_delta_restriction`
  directly against a real DuckDB backend (mirroring `delta_restricted_recompute.rs`), not a
  staged `smelt.yml` project through `execute_project` — this route's only live production
  dispatch site is the model-edge delta restriction, and per the outcome's phase 3 "Out of
  scope" note a model-edge cell's closure never actually carries `DeclaredReferentialIntegrity`
  facts in production; constructing the recipe at the level that IS reachable (the driver
  function itself) is the same accommodation `key_recurrence`'s recipe already needed, one row
  up.
- The `key_recurrence` conforming/violated legs mirror `locality_route3_recurrence_check.rs`
  directly (its own doc comment: real fixtures hit the unrelated extremal-`MAX` NOT-NULL
  diagnostic through `execute_project`'s pre-execution gate, independent of locality admission).

**Production finding (not fixed, recorded):**
- `crates/smelt-runtime/tests/model_probes.rs::monotonicity_probe_fires_named_diagnostic`
  declares `partition_column: "event_date"` against a table that has no `event_date` column.
  The probe SQL therefore fails with a DuckDB *binder* error, not a genuine violation match —
  but the wrapped error text happens to read `"Failed to execute DeclaredMonotonicityViolated
  probe for model..."`, so `err.to_string().contains("DeclaredMonotonicityViolated")` passes for
  the wrong reason. The test currently proves the diagnostic string appears somewhere in a
  failure, not that the monotonicity probe actually detected an out-of-order row. Confirmed by
  hand: this phase's own first monotonicity-recipe draft used
  `partition_column: event_time_column` (a different bug — every row its own singleton
  partition) and *failed loud* with `outcome: Dispatched` (no violation), proving the real
  detection path does discriminate correctly once the partition column is a real, constant
  output column (`user_id`, in the shipped recipe). `model_probes.rs` itself is outside this
  phase's file list; not touched.

**For the next planner:**
- Phase 8 (`ModelRunRecord.probes` population, `smelt explain` rendering): unaffected by this
  phase — no new dispatch sites were added, only new test coverage of the existing four.
- The `model_probes.rs` finding above is a real gap worth a follow-up fix (swap
  `partition_column` to an existing column, or add an explicit assertion that the probe SQL
  executed cleanly before checking for a violation) — small, mechanical, and independent of this
  outcome's remaining phase.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full `cargo test`,
  example_diagnostics).
- `cargo test -p smelt-cli --test maintenance_conformance` — 63/63 PASS, ~6s wall-clock (well
  under the ~60s budget).
- `cargo test -p smelt-cli --test e2e` (the three lifted recipes' original binary-level tests,
  untouched) — PASS.
- `cargo test -p smelt-logical --test probe_obligation` — 6/6 PASS.
- `cargo test -p smelt-runtime --test statement_parity --test model_probes --test source_probes`
  — 34/34 PASS.
