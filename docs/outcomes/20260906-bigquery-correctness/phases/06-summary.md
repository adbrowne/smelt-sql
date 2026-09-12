# Phase 6 summary — interior-chunk forward reach for Form-B models

**Shipped:**
- `IncrementalBatch::scan_start`/`scan_end` (`crates/smelt-runtime/src/windowing.rs`) — a new
  field pair carrying each batch's skew-only reach (`[bs − before, be + after)`, clamped to the
  invocation's outer scan envelope), distinct from the pre-existing lookback-only `filter_start`/
  `filter_end`.
- `derive_batch_filtered_sql` (`crates/smelt-runtime/src/execute/sources.rs`) gained a `scan_range`
  parameter, used for `inject_source_filters` in place of `run_range`; its three call sites
  (`execute/project/mod.rs`, `execute/project/dry_run.rs`, `smelt-cli/src/explain.rs`) now build
  `scan_range` from `batch.scan_start`/`scan_end`.
- Spec sentences in `docs/specs/incremental_shapes.md` §"Execution model (DuckDB)" and
  `docs/specs/model_transforms.md` naming the scan-side skew inversion per chunk.
- `docs/handoffs/2026-09-08-github-activity-findings.md` updated: root cause 3's **Fixed by**
  paragraph, punch-list items 2-3 marked Done, `silver_actor_sessions` dropped from the
  registered-divergence table.
- `crates/smelt-cli/tests/github_activity_oracle.rs`: `silver_actor_sessions` removed from
  `DIVERGENCE_REGISTRY`; new test `silver_actor_sessions_matches_the_full_refresh_oracle`.
- `crates/smelt-runtime/tests/windowing_form_b_chunking.rs`: 6 tests, including
  `lookback_and_skew_widen_independently_never_summed` (the double-counting regression guard).

**Decisions:**
- `scan_start`/`scan_end` is a **new** field, not a repurposing of `filter_start`/`filter_end` —
  see outcome.md decision log for why (the first attempt double-widened the lookback component).
- `marts_daily_active_contributors`' divergence bound was re-measured (task 6) and found
  unchanged — `succession_divergence_is_exactly_tied_row_multiplicity` still passes with the same
  predicate. Phase 7 starts from the existing registered bound, not a shifted one.
- Large-file baseline bumped for `windowing.rs` (+45), `execute/project/mod.rs` (+18), `explain.rs`
  (+5) — sign-off note in `.claude/large-file-baseline.txt`.

**For the next planner:**
- **Coverage gap the double-counting bug exposed**: no test in `smelt-runtime`'s own suite
  combines a nonzero SQL-inferred lookback with multi-chunk batching and a literal-text
  assertion — only `web_analytics_tutorial_pages_are_fresh` (a doc-freshness gate in
  `smelt-cli`) caught the regression. `lookback_and_skew_widen_independently_never_summed` now
  covers this in `windowing_form_b_chunking.rs`, but consider whether `statement_parity` should
  also gain a lookback+skew+chunking fixture so this class of bug fails closer to its source next
  time.
- **The plan's diagnosis undersold the fix's scope.** The plan (`06-plan.md`) described the
  defect purely in terms of `compute_calendar_windows`' `filter_start`/`filter_end` formula; the
  actual load-bearing fix required discovering that those fields had no consumer and wiring a
  new mechanism into three call sites in `smelt-runtime`/`smelt-cli`. Future phases touching
  `windowing.rs` should verify a field is actually consumed before trusting a plan's formula
  alone — `rg` for the field name across the workspace, not just within the crate being edited.
- Punch-list item 4 (phase 7) is next: the missing repair edge from `silver.actor_sessions`'s own
  self-rebase to `marts.daily_active_contributors`.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-runtime --test windowing_form_b_chunking --test windowing_parity --test windowing_ordered --test partition_axis_windowing --test statement_parity --test dry_run_statements` — 90 passed
- `cargo test -p smelt-cli --test github_activity_oracle --features duckdb` — 10 passed, 1 ignored (measurement-only)
- `cargo test -p smelt-cli --test tutorial_freshness --features duckdb` — 1 passed
- `cargo test -p smelt-lsp --test example_workspaces` — 37 passed
- `bash .claude/scripts/large-file-check.sh` — OK (baseline bumped with sign-off note)
