# Phase 8 summary — surface: manifest `probes`, `smelt explain` rendering, docs-site

## Shipped

- `ModelRunRecord.probes` is now populated at all three live dispatch sites: the incremental-batch
  loop and the full-refresh site each accumulate a per-model `Vec<ProbeRecord>` across their
  `dispatch_declared_model_probes`/`dispatch_and_record_append_only_postures` calls
  (`crates/smelt-runtime/src/execute.rs`); the cumulative arm keeps `Vec::new()` with a comment
  explaining it dispatches no probes today.
- New `smelt_runtime::probe_plan` module (`crates/smelt-runtime/src/probe_plan.rs`):
  `probe_plan_for_model(...)` builds the offline, pure probe-set descriptor `smelt explain` renders
  — calls `model_probes::declared_model_probes` (symbolic scope) and
  `source_probes::append_only_posture_probes` (empty baseline) for the four registry-`built` probes,
  and reads `PlanCell::skeleton_source_closure`/`KeyLocality::slice` directly for the two
  plan-driven rows (`referential_integrity`, `key_recurrence`) rather than re-deriving their
  dispatch conditions.
- `smelt explain <model>` (text and `--json`, with/without `--show-sql`) gained a `Probes (N):`
  section / `probes` JSON array — fact, diagnostic code, licensed cell, project cadence, and a
  static `+1 query per consuming run` cost line (`crates/smelt-cli/src/explain.rs`,
  `crates/smelt-cli/src/commands/explain.rs`).
- Fixed `crates/smelt-runtime/tests/model_probes.rs::monotonicity_probe_fires_named_diagnostic`
  (phase 7's recorded finding): it declared a `partition_column` the staged table didn't have, so
  it passed on a DuckDB binder error whose wrapped text happened to contain the diagnostic name.
  Now declares `unique_key: [user_id]` (a column the table has) and asserts the message names a
  real violation count, not a binder error.
- Spec delta: `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" documents the
  probe-set rendering and the JSON schema addition.
- docs-site: `reference/smelt-yml.md` gained a `probes:` field row + "Probes Configuration"
  section (previously undocumented); `reference/smelt-explain.md` gained a "Probes" section;
  `reference/state.md` gained a "Run manifest" section documenting the `probes` array and its two
  `dispatched`/`skipped` outcomes.

## Decisions

- `smelt explain` stays fully offline: `probe_plan_for_model` builds probe SQL (via the same
  dispatch-owner functions a live run calls) only to confirm a declaration is probe-backed, never
  executes it — the cost line is a static sentence, not a measurement.
- The two plan-driven registry rows (`referential_integrity`, `key_recurrence`) are read directly
  off already-derived plan data (`PlanCell::skeleton_source_closure`, `KeyLocality::slice`) rather
  than routed through a shared builder function, because their live dispatch sites
  (`maintenance_driver.rs`) are deep inside per-cell, per-route execution logic with no offline
  equivalent to call — this keeps "reuse the dispatch owners" honest without inventing a new
  offline-only code path those owners don't share.
- `--project-dir`-only integration tests (`crates/smelt-cli/tests/explain_probes.rs`) drive the
  real `smelt` binary rather than reimplementing `commands::explain::explain_maintenance_plan`'s
  pipeline in the test, matching this crate's existing test style for full-report assertions.

## For the next planner

- The outcome's phase table is now fully `done` — this was the last row. Two loose ends from this
  phase's own work, both outside phase 8's stated scope:
  - `probe_plan_for_model`'s `referential_integrity` row only reports a probe when a cell's
    `skeleton_source_closure` is already `Closed { DeclaredReferentialIntegrity }` — i.e. explain
    only shows this row for the same narrow model-edge call site phase 3 scoped live dispatch to
    (see outcome §Out of scope). A model whose declared-route dispatch is unreachable today
    (per phase 3's note) also shows no probe row in explain, which is consistent but worth knowing
    if that widening ever lands.
  - Regenerating `docs-site/docs/examples/web-analytics/*.md` via
    `python3 examples/web_analytics/generate_tutorial.py` was required to keep
    `tutorial_freshness.rs` green (one page's embedded `smelt explain` output gained the new
    `Probes (0):` section). Future phases touching `smelt explain` output anywhere should expect
    the same regeneration step.
- No new declaration kinds or wiring gaps were discovered while implementing this phase.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-runtime --test probe_manifest --test model_probes --test source_probes --test probe_dispatch` — 21 passed.
- `cargo test -p smelt-cli --test explain_probes --test explain_model --test explain_maintenance --test explain_show_sql` — 46 passed.
- `cargo test -p smelt-logical --test probe_obligation` — 6 passed (registry unchanged, as expected).
- `cargo test -p smelt-runtime --test statement_parity` — 22 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 63 passed.
