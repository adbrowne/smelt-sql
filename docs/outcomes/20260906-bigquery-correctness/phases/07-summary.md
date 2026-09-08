# Phase 7 summary — the derived output window propagates within a run

**Shipped:**
- `IncrementalWindows::output_window()` (`crates/smelt-runtime/src/windowing.rs`) — the
  batch tiling envelope `(first.partition_start, last.partition_end)`, or `None` for an
  empty batch list.
- `widen_run_window_for_upstream_outputs` (same file) — the pure widening helper: unions a
  requested window with every in-run upstream's output window, skips a mismatched-axis
  upstream with a `warn!` rather than silently dropping it, and aligns the result outward to
  the downstream's own granularity.
- `build_model_plans` (`crates/smelt-runtime/src/execute/plan.rs`) now carries a
  `model_name → output window` map across the topologically-ordered `selected` loop: before
  the `frozen_horizon` clamp, a model's requested window is widened by every upstream in its
  own `refs` that's already in the map; after `compute_incremental_windows_ordered` returns,
  the model's own output window is recorded for downstream consumers later in the loop.
- Spec deltas: `docs/specs/model_transforms.md` §Semantics "The output window is derived,
  never assumed" gained "The derived output window propagates within a run."; `docs/specs/
  incremental_models.md` §"Forward propagation — what must run" cross-references it.
- `docs/handoffs/2026-09-08-github-activity-findings.md` and `examples/github_activity/
  README.md` §"Trusting the numbers" rewritten: all four root causes are now **fixed**, not
  registered; `DIVERGENCE_REGISTRY` is empty.
- New test `crates/smelt-runtime/tests/downstream_output_window_propagation.rs` (8 tests,
  unit + end-to-end through `execute_project`) and
  `marts_daily_active_contributors_matches_the_full_refresh_oracle` in
  `crates/smelt-cli/tests/github_activity_oracle.rs`.

**Decisions:**
- Task 1's inspection confirmed the handoff's open question exactly as the plan predicted:
  the maintenance cell for this edge already exists (clocked route), so the fix is
  dispatch-window propagation, not a new cell — see outcome.md's phase-7-implementation
  decision-log entry for the full trace.
- `findings_handoff_names_no_unknown_relation` (in `github_activity_oracle.rs`) required a
  small logic change: it asserted the handoff's divergence table names at least one
  relation, which is false now that `DIVERGENCE_REGISTRY` is empty. Loosened to accept an
  empty table when the registry itself is empty, still failing on any stale claim otherwise.
- `Bound`/`Side` variants are now unconstructed (empty registry) — kept, not deleted, with
  `#[allow(dead_code)]` for the next divergence this registry finds, matching the existing
  precedent on `Side::Oracle`.

**For the next planner:**
- The fix worked on the first attempt against the full 30-day oracle — no second bug layer
  like phases 3 and 6 hit.
- `tutorial_freshness` did not need regeneration, contrary to the plan's expectation — the
  web-analytics tutorial's directive commands never select a Form-B upstream and its Form-A
  downstream together in one invocation, so this fix path is untested by that gate. Not a
  gap in this fix's correctness (the new `downstream_output_window_propagation.rs` test 6
  covers it directly), but worth knowing if a future tutorial page adds such a pairing.
- Row 8 (`docs/outcomes/20260906-bigquery-correctness/outcome.md`) is now largely satisfied
  by this phase for the "empty registry" half — the remaining work per its reworded text is
  proving the unregistered-divergence sweep still fails closed on an *empty* registry (not
  just a registry with entries), which `an_unregistered_divergence_fails`'s existing
  perturbation-based negative control already exercises path-independently of registry
  content, but wasn't re-verified as such here.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.
- `cargo test -p smelt-runtime --test downstream_output_window_propagation --test
  windowing_form_b_chunking --test windowing_parity --test windowing_ordered --test
  partition_axis_windowing --test statement_parity --test dry_run_statements --test
  since_upstream_propagation` — 135 passed, 0 failed.
- `cargo test -p smelt-cli --test github_activity_oracle --features duckdb` — 11 passed, 1
  ignored (measurement-only sweep).
- `cargo test -p smelt-cli --test github_activity_replay --features duckdb` — 17 passed.
- `cargo test -p smelt-cli --test tutorial_freshness --features duckdb` — 1 passed, no
  regeneration needed.
- `cargo test -p smelt-lsp --test example_workspaces` — 37 passed.
- `bash .claude/scripts/large-file-check.sh` — OK (`windowing.rs` baseline bumped 1218→1277
  with a sign-off note in `.claude/large-file-baseline.txt`).
