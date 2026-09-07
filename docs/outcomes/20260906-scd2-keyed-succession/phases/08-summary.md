# Phase 8 summary — Explain surface for the succession grain

**Shipped:**
- `smelt explain <model>` (text and `--json`) now renders a succession-grain
  cell's `grain:`, `identity: (k…, t)`, `technique: succession-patch`,
  `run axis: <col> (arrival-partitioned|event-time-partitioned)`, `clock:`,
  the fixed `posture:` line, an optional `pre-window filter:`, and the
  tombstone ledger as `internal state:` — `crates/smelt-cli/src/explain.rs`
  (per-cell text loop) and the new `crates/smelt-cli/src/explain/
  succession.rs` module (`SuccessionExplainView`, `SuccessionJson`,
  `SuccessionTombstoneLedgerJson`, `build_succession_explain_view`).
- The headline (`delta_signature_headline`) now renders `keyed_succession` /
  `event`-addressed for a succession model, taking precedence over the
  model's own `OutputDelta` derivation — `succession::succession_delta_signature`
  and `render_keyed_succession_emits` in the new module.
- `ExplainMaintenanceJson` gained a `succession` object (absent, never
  `null`, for a non-succession model).
- `smelt_runtime::maintenance_driver::succession` gained a shared
  `resolve_succession_run_axis`/`SuccessionAxis`/`SuccessionPartitioning`
  classifier; `resolve_live_succession_cell` now consumes it (single owner
  of the bare-name source match + arrival/event-time classification).
- `smelt_logical::maintenance::succession::SUCCESSION_POSTURES` — the
  grain's fixed posture triple, read by explain rather than restated.
- 13 new tests: 8 text-report tests + 4 JSON tests in
  `crates/smelt-cli/tests/explain_maintenance/succession.rs`, 1 driver unit
  test (`run_axis_classifies_arrival_vs_event_time_partitioning`) in
  `crates/smelt-runtime/src/maintenance_driver/succession/tests.rs`.

**Decisions:**
- The succession model's own headline `; grain: …` clause is legitimately
  absent when the model declares no `timeseries:`/`unique_key:` of its own
  (the succession grain is *recognised*, not declared) — `derived_grain` is
  `None` in that (common) case; the fixture and its tests reflect this
  rather than forcing an artificial declared grain onto the fixture.
- `lead_columns`/`lag_columns` in the JSON `succession` object are **not**
  `skip_serializing_if` empty (unlike `pre_window_filter`/`delete_flag`,
  which the spec explicitly calls out as omittable) — the spec's omission
  list names only the latter two.
- Split ~200 lines of succession-only explain code into a new
  `crates/smelt-cli/src/explain/succession.rs` module to keep
  `crates/smelt-cli/src/explain.rs`'s growth bounded; the residual growth in
  `explain.rs`/`commands/explain.rs` (thin single-owner wiring through
  `delta_signature_headline`/`build_maintenance_plan_report`/
  `build_maintenance_plan_json`) required a large-file baseline bump with a
  sign-off note in `.claude/large-file-baseline.txt` and this commit message
  — further fragmenting those functions across files would compromise their
  single ownership.

**For the next planner:**
- Phase 9 (fixture + docs) can reuse `stage_succession_project`'s shape
  (`crates/smelt-cli/tests/explain_maintenance/support.rs`) as a starting
  point for the `examples/` `customer_changes`/`customer_history` fixture,
  though the example workspace should probably use richer column names/SQL
  than the minimal test fixture.
- Not investigated: whether `smelt table`/`smelt type` (offline schema
  commands) need any succession-aware rendering — out of this phase's scope
  (explain only), worth a quick check before phase 10's spec-divergence
  sweep closes out.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model
  --test explain --test cli_unit --quiet` — 172 passed.
- `cargo test -p smelt-cli --test cli_docs_coverage --test
  explain_docs_freshness --quiet` — 6 passed.
- `cargo test -p smelt-runtime --test execute_parity --test
  statement_parity --quiet` — 45 passed.
- `cargo test -p smelt-runtime --lib maintenance_driver::succession --quiet`
  — 14 passed.
- `bash .claude/scripts/large-file-check.sh` — green after baseline update
  (see Decisions).
