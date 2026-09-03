# Phase 27a summary — `--show-sql` renders the suppressed matched arm

**Shipped:**
- `smelt_logical::maintenance::choice::resolve_cell_write_suppression` (`choice.rs`) — a new
  shared resolver folding `model_property_vector(...).comparability` → `resolve_write_suppression`
  → `resolve_write_variant` into one call. `maintenance_driver.rs`'s two inline copies (the
  `ColumnScopedMerge` and `DeleteInsert`/staged-candidate live resolvers) now call it instead of
  duplicating the sequence.
- `smelt-runtime::diagnostics::build_technique_statements` now resolves write suppression for
  `Technique::ColumnScopedMerge` (via the new shared resolver + `effective_override`, mirroring
  `maintenance_driver.rs`'s own live path) and `Technique::KeyedFold` (via a raw P2/P3-only
  resolution mirroring `cumulative.rs::resolve_cumulative_write_suppression` — see Decisions),
  dispatching to `emit_column_scoped_merge_suppressed`/`emit_keyed_fold_suppressed` on `Suppressed`.
  A `ChoiceRefusal` (e.g. a `technique: suppress` pin over a refused proof) propagates as a build
  error.
- `build_model_diagnostics`/`build_plan_cell_diagnostics` gained a `column_groups: &[ColumnGroup]`
  parameter, threaded from all callers (`smelt-cli/src/commands/explain.rs`, `smelt-ui/src/build.rs`,
  and every test call site — `smelt-ui` previously discarded `result.column_groups`).
- `resolve_technique_admissibility` now downgrades `Admitted` to `NotApplicable` for the cell's own
  technique **only** when the build failure is a write-suppression refusal (tagged via a
  `WRITE_SUPPRESSION_REFUSAL_MARKER` prefix, stripped before display) — every other pre-existing
  structural build failure keeps the old "Admitted despite empty statements" convention untouched.
- 5 new tests in `crates/smelt-runtime/tests/diagnostics.rs`: suppressed ColumnScopedMerge preview,
  first-build-posture Unconditional, incomparable-column Unconditional, suppress-pin-refusal (empty
  statements + NotApplicable), suppressed KeyedFold preview.
- Extended `keyed_fold_preview_matches_executed_statement_for_state_bearing_model`
  (`statement_parity.rs`) with a byte-identical guard assertion between the preview and the
  actually-executed `MERGE` for a real Salsa-driven fixture (`device_avg_amount`, `AVG`).
- Spec/doc edits: `docs/specs/cli.md` §`--show-sql`, `docs/specs/ui_model_diagnostics.md` §Semantics
  "Technique preview set", `docs/specs/incremental_models.md` Known Divergences (dropped the
  resolved clause), `docs-site/docs/reference/smelt-explain.md`.
- Regenerated `docs-site/docs/examples/web-analytics/deduplication.md` — the fix changes real
  `--show-sql` output for `silver.events_deduped`'s `MERGE`, which now renders the suppressed guard
  (`tutorial_freshness` test caught the drift; `python3 examples/web_analytics/generate_tutorial.py`
  fixed it).

**Decisions:**
- **KeyedFold's preview intentionally does NOT go through the override-ladder/variant-folding
  resolver.** Investigation found `cumulative.rs::resolve_cumulative_write_suppression` (the actual
  live KeyedFold write-suppression resolution) never calls `resolve_write_variant`/
  `effective_override` — unlike `maintenance_driver.rs`'s `ColumnScopedMerge` path. Using the full
  variant-folding resolver for the KeyedFold preview would have made it *diverge* from what a live
  KeyedFold run actually executes in edge cases (an override pin, or a first-build/ledger-catch-up
  trigger) — the reverse of this phase's goal. The KeyedFold preview arm instead mirrors
  `resolve_cumulative_write_suppression` exactly: raw P2/P3 only, `group_columns` from the
  classification's own aggregator output names (not the `column_groups` lookup), row identity
  re-derived from `classification.unique_key`.
- **Admissibility downgrade is scoped narrowly** (marker-string tag) rather than applying to every
  build failure for the cell's own technique, to avoid retroactively changing behavior for
  pre-existing structural failures (a real fixture, `daily_cube_metrics`, has a `ColumnScopedMerge`-
  admitted cell whose preview build already failed for an unrelated reason — "no unique_key
  declared" — under the test harness's synthetic `unique_key: &[]`; an unscoped downgrade broke
  `exactly_one_admitted_per_cell`).

**For the next planner:**
- **Real gap, not addressed here (out of this phase's scope):** `cumulative.rs`'s live KeyedFold
  write-suppression resolution does not fold the override ladder or the first-build/steady-state
  posture the way `ColumnScopedMerge`'s live driver does. A `technique: suppress`/`prefer` pin, or a
  first-build/`ledger_catch_up` KeyedFold trigger, currently has **zero effect** on what a live
  KeyedFold run executes — it always uses the raw P2/P3 proof. Worth a dedicated phase to either (a)
  wire `resolve_write_variant`+`effective_override` into `cumulative.rs`'s resolution, or (b)
  explicitly decide this asymmetry is intended and document it in `incremental_models.md`.
- 27b–27g are still `planned`/`pending` per the outcome table and were not touched.
- Did not write the plan's literal `crates/smelt-cli/tests/explain_model.rs::show_sql_renders_the_suppressed_form`
  end-to-end subprocess test: no real fixture in the repo today reaches a live `ColumnScopedMerge`
  cell (confirmed via `statement_parity.rs`'s own doc comment — membership-sensitivity now routes
  `daily_events_enriched` to `DeleteInsert`), and building a new example fixture just to reach it
  was judged out of proportion to this phase. Coverage is instead end-to-end via the extended
  `statement_parity.rs` KeyedFold test (real Salsa + real `execute_project` + real backend) plus the
  5 focused `diagnostics.rs` tests. If a `ColumnScopedMerge`-reaching fixture is added for 27b/27c/27d,
  add the CLI subprocess test then.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full `cargo test`,
  example_diagnostics).
- `cargo test -p smelt-runtime --test diagnostics --test statement_parity --test dry_run_statements` — PASS.
- `cargo test -p smelt-cli --features duckdb --test explain_model --test explain` — PASS.
- `cargo test -p smelt-ui --test api` — PASS.
- `cargo test --workspace` — PASS (341 test binaries, 0 failures) — includes `tutorial_freshness`
  after regenerating the drifted tutorial page.
